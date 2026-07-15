//! Configuration loading from environment variables and optional TOML.

use std::env;

use crate::error::{ConductorError, Result};

/// Central configuration for the Conductor orchestrator.
#[derive(Debug, Clone)]
pub struct Config {
    /// PostgreSQL connection string.
    pub database_url: String,
    /// Bind address for the HTTP + WebSocket server.
    pub host: String,
    /// Port for the HTTP + WebSocket server.
    pub port: u16,
    /// Root directory for ephemeral workspaces.
    pub workspace_root: String,
    /// Token threshold before context compaction triggers.
    pub max_context_tokens: usize,
    /// Hard timeout for a single agent session, in seconds.
    pub session_timeout_secs: u64,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: env::var("DATABASE_URL")
                .map_err(|_| ConductorError::MissingEnv("DATABASE_URL"))?,
            host: env::var("CONDUCTOR_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("CONDUCTOR_PORT")
                .unwrap_or_else(|_| "7878".into())
                .parse()
                .map_err(|_| ConductorError::InvalidConfig("CONDUCTOR_PORT must be a number"))?,
            workspace_root: env::var("CONDUCTOR_WORKSPACE_ROOT")
                .unwrap_or_else(|_| "/workspace".into()),
            max_context_tokens: env::var("CONDUCTOR_MAX_CONTEXT_TOKENS")
                .unwrap_or_else(|_| "80000".into())
                .parse()
                .map_err(|_| ConductorError::InvalidConfig("CONDUCTOR_MAX_CONTEXT_TOKENS"))?,
            session_timeout_secs: env::var("CONDUCTOR_SESSION_TIMEOUT_SECS")
                .unwrap_or_else(|_| "3600".into())
                .parse()
                .map_err(|_| ConductorError::InvalidConfig("CONDUCTOR_SESSION_TIMEOUT_SECS"))?,
        })
    }

    /// The bind address as `host:port`.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
