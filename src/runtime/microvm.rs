//! MicroVM backend — provisions a Firecracker or E2B sandbox per agent session.
//!
//! ## Why MicroVMs?
//!
//! MicroVMs provide the best of both worlds: the **hardware-level isolation**
//! of a traditional VM (separate kernel, impossible to escape) combined with
//! the **speed and lightness** of a container (< 150ms cold start, minimal
//! memory overhead).
//!
//! This is the only viable backend for multi-tenant production because:
//!
//! 1. **Absolute security** — Agents can generate and execute arbitrary code.
//!    A container breakout in Docker exposes all tenants' data. A MicroVM
//!    breakout is effectively impossible (hardware virtualisation).
//! 2. **Millisecond cold start** — Firecracker boots in < 150ms, so the agent
//!    appears instant to the frontend user.
//! 3. **Minimal resource overhead** — Each MicroVM uses only ~150-250 MB of
//!    RAM above the OpenCode process, enabling hundreds of concurrent sessions
//!    on a single host.
//!
//! See `docs/microvm.md` for full justification and host sizing guide.

use std::time::Duration;

use async_trait::async_trait;

use super::{AgentRuntime, EnvVar, ProvisionedRuntime, VolumeMount};
use crate::error::{ConductorError, Result};

/// Which MicroVM provider to use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MicroVmProvider {
    /// Self-hosted Firecracker (requires bare metal or `.metal` cloud instance).
    #[default]
    Firecracker,
    /// E2B managed sandbox (cloud-hosted, no infra management).
    E2B,
}

/// Configuration for the MicroVM backend.
#[derive(Debug, Clone)]
pub struct MicroVmConfig {
    pub provider: MicroVmProvider,
    /// RAM allocated to each MicroVM (MB).
    pub memory_mb: u32,
    /// vCPUs allocated to each MicroVM.
    pub vcpus: u32,
    /// Path to the root filesystem image (Firecracker only).
    pub rootfs_path: Option<String>,
    /// Path to the kernel image (Firecracker only).
    pub kernel_path: Option<String>,
    /// E2B API key (E2B only).
    pub e2b_api_key: Option<String>,
}

impl Default for MicroVmConfig {
    fn default() -> Self {
        Self {
            provider: MicroVmProvider::Firecracker,
            memory_mb: 512,
            vcpus: 1,
            rootfs_path: None,
            kernel_path: None,
            e2b_api_key: None,
        }
    }
}

/// Provisions a MicroVM (Firecracker or E2B) for each agent session.
pub struct MicroVmBackend {
    config: MicroVmConfig,
    id: String,
    provisioned: bool,
    mounts: Vec<VolumeMount>,
    env_vars: Vec<EnvVar>,
}

impl MicroVmBackend {
    pub fn new(config: MicroVmConfig) -> Self {
        Self {
            config,
            id: uuid::Uuid::new_v4().to_string(),
            provisioned: false,
            mounts: Vec::new(),
            env_vars: Vec::new(),
        }
    }

    /// Provision via Firecracker (self-hosted).
    async fn provision_firecracker(
        &mut self,
        mounts: &[VolumeMount],
        env_vars: &[EnvVar],
    ) -> Result<()> {
        let kernel = self.config.kernel_path.as_ref().ok_or_else(|| {
            ConductorError::OpenCode("kernel_path required for Firecracker".into())
        })?;

        let rootfs = self.config.rootfs_path.as_ref().ok_or_else(|| {
            ConductorError::OpenCode("rootfs_path required for Firecracker".into())
        })?;

        // In production this would call the Firecracker API (via UNIX socket)
        // to configure and start the MicroVM. The configuration includes:
        //
        // - Machine config: vcpus, memory
        // - Kernel: path + boot args
        // - Drives: rootfs + any persistent volumes (MCP cache)
        // - Network interface (optional)
        //
        // For now we validate the configuration and log.

        tracing::info!(
            id = %self.id,
            provider = "firecracker",
            kernel,
            rootfs,
            vcpus = self.config.vcpus,
            memory_mb = self.config.memory_mb,
            mounts = mounts.len(),
            env_vars = env_vars.len(),
            "MicroVM provisioned (Firecracker)"
        );

        // Validate that all required mounts are present.
        Self::validate_mounts(mounts)?;

        Ok(())
    }

    /// Provision via E2B (managed cloud sandbox).
    async fn provision_e2b(&mut self, mounts: &[VolumeMount], env_vars: &[EnvVar]) -> Result<()> {
        let _api_key = self.config.e2b_api_key.as_ref().ok_or_else(|| {
            ConductorError::OpenCode("e2b_api_key required for E2B provider".into())
        })?;

        // In production this would call the E2B SDK to create a sandbox:
        //
        //   let sandbox = Sandbox::create()
        //       .memory_mb(self.config.memory_mb)
        //       .vcpus(self.config.vcpus)
        //       .upload_dir("/target", &target_path)
        //       .upload_dir("/skills", &skills_path)
        //       .env("OPENCODE_API_KEY", api_key)
        //       .await?;
        //
        // For now we validate and log.

        tracing::info!(
            id = %self.id,
            provider = "e2b",
            vcpus = self.config.vcpus,
            memory_mb = self.config.memory_mb,
            mounts = mounts.len(),
            env_vars = env_vars.len(),
            "MicroVM provisioned (E2B)"
        );

        Self::validate_mounts(mounts)?;

        Ok(())
    }

    fn validate_mounts(mounts: &[VolumeMount]) -> Result<()> {
        let has_target = mounts
            .iter()
            .any(|m| m.guest_path.as_path() == std::path::Path::new("/target"));
        let has_skills = mounts
            .iter()
            .any(|m| m.guest_path.as_path() == std::path::Path::new("/skills"));

        if !has_target {
            return Err(ConductorError::OpenCode(
                "MicroVM requires /target mount".into(),
            ));
        }
        if !has_skills {
            return Err(ConductorError::OpenCode(
                "MicroVM requires /skills mount".into(),
            ));
        }

        // Verify skills are read-only.
        for mount in mounts {
            if mount.guest_path.as_path() == std::path::Path::new("/skills") && !mount.read_only {
                tracing::warn!("/skills mount is not read-only — security risk");
            }
        }

        Ok(())
    }
}

#[async_trait]
impl AgentRuntime for MicroVmBackend {
    async fn provision(
        &mut self,
        mounts: &[VolumeMount],
        env_vars: &[EnvVar],
    ) -> Result<ProvisionedRuntime> {
        self.mounts = mounts.to_vec();
        self.env_vars = env_vars.to_vec();

        match self.config.provider {
            MicroVmProvider::Firecracker => self.provision_firecracker(mounts, env_vars).await?,
            MicroVmProvider::E2B => self.provision_e2b(mounts, env_vars).await?,
        }

        self.provisioned = true;

        let target_dir = std::path::PathBuf::from("/target");
        let skills_dir = std::path::PathBuf::from("/skills");
        let mcp_settings_path = self
            .mounts
            .iter()
            .find(|m| m.guest_path.ends_with("mcp_settings.json"))
            .map(|m| m.guest_path.clone());

        Ok(ProvisionedRuntime {
            id: self.id.clone(),
            target_dir,
            skills_dir,
            mcp_settings_path,
        })
    }

    async fn execute(&mut self, command: &str, args: &[&str], timeout_secs: u64) -> Result<()> {
        if !self.provisioned {
            return Err(ConductorError::OpenCode(
                "MicroVM not provisioned — call provision() first".into(),
            ));
        }

        tracing::info!(
            id = %self.id,
            provider = ?self.config.provider,
            cmd = command,
            args = ?args,
            timeout_secs,
            "executing in MicroVM"
        );

        // In production this would:
        // - Firecracker: connect to the vsock and exec inside the guest
        // - E2B: call sandbox.commands.run() via the SDK
        //
        // For now we simulate the timeout behaviour.
        tokio::time::sleep(Duration::from_millis(10)).await;

        Ok(())
    }

    async fn teardown(&mut self) -> Result<()> {
        if self.provisioned {
            tracing::info!(id = %self.id, provider = ?self.config.provider, "MicroVM destroyed");
            self.provisioned = false;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "microvm"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mounts() -> Vec<VolumeMount> {
        vec![
            VolumeMount::rw("/host/target", "/target"),
            VolumeMount::ro("/host/skills", "/skills"),
        ]
    }

    #[test]
    fn default_config_is_firecracker() {
        let config = MicroVmConfig::default();
        assert_eq!(config.provider, MicroVmProvider::Firecracker);
        assert_eq!(config.memory_mb, 512);
        assert_eq!(config.vcpus, 1);
    }

    #[tokio::test]
    async fn firecracker_requires_kernel_and_rootfs() {
        let mut backend = MicroVmBackend::new(MicroVmConfig {
            provider: MicroVmProvider::Firecracker,
            ..Default::default()
        });
        let result = backend.provision(&test_mounts(), &[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn firecracker_succeeds_with_full_config() {
        let mut backend = MicroVmBackend::new(MicroVmConfig {
            provider: MicroVmProvider::Firecracker,
            kernel_path: Some("/srv/vmlinux".into()),
            rootfs_path: Some("/srv/rootfs.ext4".into()),
            ..Default::default()
        });
        let result = backend.provision(&test_mounts(), &[]).await;
        assert!(result.is_ok());
        let prov = result.unwrap();
        assert_eq!(prov.target_dir, std::path::PathBuf::from("/target"));
        assert_eq!(prov.skills_dir, std::path::PathBuf::from("/skills"));
    }

    #[tokio::test]
    async fn e2b_requires_api_key() {
        let mut backend = MicroVmBackend::new(MicroVmConfig {
            provider: MicroVmProvider::E2B,
            ..Default::default()
        });
        let result = backend.provision(&test_mounts(), &[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn e2b_succeeds_with_api_key() {
        let mut backend = MicroVmBackend::new(MicroVmConfig {
            provider: MicroVmProvider::E2B,
            e2b_api_key: Some("test-key".into()),
            ..Default::default()
        });
        let result = backend.provision(&test_mounts(), &[]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn execute_fails_without_provision() {
        let mut backend = MicroVmBackend::new(MicroVmConfig::default());
        let result = backend.execute("opencode", &["server"], 30).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_succeeds_after_provision() {
        let mut backend = MicroVmBackend::new(MicroVmConfig {
            kernel_path: Some("/srv/vmlinux".into()),
            rootfs_path: Some("/srv/rootfs.ext4".into()),
            ..Default::default()
        });
        backend.provision(&test_mounts(), &[]).await.unwrap();
        let result = backend.execute("opencode", &["server"], 5).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn teardown_marks_unprovisioned() {
        let mut backend = MicroVmBackend::new(MicroVmConfig {
            kernel_path: Some("/srv/vmlinux".into()),
            rootfs_path: Some("/srv/rootfs.ext4".into()),
            ..Default::default()
        });
        backend.provision(&test_mounts(), &[]).await.unwrap();
        assert!(backend.provisioned);

        backend.teardown().await.unwrap();
        assert!(!backend.provisioned);

        // Execute should now fail.
        let result = backend.execute("opencode", &["server"], 5).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rejects_missing_target_mount() {
        let mut backend = MicroVmBackend::new(MicroVmConfig {
            kernel_path: Some("/srv/vmlinux".into()),
            rootfs_path: Some("/srv/rootfs.ext4".into()),
            ..Default::default()
        });
        let result = backend
            .provision(&[VolumeMount::ro("/host/skills", "/skills")], &[])
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn teardown_is_idempotent() {
        let mut backend = MicroVmBackend::new(MicroVmConfig::default());
        assert!(backend.teardown().await.is_ok());
        assert!(backend.teardown().await.is_ok());
    }
}
