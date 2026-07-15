//! Distributed advisory locks via PostgreSQL `pg_advisory_lock`.
//!
//! This prevents two agents from modifying the same branch of the same project
//! simultaneously, avoiding Git corruption.
//!
//! The lock key is derived deterministically from `(project_id, branch)` so
//! that concurrent work on *different* branches is never blocked.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ConductorError, Result};

/// A held advisory lock. Dropping this value does **not** release the lock;
//! call [`release`](Self::release) explicitly.
pub struct AdvisoryLock {
    pool: PgPool,
    key: i64,
}

impl AdvisoryLock {
    /// Release the advisory lock back to PostgreSQL.
    pub async fn release(self) -> Result<()> {
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(self.key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Manages distributed advisory locks.
#[derive(Clone)]
pub struct LockManager {
    pool: PgPool,
}

impl LockManager {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Derive a deterministic 64-bit lock key from a `(project, branch)` pair.
    ///
    /// Uses the high 32 bits from the project UUID and a CRC-like fold of the
    /// branch name into the low 32 bits. This keeps different project/branch
    /// combinations in distinct lock spaces.
    fn derive_key(project_id: Uuid, branch: &str) -> i64 {
        let uuid_hi = (project_id.as_u128() >> 64) as u32;
        let branch_hash = branch.bytes().fold(0u32, |acc, b| {
            acc.wrapping_mul(31).wrapping_add(b as u32)
        });
        let combined = ((uuid_hi as u64) << 32) | (branch_hash as u64);
        // Map u64 to i64 range safely (pg_advisory_lock takes a bigint).
        combined as i64
    }

    /// Attempt to acquire an advisory lock.
    ///
    /// Uses `pg_try_advisory_lock` — returns immediately rather than blocking.
    /// If the lock cannot be acquired, returns [`ConductorError::LockBusy`].
    pub async fn try_acquire(&self, project_id: Uuid, branch: &str) -> Result<AdvisoryLock> {
        let key = Self::derive_key(project_id, branch);

        let acquired: (bool,) =
            sqlx::query_as("SELECT pg_try_advisory_lock($1)")
                .bind(key)
                .fetch_one(&self.pool)
                .await?;

        if acquired.0 {
            tracing::debug!(key, "advisory lock acquired");
            Ok(AdvisoryLock {
                pool: self.pool.clone(),
                key,
            })
        } else {
            Err(ConductorError::LockBusy(format!(
                "{project_id}/{branch}"
            )))
        }
    }

    /// Blocking acquire — waits until the lock is available.
    pub async fn acquire(&self, project_id: Uuid, branch: &str) -> Result<AdvisoryLock> {
        let key = Self::derive_key(project_id, branch);

        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(key)
            .execute(&self.pool)
            .await?;

        tracing::debug!(key, "advisory lock acquired (blocking)");
        Ok(AdvisoryLock {
            pool: self.pool.clone(),
            key,
        })
    }
}
