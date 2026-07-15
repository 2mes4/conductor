//! Firecracker MicroVM REST client — HTTP/1.1 over a UNIX domain socket.
//!
//! Firecracker exposes a simple REST API on a UNIX socket. This client sends
//! PUT requests to configure the machine, boot source, drives, and actions.
//!
//! All communication is done via raw HTTP over `tokio::net::UnixStream` — no
//! extra HTTP library needed since Firecracker's API is trivially simple.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::error::{ConductorError, Result};

/// Firecracker REST API client.
pub struct FirecrackerClient {
    socket_path: PathBuf,
}

/// Machine configuration for a Firecracker MicroVM.
#[derive(Debug, Serialize)]
pub struct MachineConfig {
    pub vcpu_count: u32,
    pub mem_size_mib: u32,
}

/// Boot source (kernel + args).
#[derive(Debug, Serialize)]
pub struct BootSource {
    pub kernel_image_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_args: Option<String>,
}

/// Drive configuration (root filesystem).
#[derive(Debug, Serialize)]
pub struct DriveConfig {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
}

/// Action to perform on the MicroVM.
#[derive(Debug, Serialize)]
pub struct InstanceAction {
    pub action_type: String,
}

impl FirecrackerClient {
    /// Create a client pointing at the given Firecracker API socket.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// Set the machine configuration (vCPUs + memory).
    pub async fn put_machine_config(&self, config: &MachineConfig) -> Result<()> {
        self.put("/machine-config", config).await
    }

    /// Set the boot source (kernel path + optional boot args).
    pub async fn put_boot_source(&self, kernel_path: &str, boot_args: Option<&str>) -> Result<()> {
        let source = BootSource {
            kernel_image_path: kernel_path.to_string(),
            boot_args: boot_args.map(String::from),
        };
        self.put("/boot-source", &source).await
    }

    /// Attach a drive (root filesystem or data volume).
    pub async fn put_drive(&self, drive: &DriveConfig) -> Result<()> {
        let path = format!("/drives/{}", drive.drive_id);
        self.put(&path, drive).await
    }

    /// Start the MicroVM.
    pub async fn start(&self) -> Result<()> {
        let action = InstanceAction {
            action_type: "InstanceStart".into(),
        };
        self.put("/actions", &action).await
    }

    /// Stop the MicroVM (send CtrlAltDel).
    pub async fn send_ctrl_alt_del(&self) -> Result<()> {
        let action = InstanceAction {
            action_type: "SendCtrlAltDel".into(),
        };
        self.put("/actions", &action).await
    }

    /// Configure the MicroVM and start it in one call.
    pub async fn create_and_start(
        &self,
        machine: &MachineConfig,
        kernel_path: &str,
        boot_args: &str,
        rootfs_path: &str,
    ) -> Result<()> {
        self.put_machine_config(machine).await?;
        self.put_boot_source(kernel_path, Some(boot_args)).await?;
        self.put_drive(&DriveConfig {
            drive_id: "rootfs".into(),
            path_on_host: rootfs_path.to_string(),
            is_root_device: true,
            is_read_only: false,
        })
        .await?;
        self.start().await?;
        tracing::info!("Firecracker MicroVM created and started");
        Ok(())
    }

    // ── Internal HTTP-over-UNIX-socket implementation ─────────

    async fn put<T: Serialize>(&self, path: &str, body: &T) -> Result<()> {
        let json = serde_json::to_string(body)?;
        self.put_raw(path, &json).await
    }

    async fn put_raw(&self, path: &str, body: &str) -> Result<()> {
        let mut stream = UnixStream::connect(&self.socket_path).await.map_err(|e| {
            ConductorError::OpenCode(format!(
                "failed to connect to Firecracker socket {}: {e}",
                self.socket_path.display()
            ))
        })?;

        let request = format!(
            "PUT {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| ConductorError::OpenCode(format!("Firecracker write failed: {e}")))?;

        let mut response = Vec::with_capacity(1024);
        stream
            .read_to_end(&mut response)
            .await
            .map_err(|e| ConductorError::OpenCode(format!("Firecracker read failed: {e}")))?;

        let response_str = String::from_utf8_lossy(&response);

        if let Some(status_line) = response_str.lines().next() {
            let parts: Vec<&str> = status_line.split_whitespace().collect();
            if parts.len() >= 2 {
                let status_code: u16 = parts[1].parse().unwrap_or(0);
                if !(200..300).contains(&status_code) {
                    return Err(ConductorError::OpenCode(format!(
                        "Firecracker API error: {status_line} — body: {response_str}"
                    )));
                }
            }
        }

        Ok(())
    }

    /// Check if the socket exists and is connectable.
    pub async fn is_reachable(&self) -> bool {
        if !self.socket_path.exists() {
            return false;
        }
        UnixStream::connect(&self.socket_path).await.is_ok()
    }

    /// Get the socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_config_serializes_correctly() {
        let config = MachineConfig {
            vcpu_count: 2,
            mem_size_mib: 1024,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"vcpu_count\":2"));
        assert!(json.contains("\"mem_size_mib\":1024"));
    }

    #[test]
    fn boot_source_omits_null_args() {
        let source = BootSource {
            kernel_image_path: "/vmlinux".into(),
            boot_args: None,
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(!json.contains("boot_args"));
    }

    #[test]
    fn boot_source_includes_args() {
        let source = BootSource {
            kernel_image_path: "/vmlinux".into(),
            boot_args: Some("console=ttyS0".into()),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("console=ttyS0"));
    }

    #[test]
    fn client_stores_socket_path() {
        let client = FirecrackerClient::new("/tmp/firecracker.sock");
        assert_eq!(client.socket_path(), Path::new("/tmp/firecracker.sock"));
    }

    #[tokio::test]
    async fn is_reachable_returns_false_for_nonexistent_socket() {
        let client = FirecrackerClient::new("/tmp/nonexistent_firecracker_socket_test.sock");
        assert!(!client.is_reachable().await);
    }
}
