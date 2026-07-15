//! codebase-memory-mcp — structured codebase access via a local knowledge graph.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ConductorError, Result};

/// Whether to persist the MCP SQLite graph between sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheMode {
    /// Build the graph in RAM from scratch each session (default, recommended).
    #[default]
    Ephemeral,
    /// Cache the graph on disk for incremental re-indexing of large repos.
    Persistent,
}

/// Configuration for a single MCP server entry in the settings file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// The command to launch the MCP server.
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Environment variables for the MCP server process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<serde_json::Value>,
}

/// Manages the codebase-memory-mcp lifecycle configuration.
pub struct CodebaseMemoryMcp {
    /// Path to the pre-baked MCP binary inside the guest.
    binary_path: PathBuf,
    /// The root directory to index (always `/target`).
    scan_root: PathBuf,
    /// Where to store the SQLite graph (in-memory or persistent path).
    cache_mode: CacheMode,
    /// Host-side path for persistent cache (if applicable).
    cache_path: Option<PathBuf>,
}

impl CodebaseMemoryMcp {
    /// Create a new MCP config builder.
    ///
    /// - `scan_root` must be inside the agent's `/target` directory.
    /// - `cache_path` is the host path for persistent caching (if enabled).
    pub fn new(scan_root: impl Into<PathBuf>, cache_mode: CacheMode) -> Result<Self> {
        let scan_root = scan_root.into();

        // The scan root must be an absolute guest path (starts with /).
        if !scan_root.is_absolute() {
            return Err(ConductorError::PathTraversal(format!(
                "MCP scan root must be absolute: {}",
                scan_root.display()
            )));
        }

        Ok(Self {
            binary_path: PathBuf::from("/usr/local/bin/codebase-memory-mcp"),
            scan_root,
            cache_mode,
            cache_path: None,
        })
    }

    /// Set a custom binary path (e.g. for local development).
    pub fn with_binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary_path = path.into();
        self
    }

    /// Set the persistent cache host path (for `CacheMode::Persistent`).
    pub fn with_cache_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_path = Some(path.into());
        self
    }

    /// Get the guest-side binary path.
    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    /// Get the scan root (always `/target` or a subdirectory).
    pub fn scan_root(&self) -> &Path {
        &self.scan_root
    }

    /// Get the cache mode.
    pub fn cache_mode(&self) -> CacheMode {
        self.cache_mode
    }

    /// Get the host-side cache path for persistent mode.
    pub fn cache_path(&self) -> Option<&Path> {
        self.cache_path.as_deref()
    }

    /// Build the `mcp_settings.json` content that OpenCode expects.
    ///
    /// This JSON instructs OpenCode to launch the MCP server as a subprocess,
    /// scoped to the agent's `/target` directory.
    pub fn build_settings(&self) -> Result<serde_json::Value> {
        let mut args = vec![self.scan_root.to_string_lossy().to_string()];

        let env = match self.cache_mode {
            CacheMode::Ephemeral => {
                args.push("--memory".into());
                args.push(":memory:".into());
                None
            }
            CacheMode::Persistent => {
                let guest_cache = "/target/.mcp_cache/graph.db";
                args.push("--memory".into());
                args.push(guest_cache.into());
                Some(serde_json::json!({
                    "MCP_CACHE_DIR": guest_cache
                }))
            }
        };

        // Validate the binary path doesn't contain traversal (sanity check).
        let bin_str = self.binary_path.to_string_lossy();
        if bin_str.contains("..") {
            return Err(ConductorError::PathTraversal(bin_str.to_string()));
        }

        let server = McpServerConfig {
            command: bin_str.to_string(),
            args,
            env,
        };

        let settings = serde_json::json!({
            "mcpServers": {
                "codebase-memory": server
            }
        });

        Ok(settings)
    }

    /// Write the settings JSON to a file on the host (to be mounted into the guest).
    pub fn write_settings(&self, output_path: &Path) -> Result<()> {
        let settings = self.build_settings()?;
        let json = serde_json::to_string_pretty(&settings)?;
        std::fs::write(output_path, json)?;
        tracing::info!(
            output = %output_path.display(),
            scan_root = %self.scan_root.display(),
            ?self.cache_mode,
            "MCP settings written"
        );
        Ok(())
    }

    /// Ensure the persistent cache directory exists on the host.
    pub fn ensure_cache_dir(&self) -> Result<()> {
        if let Some(path) = &self.cache_path {
            std::fs::create_dir_all(path)?;
            tracing::debug!(cache_dir = %path.display(), "MCP cache dir ensured");
        }
        Ok(())
    }
}

/// Compute the default persistent cache path for a workspace.
pub fn default_cache_path(workspace_root: &Path, tenant: &str, project: &str) -> PathBuf {
    workspace_root.join(tenant).join(project).join(".mcp_cache")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn ephemeral_builds_settings() {
        let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Ephemeral).unwrap();
        let settings = mcp.build_settings().unwrap();

        let servers = settings.get("mcpServers").unwrap();
        let server = servers.get("codebase-memory").unwrap();
        let command = server.get("command").unwrap().as_str().unwrap();
        let args = server.get("args").unwrap().as_array().unwrap();

        assert_eq!(command, "/usr/local/bin/codebase-memory-mcp");
        assert_eq!(args[0], "/target");
        assert!(args.contains(&serde_json::json!(":memory:")));
        assert!(server.get("env").is_none());
    }

    #[test]
    fn persistent_builds_settings_with_cache_env() {
        let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Persistent).unwrap();
        let settings = mcp.build_settings().unwrap();

        let server = settings
            .get("mcpServers")
            .unwrap()
            .get("codebase-memory")
            .unwrap();

        let env = server.get("env").unwrap();
        let cache_dir = env.get("MCP_CACHE_DIR").unwrap().as_str().unwrap();
        assert!(cache_dir.contains("/target/.mcp_cache"));
    }

    #[test]
    fn rejects_relative_scan_root() {
        let result = CodebaseMemoryMcp::new("relative/path", CacheMode::Ephemeral);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_traversal_in_binary_path() {
        let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Ephemeral)
            .unwrap()
            .with_binary("../../../etc/evil");

        let result = mcp.build_settings();
        assert!(result.is_err());
    }

    #[test]
    fn custom_binary_path_works() {
        let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Ephemeral)
            .unwrap()
            .with_binary("/opt/custom/mcp-server");

        let settings = mcp.build_settings().unwrap();
        let command = settings
            .get("mcpServers")
            .unwrap()
            .get("codebase-memory")
            .unwrap()
            .get("command")
            .unwrap()
            .as_str()
            .unwrap();

        assert_eq!(command, "/opt/custom/mcp-server");
    }

    #[test]
    fn write_settings_creates_file() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().join("mcp_settings.json");

        let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Ephemeral).unwrap();
        mcp.write_settings(&output).unwrap();

        assert!(output.exists());
        let content = std::fs::read_to_string(&output).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.get("mcpServers").is_some());
    }

    #[test]
    fn ensure_cache_dir_creates_directory() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join("deep").join("nested").join("cache");

        let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Persistent)
            .unwrap()
            .with_cache_path(&cache_dir);

        mcp.ensure_cache_dir().unwrap();
        assert!(cache_dir.exists());
    }

    #[test]
    fn default_cache_path_is_correct() {
        let path = default_cache_path(std::path::Path::new("/workspace"), "acme", "webapp");
        assert_eq!(path, PathBuf::from("/workspace/acme/webapp/.mcp_cache"));
    }

    #[test]
    fn ephemeral_has_no_cache_path_by_default() {
        let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Ephemeral).unwrap();
        assert!(mcp.cache_path().is_none());
    }

    #[test]
    fn scan_root_can_be_subdirectory_of_target() {
        let mcp = CodebaseMemoryMcp::new("/target/src", CacheMode::Ephemeral).unwrap();
        let settings = mcp.build_settings().unwrap();
        let args = settings
            .get("mcpServers")
            .unwrap()
            .get("codebase-memory")
            .unwrap()
            .get("args")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(args[0], "/target/src");
    }
}
