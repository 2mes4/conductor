//! API key authentication middleware.
//!
//! Extracts the API key from the `Authorization: Bearer <key>` header,
//! hashes it with SHA-256, and looks up the tenant in PostgreSQL.
//! The authenticated tenant is injected into request extensions for
//! downstream handlers.

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ConductorError;
use crate::server::AppState;

/// The authenticated tenant context, available in request extensions.
#[derive(Debug, Clone)]
pub struct AuthTenant {
    pub tenant_id: Uuid,
    pub tenant_slug: String,
}

/// Hash an API key for secure storage and lookup.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Generate a new random API key and its hash.
pub fn generate_api_key() -> (String, String) {
    let raw = uuid::Uuid::new_v4().to_string();
    let hash = hash_api_key(&raw);
    (raw, hash)
}

/// Authentication middleware — validates the API key and injects `AuthTenant`.
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => return Err(StatusCode::UNAUTHORIZED),
    };

    let key_hash = hash_api_key(token);

    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT t.id, t.slug FROM api_keys ak
         JOIN tenants t ON ak.tenant_id = t.id
         WHERE ak.key_hash = $1 AND ak.revoked_at IS NULL",
    )
    .bind(&key_hash)
    .fetch_optional(state.db.pool())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (tenant_id, tenant_slug) = match row {
        Some((id, slug)) => (id, slug),
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    req.extensions_mut().insert(AuthTenant {
        tenant_id,
        tenant_slug,
    });

    Ok(next.run(req).await)
}

/// Create a new API key for a tenant. Returns the raw key (shown once).
pub async fn create_api_key(
    state: &AppState,
    tenant_id: Uuid,
    label: &str,
) -> Result<String, ConductorError> {
    let (raw, hash) = generate_api_key();

    sqlx::query("INSERT INTO api_keys (tenant_id, key_hash, label) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind(&hash)
        .bind(label)
        .execute(state.db.pool())
        .await?;

    tracing::info!(tenant_id = %tenant_id, label, "API key created");
    Ok(raw)
}

/// Revoke an API key by its hash.
pub async fn revoke_api_key(state: &AppState, key_hash: &str) -> Result<(), ConductorError> {
    sqlx::query("UPDATE api_keys SET revoked_at = NOW() WHERE key_hash = $1")
        .bind(key_hash)
        .execute(state.db.pool())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let h1 = hash_api_key("test-key-123");
        let h2 = hash_api_key("test-key-123");
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_is_hex_string() {
        let hash = hash_api_key("test");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn different_keys_different_hashes() {
        let h1 = hash_api_key("key-one");
        let h2 = hash_api_key("key-two");
        assert_ne!(h1, h2);
    }

    #[test]
    fn generate_api_key_produces_valid_pair() {
        let (raw, hash) = generate_api_key();
        assert!(!raw.is_empty());
        assert_eq!(hash, hash_api_key(&raw));
    }
}
