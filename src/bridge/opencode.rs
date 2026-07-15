//! OpenCode Server bridge — spawns the agent inside a runtime (local process
//! or MicroVM), injects the session payload, and controls execution.
//!
//! Rust generates ephemeral environment variables (API keys), provisions the
//! execution backend, mounts `/target` as read-write and `/skills` as
//! read-only, writes the MCP settings file, and injects the session payload
//! (history, tool schemas, instruction).

use std::path::PathBuf;

use uuid::Uuid;

use crate::bridge::manifest::AgentStackManifest;
use crate::error::Result;
use crate::runtime::{AgentRuntime, EnvVar, ProvisionedRuntime, VolumeMount};

/// The payload injected into OpenCode via its API.
#[derive(Debug, serde::Serialize)]
pub struct InjectionPayload {
    pub session_id: Uuid,
    pub instruction: String,
    /// Recovered and (optionally pre-compacted) history from Postgres.
    pub history: serde_json::Value,
    /// JSON Schemas of the tools mounted at `/skills`.
    pub tool_schemas: Vec<serde_json::Value>,
    /// Path to the MCP settings file inside the guest (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_settings_path: Option<String>,
}

/// Manages the lifecycle of a single OpenCode session inside a runtime backend.
pub struct OpenCodeBridge {
    runtime: Box<dyn AgentRuntime>,
    provisioned: Option<ProvisionedRuntime>,
    /// Path on the host where the MCP settings JSON was written.
    mcp_settings_host_path: Option<PathBuf>,
}

/// Configuration for provisioning an OpenCode session.
pub struct BridgeConfig {
    /// Host path to the target checkout.
    pub target_host: PathBuf,
    /// Host path to the skills checkout.
    pub skills_host: PathBuf,
    /// Host path to the MCP settings file (if MCP is enabled).
    pub mcp_settings_host: Option<PathBuf>,
    /// Host path to persistent MCP cache (if persistent mode).
    pub mcp_cache_host: Option<PathBuf>,
    /// API key to inject.
    pub api_key: String,
    /// Hard timeout in seconds.
    pub timeout_secs: u64,
}

impl OpenCodeBridge {
    /// Create a new bridge with the given runtime backend.
    pub fn new(runtime: Box<dyn AgentRuntime>) -> Self {
        Self {
            runtime,
            provisioned: None,
            mcp_settings_host_path: None,
        }
    }

    /// Provision the runtime, mount directories, and start OpenCode.
    pub async fn start(&mut self, config: &BridgeConfig) -> Result<()> {
        let mut mounts = vec![
            VolumeMount::rw(&config.target_host, "/target"),
            VolumeMount::ro(&config.skills_host, "/skills"),
        ];

        // Mount MCP settings if present.
        if let Some(mcp_settings) = &config.mcp_settings_host {
            mounts.push(VolumeMount::ro(mcp_settings, "/config/mcp_settings.json"));
        }

        // Mount persistent MCP cache if present.
        if let Some(mcp_cache) = &config.mcp_cache_host {
            mounts.push(VolumeMount::rw(mcp_cache, "/target/.mcp_cache"));
        }

        let env_vars = vec![
            EnvVar {
                key: "OPENCODE_API_KEY".into(),
                value: config.api_key.clone(),
            },
            EnvVar {
                key: "OPENCODE_TARGET_DIR".into(),
                value: "/target".into(),
            },
            EnvVar {
                key: "OPENCODE_SKILLS_DIR".into(),
                value: "/skills".into(),
            },
            EnvVar {
                key: "OPENCODE_SKILLS_READONLY".into(),
                value: "true".into(),
            },
        ];

        let provisioned = self.runtime.provision(&mounts, &env_vars).await?;
        tracing::info!(
            runtime = self.runtime.name(),
            id = %provisioned.id,
            "OpenCode runtime provisioned"
        );

        self.mcp_settings_host_path = config.mcp_settings_host.clone();
        self.provisioned = Some(provisioned);
        Ok(())
    }

    /// Inject the session payload into the running OpenCode instance.
    pub async fn inject(&self, payload: &InjectionPayload) -> Result<()> {
        let json = serde_json::to_string_pretty(payload)?;
        tracing::debug!(payload = %json, "injecting payload into OpenCode");
        // TODO: POST to OpenCode Server API once its HTTP interface is stable.
        Ok(())
    }

    /// Wait for the OpenCode process to complete or enforce a timeout.
    pub async fn wait(&mut self, timeout_secs: u64) -> Result<()> {
        let opencode_bin = std::env::var("OPENCODE_PATH").unwrap_or_else(|_| "opencode".into());

        self.runtime
            .execute(&opencode_bin, &["server"], timeout_secs)
            .await
    }

    /// Teardown the runtime — kill processes, destroy MicroVMs, free resources.
    pub async fn teardown(&mut self) -> Result<()> {
        self.runtime.teardown().await?;
        self.provisioned = None;

        // Clean up the ephemeral MCP settings file.
        if let Some(path) = &self.mcp_settings_host_path {
            let _ = std::fs::remove_file(path);
        }

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
            mcp_settings_path: None,
        }
    }

    /// Get the provisioned runtime info.
    pub fn provisioned(&self) -> Option<&ProvisionedRuntime> {
        self.provisioned.as_ref()
    }

    /// Get the runtime backend name.
    pub fn runtime_name(&self) -> &'static str {
        self.runtime.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_payload_collects_tool_schemas() {
        let manifest_json = r#"{
            "name": "test",
            "version": "1.0",
            "tools": [{
                "id": "lint",
                "name": "Linter",
                "executable": "tools/lint.sh",
                "schema": {"type": "object"},
                "write_access": false
            }]
        }"#;
        let manifest = AgentStackManifest::from_json(manifest_json).unwrap();
        let payload = OpenCodeBridge::build_payload(
            Uuid::new_v4(),
            "test instruction",
            serde_json::json!({"messages": []}),
            &manifest,
        );

        assert_eq!(payload.tool_schemas.len(), 1);
        assert_eq!(payload.instruction, "test instruction");
    }
}
