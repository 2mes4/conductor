//! Unified error types for Conductor.

use thiserror::Error;

/// Concrete error type used throughout the crate.
#[derive(Debug, Error)]
pub enum ConductorError {
    #[error("missing required environment variable: {0}")]
    MissingEnv(&'static str),

    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("git operation failed: {0}")]
    Git(#[from] git2::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("concurrency lock could not be acquired for {0}")]
    LockBusy(String),

    #[error("session timed out after {0}s")]
    SessionTimeout(u64),

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("path traversal detected: {0}")]
    PathTraversal(String),

    #[error("OpenCode process error: {0}")]
    OpenCode(String),

    #[error("{0}")]
    Other(String),
}

impl ConductorError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

/// Convenience `Result` alias.
pub type Result<T> = std::result::Result<T, ConductorError>;
