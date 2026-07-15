//! Execution runtime abstraction — pluggable backends for running agents.
//!
//! The runtime layer decouples **where** OpenCode executes from **how** the
//! orchestrator manages it. Two backends are provided:
//!
//! - [`local::LocalProcessBackend`] — spawns OpenCode as a bare OS process.
//!   Used in development or single-tenant scenarios.
//!
//! - [`microvm::MicroVmBackend`] — provisions a Firecracker / E2B MicroVM for
//!   each agent session. Provides hardware-level isolation, < 150ms cold start,
//!   and minimal memory overhead. The only viable option for multi-tenant
//!   production.
//!
//! Both backends implement the [`AgentRuntime`] trait, so the orchestrator
//! code is identical regardless of execution environment.

pub mod firecracker;
pub mod local;
pub mod microvm;

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::error::Result;

/// Configuration for mounting a directory inside the execution environment.
#[derive(Debug, Clone)]
pub struct VolumeMount {
    /// Path on the host filesystem.
    pub host_path: PathBuf,
    /// Path inside the guest (MicroVM) or working directory (local).
    pub guest_path: PathBuf,
    /// Whether the mount is read-only.
    pub read_only: bool,
}

impl VolumeMount {
    pub fn new(host: impl Into<PathBuf>, guest: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            host_path: host.into(),
            guest_path: guest.into(),
            read_only,
        }
    }

    /// Read-write mount.
    pub fn rw(host: impl Into<PathBuf>, guest: impl Into<PathBuf>) -> Self {
        Self::new(host, guest, false)
    }

    /// Read-only mount.
    pub fn ro(host: impl Into<PathBuf>, guest: impl Into<PathBuf>) -> Self {
        Self::new(host, guest, true)
    }
}

/// Environment variable to inject into the agent's process.
#[derive(Debug, Clone)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

/// A fully-provisioned execution environment ready for OpenCode.
pub struct ProvisionedRuntime {
    /// Unique identifier for this runtime instance.
    pub id: String,
    /// The guest path where `/target` is mounted.
    pub target_dir: PathBuf,
    /// The guest path where `/skills` is mounted.
    pub skills_dir: PathBuf,
    /// The guest path where MCP settings live (if any).
    pub mcp_settings_path: Option<PathBuf>,
}

/// Trait abstracting the execution backend.
///
/// Implementations are responsible for:
/// 1. Provisioning an isolated environment (process or MicroVM)
/// 2. Mounting the target + skills directories
/// 3. Injecting environment variables (API keys, MCP paths)
/// 4. Spawning the OpenCode server inside that environment
/// 5. Enforcing a hard timeout and tearing down
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Provision the environment and start the OpenCode server.
    ///
    /// Returns a [`ProvisionedRuntime`] describing the mounted paths.
    async fn provision(
        &mut self,
        mounts: &[VolumeMount],
        env_vars: &[EnvVar],
    ) -> Result<ProvisionedRuntime>;

    /// Execute a command inside the runtime and wait for completion (or timeout).
    ///
    /// `timeout_secs` is a hard limit — if exceeded, the runtime is forcibly
    /// destroyed.
    async fn execute(&mut self, command: &str, args: &[&str], timeout_secs: u64) -> Result<()>;

    /// Tear down the runtime: kill processes, destroy MicroVMs, free resources.
    async fn teardown(&mut self) -> Result<()>;

    /// Human-readable name of the backend (e.g. `"microvm"`, `"local"`).
    fn name(&self) -> &'static str;
}

/// Which backend to use for agent execution.
#[derive(Debug, Clone, Default)]
pub enum RuntimeKind {
    /// Local bare process — development only.
    #[default]
    Local,
    /// Firecracker MicroVM — production multi-tenant.
    MicroVm,
    /// E2B managed sandbox — cloud-hosted MicroVM.
    E2B,
}

impl RuntimeKind {
    pub fn from_env() -> Self {
        match std::env::var("CONDUCTOR_RUNTIME")
            .unwrap_or_else(|_| "local".into())
            .to_lowercase()
            .as_str()
        {
            "microvm" | "firecracker" => Self::MicroVm,
            "e2b" => Self::E2B,
            _ => Self::Local,
        }
    }
}

/// Factory: create a runtime backend of the requested kind.
pub fn create(kind: &RuntimeKind) -> Box<dyn AgentRuntime> {
    match kind {
        RuntimeKind::Local => Box::new(local::LocalProcessBackend::new()),
        RuntimeKind::MicroVm => Box::new(microvm::MicroVmBackend::new(
            microvm::MicroVmConfig::default(),
        )),
        RuntimeKind::E2B => Box::new(microvm::MicroVmBackend::new(microvm::MicroVmConfig {
            provider: microvm::MicroVmProvider::E2B,
            ..Default::default()
        })),
    }
}

/// Resolve a host path to its guest equivalent given the mount table.
pub fn resolve_guest_path(host_path: &Path, mounts: &[VolumeMount]) -> Option<PathBuf> {
    for mount in mounts {
        if let Ok(rel) = host_path.strip_prefix(&mount.host_path) {
            return Some(mount.guest_path.join(rel));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_mount_rw() {
        let m = VolumeMount::rw("/host/target", "/target");
        assert!(!m.read_only);
    }

    #[test]
    fn volume_mount_ro() {
        let m = VolumeMount::ro("/host/skills", "/skills");
        assert!(m.read_only);
    }

    #[test]
    fn resolve_guest_path_finds_mount() {
        let mounts = vec![
            VolumeMount::rw("/workspace/proj/target", "/target"),
            VolumeMount::ro("/workspace/proj/skills", "/skills"),
        ];
        let resolved = resolve_guest_path(Path::new("/workspace/proj/target/src/main.rs"), &mounts);
        assert_eq!(resolved, Some(PathBuf::from("/target/src/main.rs")));
    }

    #[test]
    fn resolve_guest_path_returns_none_if_not_mounted() {
        let mounts = vec![VolumeMount::rw("/workspace/proj/target", "/target")];
        assert!(resolve_guest_path(Path::new("/etc/passwd"), &mounts).is_none());
    }

    #[test]
    fn runtime_kind_from_env_defaults_local() {
        std::env::remove_var("CONDUCTOR_RUNTIME");
        assert!(matches!(RuntimeKind::from_env(), RuntimeKind::Local));
    }

    #[test]
    fn factory_creates_local() {
        let rt = create(&RuntimeKind::Local);
        assert_eq!(rt.name(), "local");
    }

    #[test]
    fn factory_creates_microvm() {
        let rt = create(&RuntimeKind::MicroVm);
        assert_eq!(rt.name(), "microvm");
    }

    #[test]
    fn factory_creates_e2b() {
        let rt = create(&RuntimeKind::E2B);
        assert_eq!(rt.name(), "microvm");
    }
}
