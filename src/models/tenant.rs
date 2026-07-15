//! Tenant and Project models — the multi-tenant hierarchy.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A tenant (organisation / customer). Provides multi-tenant isolation.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
}

/// A project belonging to a tenant. Maps to a Git repository.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    /// The clone URL of the target repository.
    pub repo_url: String,
    /// Default branch (e.g. `main`).
    pub default_branch: String,
    pub created_at: DateTime<Utc>,
}
