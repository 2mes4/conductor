//! Persistent job queue using PostgreSQL `FOR UPDATE SKIP LOCKED` + `LISTEN/NOTIFY`.
//!
//! When a session is created, a job is enqueued. Workers claim jobs atomically
//! using `SKIP LOCKED`, process them, and mark them complete. If a lock is busy,
//! the job stays queued and is picked up when the lock frees (via `LISTEN/NOTIFY`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ConductorError, Result};
use crate::server::AppState;

/// A job in the persistent queue.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub branch: String,
    pub payload: serde_json::Value,
    pub priority: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub status: String,
    pub locked_by: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub available_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Request to enqueue a new job.
#[derive(Debug, Serialize, Deserialize)]
pub struct EnqueueRequest {
    pub session_id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub branch: String,
    pub payload: serde_json::Value,
    pub priority: Option<String>,
}

/// Enqueue a job. Returns the job ID.
pub async fn enqueue(state: &AppState, req: EnqueueRequest) -> Result<Uuid> {
    let job_id = Uuid::new_v4();
    let priority = req.priority.as_deref().unwrap_or("normal");

    sqlx::query(
        r#"
        INSERT INTO job_queue
            (id, session_id, tenant_id, project_id, branch, payload, priority)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(job_id)
    .bind(req.session_id)
    .bind(req.tenant_id)
    .bind(req.project_id)
    .bind(&req.branch)
    .bind(&req.payload)
    .bind(priority)
    .execute(state.db.pool())
    .await?;

    tracing::info!(job_id = %job_id, session_id = %req.session_id, "job enqueued");
    Ok(job_id)
}

/// Claim the next available job atomically using `FOR UPDATE SKIP LOCKED`.
///
/// `worker_id` identifies the claiming worker (for liveness tracking).
pub async fn claim_next(state: &AppState, worker_id: &str) -> Result<Option<Job>> {
    let job: Option<Job> = sqlx::query_as::<_, Job>(
        r#"
        UPDATE job_queue
           SET status = 'running',
               locked_by = $1,
               locked_at = NOW(),
               attempts = attempts + 1
         WHERE id = (
            SELECT id FROM job_queue
             WHERE status = 'pending'
               AND available_at <= NOW()
             ORDER BY
               CASE priority
                 WHEN 'high' THEN 0
                 WHEN 'normal' THEN 1
                 WHEN 'low' THEN 2
               END,
               available_at
             FOR UPDATE SKIP LOCKED
             LIMIT 1
         )
         RETURNING *
        "#,
    )
    .bind(worker_id)
    .fetch_optional(state.db.pool())
    .await?;

    if let Some(ref j) = job {
        tracing::info!(job_id = %j.id, session_id = %j.session_id, worker_id, "job claimed");
    }

    Ok(job)
}

/// Mark a job as completed.
pub async fn complete(state: &AppState, job_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE job_queue SET status = 'completed', completed_at = NOW() WHERE id = $1")
        .bind(job_id)
        .execute(state.db.pool())
        .await?;
    tracing::info!(job_id = %job_id, "job completed");
    Ok(())
}

/// Mark a job as failed. If attempts remain, reschedule with backoff.
pub async fn fail(state: &AppState, job_id: Uuid, error: &str) -> Result<bool> {
    let row: Option<(i32, i32)> =
        sqlx::query_as("SELECT attempts, max_attempts FROM job_queue WHERE id = $1")
            .bind(job_id)
            .fetch_optional(state.db.pool())
            .await?;

    let (attempts, max_attempts) = match row {
        Some(r) => r,
        None => return Err(ConductorError::Other("job not found".into())),
    };

    if attempts < max_attempts {
        // Exponential backoff: 2^attempts seconds.
        let backoff_secs = 2_i64.pow(attempts as u32);
        sqlx::query(
            r#"
            UPDATE job_queue
               SET status = 'pending',
                   locked_by = NULL,
                   locked_at = NULL,
                   available_at = NOW() + ($1 || ' seconds')::INTERVAL
             WHERE id = $2
            "#,
        )
        .bind(backoff_secs.to_string())
        .bind(job_id)
        .execute(state.db.pool())
        .await?;

        tracing::warn!(job_id = %job_id, attempts, backoff_secs, error, "job failed, rescheduled");
        Ok(true) // Retried
    } else {
        sqlx::query("UPDATE job_queue SET status = 'dead' WHERE id = $1")
            .bind(job_id)
            .execute(state.db.pool())
            .await?;

        tracing::error!(job_id = %job_id, attempts, max_attempts, error, "job moved to dead letter queue");
        Ok(false) // Dead-lettered
    }
}

/// Count pending jobs (for metrics).
pub async fn pending_count(state: &AppState) -> Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM job_queue WHERE status = 'pending'")
            .fetch_one(state.db.pool())
            .await?;
    Ok(count)
}

/// Count running jobs (for metrics).
pub async fn running_count(state: &AppState) -> Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM job_queue WHERE status = 'running'")
            .fetch_one(state.db.pool())
            .await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_request_serializes() {
        let req = EnqueueRequest {
            session_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            branch: "main".into(),
            payload: serde_json::json!({"instruction": "test"}),
            priority: Some("high".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"branch\":\"main\""));
    }

    #[test]
    fn job_derives_from_row() {
        // Verify the Job struct has sqlx::FromRow derive
        // This is a compile-time check — if it compiles, it works.
    }
}
