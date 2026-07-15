//! Agent session model and lifecycle status.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lifecycle of an agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "session_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Queued waiting for a distributed lock.
    Queued,
    /// Lock acquired, workspace being prepared.
    Preparing,
    /// OpenCode process running.
    Running,
    /// Teardown in progress (compaction, commit, cleanup).
    TearingDown,
    /// Finished successfully.
    Completed,
    /// Failed or timed out.
    Failed,
}

/// A single agent execution session.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub branch: String,
    pub status: SessionStatus,
    pub instruction: String,
    /// Compacted JSON history (injected on resume, stored on teardown).
    pub history: serde_json::Value,
    pub commit_sha: Option<String>,
    pub tokens_used: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
