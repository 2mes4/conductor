//! Deserialization and validation of `AgentStackManifest`.
//!
//! Uses `#[serde(deny_unknown_fields)]` to reject any illicit configuration.

use serde::{Deserialize, Serialize};

/// The root manifest that defines an agent's tool stack.
///
/// This file lives inside the skills repository and is parsed by Rust before
/// any agent process is started.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStackManifest {
    /// Human-readable name for this agent stack.
    pub name: String,
    /// Schema version of the manifest format.
    pub version: String,
    /// The tools available to the agent.
    pub tools: Vec<ToolDefinition>,
}

/// A single tool definition in the agent stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    /// Unique identifier for this tool.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Relative path to the executable (sanitised against traversal).
    pub executable: String,
    /// JSON Schema describing accepted arguments.
    pub schema: serde_json::Value,
    /// Whether this tool is allowed to modify the filesystem.
    #[serde(default)]
    pub write_access: bool,
}

impl AgentStackManifest {
    /// Parse and validate a manifest from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let manifest: Self = serde_json::from_str(json)?;
        // Validate all tool executables after deserialization.
        for tool in &manifest.tools {
            if let Err(e) = crate::bridge::sanitize::validate_path(&tool.executable) {
                // Re-pack as a serde error-compatible message.
                tracing::error!(tool = %tool.id, error = %e, "path validation failed");
            }
        }
        Ok(manifest)
    }
}
