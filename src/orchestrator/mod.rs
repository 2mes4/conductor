//! The orchestration lifecycle — ties the four architectural layers together.
//!
//! ```text
//!  ┌─────────┐    ┌──────────┐    ┌─────────┐    ┌──────────┐
//!  │  1.State │───▶│2.Checkout│───▶│3.Bridge │───▶│4.Teardown│
//!  │ Lock+DB  │    │ target+   │    │ OpenCode│    │ compact+ │
//!  │          │◀───│  skills   │◀───│ inject  │◀───│ persist  │
//!  └─────────┘    └──────────┘    └─────────┘    └──────────┘
//! ```

use std::path::PathBuf;

use uuid::Uuid;

use crate::bridge::{manifest::AgentStackManifest, OpenCodeBridge};
use crate::checkout::{skills, target, Workspace};
use crate::error::{ConductorError, Result};
use crate::models::{AgentTask, SessionStatus};
use crate::server::AppState;
use crate::teardown;

/// Execute the full agent session lifecycle.
///
/// This function orchestrates all four layers sequentially:
///
/// 1. **Acquire lock** — `pg_advisory_lock` for the project/branch pair.
/// 2. **Prepare workspace** — Dual-Checkout of `/target` and `/skills`.
/// 3. **Run agent** — spawn OpenCode, inject payload, wait for completion.
/// 4. **Teardown** — compact history, commit + push, persist to DB, cleanup.
pub async fn run_session(
    state: AppState,
    session_id: Uuid,
    task: AgentTask,
) -> Result<()> {
    // ── Layer 1: Acquire distributed lock ────────────────────
    state
        .db
        .update_session_status(session_id, SessionStatus::Preparing)
        .await?;

    let lock = state.locks.acquire(task.tenant_id, task.branch()).await?;

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

    let target_repo =
        target::prepare_target(&workspace.target, &project.repo_url, &task.branch)?;
    let _skills_repo = skills::prepare_skills(&workspace.skills, &task.skills_repo)?;

    // Load the manifest from the skills directory.
    let manifest_path = workspace.skills.join("manifest.json");
    let manifest = load_manifest(&manifest_path)?;

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

    // ── Layer 3: Bridge — spawn OpenCode & inject payload ────
    state
        .db
        .update_session_status(session_id, SessionStatus::Running)
        .await?;

    let opencode_path = std::env::var("OPENCODE_PATH")
        .unwrap_or_else(|_| "opencode".into());

    let mut bridge = OpenCodeBridge::spawn(
        &PathBuf::from(&opencode_path),
        &workspace.target,
        &workspace.skills,
        &std::env::var("OPENCODE_API_KEY").unwrap_or_default(),
    )
    .await?;

    let payload = OpenCodeBridge::build_payload(
        session_id,
        &task.instruction,
        history,
        &manifest,
    );
    bridge.inject(&payload).await?;

    let run_result = bridge
        .wait_with_timeout(state.config.session_timeout_secs)
        .await;

    // ── Layer 4: Teardown ────────────────────────────────────
    state
        .db
        .update_session_status(session_id, SessionStatus::TearingDown)
        .await?;

    // Extract session output (placeholder — real implementation reads from
    // the OpenCode process stdout/API).
    let raw_history = serde_json::json!({"messages": []});

    // Compact if over token budget.
    let (compacted, tokens) =
        teardown::compact::compact_history(&raw_history, state.config.max_context_tokens)?;

    // Commit + push changes.
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

    // Kill the OpenCode process if still alive.
    let _ = bridge.kill().await;

    // Clean up the workspace.
    let _ = teardown::persist::cleanup_workspace(&workspace.root);

    // Release the distributed lock.
    lock.release().await?;

    tracing::info!(%session_id, ?final_status, "session lifecycle complete");
    Ok(())
}

impl AgentTask {
    fn branch(&self) -> &str {
        &self.branch
    }
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
