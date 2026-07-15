//! Per-tenant resource quotas.
//!
//! Enforces limits on concurrent sessions, session timeout, context tokens,
//! and MicroVM resource allocation per tenant.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ConductorError, Result};
use crate::server::AppState;

/// Quota configuration for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TenantQuota {
    pub tenant_id: Uuid,
    pub max_concurrent_sessions: i32,
    pub max_session_timeout_secs: i32,
    pub max_context_tokens: i32,
    pub microvm_memory_mb: i32,
    pub microvm_vcpus: i32,
}

impl Default for TenantQuota {
    fn default() -> Self {
        Self {
            tenant_id: Uuid::nil(),
            max_concurrent_sessions: 5,
            max_session_timeout_secs: 3600,
            max_context_tokens: 80000,
            microvm_memory_mb: 512,
            microvm_vcpus: 1,
        }
    }
}

/// Load the quota for a tenant from the database.
pub async fn get_quota(state: &AppState, tenant_id: Uuid) -> Result<TenantQuota> {
    let quota =
        sqlx::query_as::<_, TenantQuota>("SELECT * FROM tenant_quotas WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(state.db.pool())
            .await?;
    Ok(quota)
}

/// Count active (non-terminal) sessions for a tenant.
pub async fn count_active_sessions(state: &AppState, tenant_id: Uuid) -> Result<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sessions
         WHERE tenant_id = $1
           AND status IN ('queued', 'preparing', 'running', 'tearingdown')",
    )
    .bind(tenant_id)
    .fetch_one(state.db.pool())
    .await?;
    Ok(count)
}

/// Check whether a tenant can start a new session.
///
/// Returns the quota if allowed, or an error if the limit is exceeded.
pub async fn check_and_reserve(state: &AppState, tenant_id: Uuid) -> Result<TenantQuota> {
    let quota = get_quota(state, tenant_id).await?;
    let active = count_active_sessions(state, tenant_id).await?;

    if active >= quota.max_concurrent_sessions as i64 {
        return Err(ConductorError::Other(format!(
            "tenant {tenant_id} has {active} active sessions (limit: {})",
            quota.max_concurrent_sessions
        )));
    }

    tracing::debug!(
        tenant_id = %tenant_id,
        active,
        limit = quota.max_concurrent_sessions,
        "quota check passed"
    );

    Ok(quota)
}

/// Update a tenant's quota.
pub async fn update_quota(state: &AppState, quota: &TenantQuota) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE tenant_quotas
           SET max_concurrent_sessions = $2,
               max_session_timeout_secs = $3,
               max_context_tokens = $4,
               microvm_memory_mb = $5,
               microvm_vcpus = $6,
               updated_at = NOW()
         WHERE tenant_id = $1
        "#,
    )
    .bind(quota.tenant_id)
    .bind(quota.max_concurrent_sessions)
    .bind(quota.max_session_timeout_secs)
    .bind(quota.max_context_tokens)
    .bind(quota.microvm_memory_mb)
    .bind(quota.microvm_vcpus)
    .execute(state.db.pool())
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_quota_has_sensible_limits() {
        let q = TenantQuota::default();
        assert_eq!(q.max_concurrent_sessions, 5);
        assert_eq!(q.max_session_timeout_secs, 3600);
        assert_eq!(q.max_context_tokens, 80000);
        assert_eq!(q.microvm_memory_mb, 512);
        assert_eq!(q.microvm_vcpus, 1);
    }

    #[test]
    fn quota_serializes_to_json() {
        let q = TenantQuota::default();
        let json = serde_json::to_string(&q).unwrap();
        assert!(json.contains("\"max_concurrent_sessions\":5"));
    }
}
