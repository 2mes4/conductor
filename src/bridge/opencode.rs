//! OpenCode Server bridge — spawn, inject payload, and control the agent
//! process.
//!
//! Rust generates ephemeral environment variables (API keys), starts the
//! OpenCode Server process, mounts `/target` as read-write and `/skills` as
//! read-only, and injects the session payload (history, tool schemas,
//! instruction).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::{Child, Command};
use uuid::Uuid;

use crate::bridge::manifest::AgentStackManifest;
use crate::error::{ConductorError, Result};

/// The payload injected into OpenCode via its API.
#[derive(Debug, serde::Serialize)]
pub struct InjectionPayload {
    pub session_id: Uuid,
    pub instruction: String,
    /// Recovered and (optionally pre-compacted) history from Postgres.
    pub history: serde_json::Value,
    /// JSON Schemas of the tools mounted at `/skills`.
    pub tool_schemas: Vec<serde_json::Value>,
}

/// Manages the lifecycle of a single OpenCode Server process.
pub struct OpenCodeBridge {
    child: Option<Child>,
    workspace_target: PathBuf,
    workspace_skills: PathBuf,
}

impl OpenCodeBridge {
    /// Spawn an OpenCode Server process with the correct mounts and env vars.
    pub async fn spawn(
        opencode_path: &Path,
        target: &Path,
        skills: &Path,
        api_key: &str,
    ) -> Result<Self> {
        let child = Command::new(opencode_path)
            .arg("server")
            .env("OPENCODE_API_KEY", api_key)
            .env("OPENCODE_TARGET_DIR", target)
            .env("OPENCODE_SKILLS_DIR", skills)
            .env("OPENCODE_SKILLS_READONLY", "true")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ConductorError::OpenCode(format!("failed to spawn: {e}")))?;

        tracing::info!("OpenCode server process spawned");

        Ok(Self {
            child: Some(child),
            workspace_target: target.to_path_buf(),
            workspace_skills: skills.to_path_buf(),
        })
    }

    /// Inject the session payload into the running OpenCode instance.
    ///
    /// In production this performs an HTTP POST to the OpenCode API. For now
    /// this is a placeholder that serialises the payload for logging.
    pub async fn inject(&self, payload: &InjectionPayload) -> Result<()> {
        let json = serde_json::to_string_pretty(payload)?;
        tracing::debug!(payload = %json, "injecting payload into OpenCode");
        // TODO: POST to OpenCode Server API once its HTTP interface is stable.
        Ok(())
    }

    /// Build the injection payload from the manifest, history, and instruction.
    pub fn build_payload(
        session_id: Uuid,
        instruction: &str,
        history: serde_json::Value,
        manifest: &AgentStackManifest,
    ) -> InjectionPayload {
        let tool_schemas = manifest.tools.iter().map(|t| t.schema.clone()).collect();
        InjectionPayload {
            session_id,
            instruction: instruction.to_string(),
            history,
            tool_schemas,
        }
    }

    /// Wait for the OpenCode process to complete or enforce a timeout.
    pub async fn wait_with_timeout(&mut self, timeout_secs: u64) -> Result<()> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| ConductorError::OpenCode("process not running".into()))?;

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await;

        match result {
            Ok(Ok(status)) if status.success() => {
                tracing::info!("OpenCode process exited successfully");
                Ok(())
            }
            Ok(Ok(status)) => Err(ConductorError::OpenCode(format!(
                "OpenCode exited with status {status}"
            ))),
            Ok(Err(e)) => Err(ConductorError::OpenCode(format!("wait error: {e}"))),
            Err(_) => {
                tracing::warn!("OpenCode timed out — killing process");
                self.kill().await?;
                Err(ConductorError::SessionTimeout(timeout_secs))
            }
        }
    }

    /// Kill the OpenCode process if it is still running.
    pub async fn kill(&mut self) -> Result<()> {
        if let Some(child) = &mut self.child {
            child
                .kill()
                .await
                .map_err(|e| ConductorError::OpenCode(format!("failed to kill process: {e}")))?;
            tracing::warn!("OpenCode process killed");
        }
        Ok(())
    }

    pub fn target_dir(&self) -> &Path {
        &self.workspace_target
    }

    pub fn skills_dir(&self) -> &Path {
        &self.workspace_skills
    }
}

impl Drop for OpenCodeBridge {
    fn drop(&mut self) {
        // Best-effort synchronous kill if the async kill wasn't called.
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}
