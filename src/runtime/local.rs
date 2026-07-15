//! Local process backend — spawns OpenCode as a bare OS process.
//!
//! Use only in development or single-tenant scenarios. No isolation is
//! provided; the agent process shares the host kernel and filesystem.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::{Child, Command};

use super::{AgentRuntime, EnvVar, ProvisionedRuntime, VolumeMount};
use crate::error::{ConductorError, Result};

/// Spawns OpenCode as a local child process.
pub struct LocalProcessBackend {
    child: Option<Child>,
    id: String,
    mounts: Vec<VolumeMount>,
}

impl LocalProcessBackend {
    pub fn new() -> Self {
        Self {
            child: None,
            id: uuid::Uuid::new_v4().to_string(),
            mounts: Vec::new(),
        }
    }
}

impl Default for LocalProcessBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentRuntime for LocalProcessBackend {
    async fn provision(
        &mut self,
        mounts: &[VolumeMount],
        env_vars: &[EnvVar],
    ) -> Result<ProvisionedRuntime> {
        self.mounts = mounts.to_vec();

        // In local mode, guest paths map 1:1 to host paths (no translation).
        let target_dir = mounts
            .iter()
            .find(|m| m.guest_path == std::path::Path::new("/target"))
            .map(|m| m.host_path.clone())
            .ok_or_else(|| ConductorError::OpenCode("missing /target mount".into()))?;

        let skills_dir = mounts
            .iter()
            .find(|m| m.guest_path == std::path::Path::new("/skills"))
            .map(|m| m.host_path.clone())
            .ok_or_else(|| ConductorError::OpenCode("missing /skills mount".into()))?;

        let mcp_settings_path = mounts
            .iter()
            .find(|m| m.guest_path.ends_with("mcp_settings.json"))
            .map(|m| m.host_path.clone());

        // Verify the OpenCode binary is accessible.
        let opencode_bin = std::env::var("OPENCODE_PATH").unwrap_or_else(|_| "opencode".into());
        let which = tokio::process::Command::new("which")
            .arg(&opencode_bin)
            .output()
            .await;

        if which.is_err() || !which.as_ref().unwrap().status.success() {
            tracing::warn!(
                bin = %opencode_bin,
                "OpenCode binary not found — provision will fail at execute time"
            );
        }

        tracing::info!(
            id = %self.id,
            ?target_dir,
            ?skills_dir,
            env_vars = env_vars.len(),
            "local runtime provisioned"
        );

        Ok(ProvisionedRuntime {
            id: self.id.clone(),
            target_dir,
            skills_dir,
            mcp_settings_path,
        })
    }

    async fn execute(&mut self, command: &str, args: &[&str], timeout_secs: u64) -> Result<()> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Inherit environment — API keys are set via env_vars at provision time
        // in production; locally we rely on the shell environment.
        cmd.kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| ConductorError::OpenCode(format!("failed to spawn '{command}': {e}")))?;

        self.child = Some(child);
        tracing::info!(cmd = command, timeout_secs, "local process started");

        let child_ref = self
            .child
            .as_mut()
            .ok_or_else(|| ConductorError::OpenCode("no process running".into()))?;

        let result =
            tokio::time::timeout(Duration::from_secs(timeout_secs), child_ref.wait()).await;

        match result {
            Ok(Ok(status)) if status.success() => {
                tracing::info!("local process exited successfully");
                Ok(())
            }
            Ok(Ok(status)) => Err(ConductorError::OpenCode(format!(
                "process exited with status {status}"
            ))),
            Ok(Err(e)) => Err(ConductorError::OpenCode(format!("wait error: {e}"))),
            Err(_) => {
                tracing::warn!("local process timed out — killing");
                self.teardown().await?;
                Err(ConductorError::SessionTimeout(timeout_secs))
            }
        }
    }

    async fn teardown(&mut self) -> Result<()> {
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
            let _ = child.wait().await;
            tracing::info!("local process torn down");
        }
        self.child = None;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "local"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn provision_requires_target_mount() {
        let mut rt = LocalProcessBackend::new();
        let result = rt
            .provision(&[VolumeMount::ro("/skills", "/skills")], &[])
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn provision_requires_skills_mount() {
        let mut rt = LocalProcessBackend::new();
        let result = rt
            .provision(&[VolumeMount::rw("/target", "/target")], &[])
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn provision_succeeds_with_both_mounts() {
        let mut rt = LocalProcessBackend::new();
        let result = rt
            .provision(
                &[
                    VolumeMount::rw("/host/target", "/target"),
                    VolumeMount::ro("/host/skills", "/skills"),
                ],
                &[],
            )
            .await;
        assert!(result.is_ok());
        let prov = result.unwrap();
        assert_eq!(prov.target_dir, std::path::PathBuf::from("/host/target"));
        assert_eq!(prov.skills_dir, std::path::PathBuf::from("/host/skills"));
    }

    #[tokio::test]
    async fn execute_times_out() {
        let mut rt = LocalProcessBackend::new();
        rt.provision(
            &[
                VolumeMount::rw("/tmp", "/target"),
                VolumeMount::ro("/tmp", "/skills"),
            ],
            &[],
        )
        .await
        .unwrap();

        let result = rt.execute("sleep", &["10"], 1).await;
        assert!(matches!(result, Err(ConductorError::SessionTimeout(1))));
    }

    #[tokio::test]
    async fn teardown_is_idempotent() {
        let mut rt = LocalProcessBackend::new();
        assert!(rt.teardown().await.is_ok());
        assert!(rt.teardown().await.is_ok());
    }
}
