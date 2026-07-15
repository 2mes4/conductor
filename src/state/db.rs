//! Database connection pool and data-access queries.

use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

use crate::error::{ConductorError, Result};
use crate::models::{Project, Session, SessionStatus, Tenant};

/// Thin wrapper around the PostgreSQL connection pool.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Create a connection pool and verify connectivity.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .acquire_timeout(Duration::from_secs(10))
            .connect(url)
            .await?;

        sqlx::migrate!("../migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // ── Tenant queries ──────────────────────────────────────

    pub async fn get_tenant(&self, id: Uuid) -> Result<Tenant> {
        let tenant = sqlx::query_as::<_, Tenant>("SELECT * FROM tenants WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(tenant)
    }

    // ── Project queries ─────────────────────────────────────

    pub async fn get_project(&self, id: Uuid) -> Result<Project> {
        let project = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(project)
    }

    // ── Session queries ─────────────────────────────────────

    pub async fn create_session(&self, session: &Session) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions
                (id, tenant_id, project_id, branch, status, instruction, history, commit_sha, tokens_used)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(session.id)
        .bind(session.tenant_id)
        .bind(session.project_id)
        .bind(&session.branch)
        .bind(session.status)
        .bind(&session.instruction)
        .bind(&session.history)
        .bind(session.commit_sha)
        .bind(session.tokens_used)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, id: Uuid) -> Result<Session> {
        let session = sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(session)
    }

    pub async fn update_session_status(
        &self,
        id: Uuid,
        status: SessionStatus,
    ) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = $2, updated_at = NOW() WHERE id = $1")
            .bind(id)
            .bind(status)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Persist the compacted history, token count, and optional commit SHA.
    pub async fn finalize_session(
        &self,
        id: Uuid,
        status: SessionStatus,
        history: &serde_json::Value,
        tokens_used: i64,
        commit_sha: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions
               SET status = $2, history = $3, tokens_used = $4,
                   commit_sha = $5, updated_at = NOW()
             WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(history)
        .bind(tokens_used)
        .bind(commit_sha)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Retrieve the most recent session history for resumption.
    pub async fn get_latest_history(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        branch: &str,
    ) -> Result<Option<serde_json::Value>> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT history FROM sessions
             WHERE tenant_id = $1 AND project_id = $2 AND branch = $3
               AND status IN ('completed')
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(tenant_id)
        .bind(project_id)
        .bind(branch)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(h,)| h))
    }

    pub async fn get_or_create_tenant_by_slug(&self, slug: &str) -> Result<Tenant> {
        let tenant =
            sqlx::query_as::<_, Tenant>("SELECT * FROM tenants WHERE slug = $1")
                .bind(slug)
                .fetch_optional(&self.pool)
                .await?;

        match tenant {
            Some(t) => Ok(t),
            None => Err(ConductorError::Other(format!(
                "tenant with slug '{slug}' not found"
            ))),
        }
    }
}
