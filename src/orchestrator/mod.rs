//! The orchestration lifecycle — ties all layers together.
//!
//! ```text
//!  ┌───────────┐   ┌────────────┐   ┌───────────────┐   ┌──────────────┐
//!  │ 1. State  │──▶│ 2. Checkout│──▶│ 3. Bridge     │──▶│ 4. Teardown  │
//!  │ Lock + DB │   │ target +   │   │ Runtime + MCP │   │ compact +    │
//!  │           │◀──│  skills    │◀──│ + inject      │◀──│ persist      │
//!  └───────────┘   └────────────┘   └───────────────┘   └──────────────┘
//! ```

use std::time::Instant;

use uuid::Uuid;

use crate::bridge::{manifest::AgentStackManifest, BridgeConfig, OpenCodeBridge};
use crate::checkout::{credentials::GitCredentials, skills, target, Workspace};
use crate::error::{ConductorError, Result};
use crate::mcp::{preindex, CacheMode, CodebaseMemoryMcp};
use crate::models::{AgentTask, SessionStatus};
use crate::runtime::{self, RuntimeKind};
use crate::server::AppState;
use crate::teardown;

/// Execute the full agent session lifecycle.
pub async fn run_session(state: AppState, session_id: Uuid, task: AgentTask) -> Result<()> {
    let session_start = Instant::now();

    // ── Layer 1: Acquire distributed lock ────────────────────
    state
        .db
        .update_session_status(session_id, SessionStatus::Preparing)
        .await?;

    crate::server::metrics_recorder::lock_acquired("attempted");

    let lock = state.locks.acquire(task.tenant_id, &task.branch).await?;

    crate::server::metrics_recorder::lock_acquired("acquired");

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

    let _creds = GitCredentials::from_env();
    let target_repo = target::prepare_target(&workspace.target, &project.repo_url, &task.branch)?;
    let _skills_repo = skills::prepare_skills(&workspace.skills, &task.skills_repo)?;

    // Load the manifest from the skills directory.
    let manifest_path = workspace.skills.join("manifest.json");
    let manifest = load_manifest(&manifest_path)?;

    // ── Smart MCP Pre-indexing ───────────────────────────────
    let analysis = preindex::analyze_repo(&target_repo)?;
    let cache_mode = preindex::decide_cache_mode(&analysis, workspace.mcp_cache.exists());

    let mcp = CodebaseMemoryMcp::new("/target", cache_mode)
        .map_err(|e| ConductorError::Other(format!("MCP setup failed: {e}")))?;

    let mcp_settings_host = workspace.root.join("mcp_settings.json");
    mcp.write_settings(&mcp_settings_host)?;

    if cache_mode == CacheMode::Persistent {
        workspace.ensure_mcp_cache()?;

        // Check if existing cache is valid (skip re-index).
        if preindex::is_cache_valid(&analysis, &workspace.mcp_cache)? {
            tracing::info!(session_id = %session_id, "MCP cache valid — no re-index needed");
        }
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

    crate::server::metrics_recorder::microvm_provisioned(bridge.runtime_name());

    let mut payload =
        OpenCodeBridge::build_payload(session_id, &task.instruction, history, &manifest);
    payload.mcp_settings_path = bridge
        .provisioned()
        .and_then(|p| p.mcp_settings_path.clone())
        .map(|p| p.to_string_lossy().to_string());

    bridge.inject(&payload).await?;

    // Publish status event for WebSocket subscribers.
    state.events.publish(
        session_id,
        crate::bridge::LogEvent::Status {
            status: "running".into(),
        },
    );

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

    // Update MCP cache metadata if persistent.
    if cache_mode == CacheMode::Persistent {
        let meta = preindex::CacheMeta {
            indexed_commit_sha: analysis.head_sha.clone(),
            indexed_at: chrono::Utc::now(),
            file_count: analysis.file_count,
            total_size_bytes: analysis.total_size_bytes,
        };
        let _ = preindex::write_cache_meta(&workspace.mcp_cache, &meta);
    }

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

    // Publish final event for WebSocket subscribers.
    state.events.publish(
        session_id,
        crate::bridge::LogEvent::Done {
            commit_sha: commit_sha.clone(),
        },
    );

    // Metrics
    let duration = session_start.elapsed();
    crate::server::metrics_recorder::session_completed(&tenant.slug, run_result.is_ok());
    crate::server::metrics_recorder::session_duration(duration);
    crate::server::metrics_recorder::tokens_consumed(&tenant.slug, tokens as u64);

    let _ = bridge.teardown().await;
    let _ = std::fs::remove_file(&mcp_settings_host);
    let _ = teardown::persist::cleanup_workspace(&workspace.root);
    state.events.remove_session(session_id);

    lock.release().await?;

    tracing::info!(%session_id, ?final_status, ?duration, "session lifecycle complete");
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
