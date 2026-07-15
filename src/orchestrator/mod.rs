//! The orchestration lifecycle — ties all layers together.
//!
//! ```text
//!  ┌───────────┐   ┌────────────┐   ┌───────────────┐   ┌──────────────┐
//!  │ 1. State  │──▶│ 2. Checkout│──▶│ 3. Bridge     │──▶│ 4. Teardown  │
//!  │ Lock + DB │   │ target +   │   │ Runtime + MCP │   │ compact +    │
//!  │           │◀──│  skills    │◀──│ + inject      │◀──│ persist      │
//!  └───────────┘   └────────────┘   └───────────────┘   └──────────────┘
//!                                       │
//!                                  ┌────┴────┐
//!                                  │ MCP     │
//!                                  │ config  │
//!                                  └─────────┘
//! ```

use crate::bridge::{manifest::AgentStackManifest, BridgeConfig, OpenCodeBridge};
use crate::checkout::{skills, target, Workspace};
use crate::error::{ConductorError, Result};
use crate::mcp::{CacheMode, CodebaseMemoryMcp};
use crate::models::{AgentTask, SessionStatus};
use crate::runtime::{self, RuntimeKind};
use crate::server::AppState;
use crate::teardown;
use uuid::Uuid;

/// Execute the full agent session lifecycle.
pub async fn run_session(state: AppState, session_id: Uuid, task: AgentTask) -> Result<()> {
    // ── Layer 1: Acquire distributed lock ────────────────────
    state
        .db
        .update_session_status(session_id, SessionStatus::Preparing)
        .await?;

    let lock = state.locks.acquire(task.tenant_id, &task.branch).await?;

    // ── Layer 2: Dual-Checkout ───────────────────────────────
    let project = state.db.get_project(task.project_id).await?;
    let tenant = state.db.get_tenant(task.tenant_id).await?;

    let workspace = Workspace::new(
        &state.config.workspace_root,
        &tenant.slug,
        &project.name,
        &task.branch,
    );
    workspace.ensure_root()?;

    let target_repo = target::prepare_target(&workspace.target, &project.repo_url, &task.branch)?;
    let _skills_repo = skills::prepare_skills(&workspace.skills, &task.skills_repo)?;

    // Load the manifest from the skills directory.
    let manifest_path = workspace.skills.join("manifest.json");
    let manifest = load_manifest(&manifest_path)?;

    // ── MCP Setup ────────────────────────────────────────────
    // Configure codebase-memory-mcp scoped to /target.
    let cache_mode = if workspace.mcp_cache.exists() {
        CacheMode::Persistent
    } else {
        CacheMode::Ephemeral
    };

    let mcp = CodebaseMemoryMcp::new("/target", cache_mode)
        .map_err(|e| ConductorError::Other(format!("MCP setup failed: {e}")))?;

    // Write MCP settings to a temp file that will be mounted into the guest.
    let mcp_settings_host = workspace.root.join("mcp_settings.json");
    mcp.write_settings(&mcp_settings_host)?;

    if cache_mode == CacheMode::Persistent {
        workspace.ensure_mcp_cache()?;
    }

    // Recover history from a previous session if resuming.
    let history = if let Some(prev) = task.resume_from {
        state.db.get_session(prev).await?.history
    } else {
        state
            .db
            .get_latest_history(task.tenant_id, task.project_id, &task.branch)
            .await?
            .unwrap_or_else(|| serde_json::json!({"messages": []}))
    };

    // ── Layer 3: Bridge — provision runtime & inject payload ─
    state
        .db
        .update_session_status(session_id, SessionStatus::Running)
        .await?;

    let runtime_kind = RuntimeKind::from_env();
    let runtime_backend = runtime::create(&runtime_kind);
    let mut bridge = OpenCodeBridge::new(runtime_backend);

    let bridge_config = BridgeConfig {
        target_host: workspace.target.clone(),
        skills_host: workspace.skills.clone(),
        mcp_settings_host: Some(mcp_settings_host.clone()),
        mcp_cache_host: if cache_mode == CacheMode::Persistent {
            Some(workspace.mcp_cache.clone())
        } else {
            None
        },
        api_key: std::env::var("OPENCODE_API_KEY").unwrap_or_default(),
        timeout_secs: state.config.session_timeout_secs,
    };

    bridge.start(&bridge_config).await?;

    let mut payload =
        OpenCodeBridge::build_payload(session_id, &task.instruction, history, &manifest);
    payload.mcp_settings_path = bridge
        .provisioned()
        .and_then(|p| p.mcp_settings_path.clone())
        .map(|p| p.to_string_lossy().to_string());

    bridge.inject(&payload).await?;

    let run_result = bridge.wait(state.config.session_timeout_secs).await;

    // ── Layer 4: Teardown ────────────────────────────────────
    state
        .db
        .update_session_status(session_id, SessionStatus::TearingDown)
        .await?;

    let raw_history = serde_json::json!({"messages": []});

    let (compacted, tokens) =
        teardown::compact::compact_history(&raw_history, state.config.max_context_tokens)?;

    let commit_sha = match &run_result {
        Ok(()) => teardown::persist::save_changes(
            &target_repo,
            &task.branch,
            teardown::persist::DEFAULT_COMMIT_MSG,
        )?,
        Err(_) => None,
    };

    let final_status = if run_result.is_ok() {
        SessionStatus::Completed
    } else {
        SessionStatus::Failed
    };

    state
        .db
        .finalize_session(
            session_id,
            final_status,
            &compacted,
            tokens as i64,
            commit_sha.as_deref(),
        )
        .await?;

    let _ = bridge.teardown().await;

    // Clean up the MCP settings file.
    let _ = std::fs::remove_file(&mcp_settings_host);

    // Clean up the workspace.
    let _ = teardown::persist::cleanup_workspace(&workspace.root);

    // Release the distributed lock.
    lock.release().await?;

    tracing::info!(%session_id, ?final_status, "session lifecycle complete");
    Ok(())
}

/// Load and validate the `AgentStackManifest` from the skills directory.
fn load_manifest(path: &std::path::Path) -> Result<AgentStackManifest> {
    if !path.exists() {
        return Err(ConductorError::InvalidManifest(format!(
            "manifest not found at {}",
            path.display()
        )));
    }
    let content = std::fs::read_to_string(path)?;
    let manifest = AgentStackManifest::from_json(&content)
        .map_err(|e| ConductorError::InvalidManifest(e.to_string()))?;
    tracing::info!(tools = manifest.tools.len(), "manifest loaded");
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_manifest_fails_for_missing_file() {
        let result = load_manifest(std::path::Path::new("/nonexistent/manifest.json"));
        assert!(result.is_err());
    }
}
