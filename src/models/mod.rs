//! Domain models: Tenant → Project → Session hierarchy.

pub mod session;
pub mod tenant;

pub use session::{Session, SessionStatus};
pub use tenant::{Project, Tenant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A request to run an agent against a specific project branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub branch: String,
    /// The skill/tool repository to mount at `/skills`.
    pub skills_repo: String,
    /// The natural-language instruction for the agent.
    pub instruction: String,
    /// Optional resumed session ID for continuity.
    pub resume_from: Option<Uuid>,
}

/// The outcome of a completed agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub session_id: Uuid,
    pub status: SessionStatus,
    pub commit_sha: Option<String>,
    pub tokens_used: usize,
    pub completed_at: DateTime<Utc>,
    /// Compacted session history stored as JSON.
    pub history: serde_json::Value,
}
