//! REST API routes.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ws;
use crate::error::ConductorError;
use crate::models::{AgentTask, Session, SessionStatus};
use crate::server::auth;
use crate::server::AppState;

/// Build the v1 API router.
pub fn build() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/sessions", post(create_session))
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id/status", get(get_session_status))
        .route("/sessions/:id/stream", get(ws::stream_session))
        // Auth-protected admin routes
        .route("/api-keys", post(create_api_key_route))
}

pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

// ─── Session endpoints ───────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub tenant_slug: String,
    pub project_id: Uuid,
    pub branch: String,
    pub skills_repo: String,
    pub instruction: String,
    pub resume_from: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session_id: Uuid,
    pub status: &'static str,
}

async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, ApiError> {
    let tenant = state
        .db
        .get_or_create_tenant_by_slug(&req.tenant_slug)
        .await
        .map_err(ApiError::from)?;

    // Check quota
    let _quota = crate::state::quotas::check_and_reserve(&state, tenant.id)
        .await
        .map_err(ApiError::from)?;

    let session = Session {
        id: Uuid::new_v4(),
        tenant_id: tenant.id,
        project_id: req.project_id,
        branch: req.branch.clone(),
        status: SessionStatus::Queued,
        instruction: req.instruction.clone(),
        history: serde_json::json!({"messages": []}),
        commit_sha: None,
        tokens_used: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    state
        .db
        .create_session(&session)
        .await
        .map_err(ApiError::from)?;

    // Create event channel for real-time streaming.
    state.events.create_session(session.id);

    // Enqueue the job.
    let task = AgentTask {
        tenant_id: tenant.id,
        project_id: req.project_id,
        branch: req.branch.clone(),
        skills_repo: req.skills_repo,
        instruction: req.instruction,
        resume_from: req.resume_from,
    };

    let job_req = crate::state::queue::EnqueueRequest {
        session_id: session.id,
        tenant_id: tenant.id,
        project_id: req.project_id,
        branch: req.branch,
        payload: serde_json::to_value(&task).unwrap_or_default(),
        priority: None,
    };

    let _job_id = crate::state::queue::enqueue(&state, job_req)
        .await
        .map_err(ApiError::from)?;

    crate::server::metrics_recorder::session_started(&tenant.slug);

    Ok(Json(CreateSessionResponse {
        session_id: session.id,
        status: "queued",
    }))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Session>, ApiError> {
    let session = state.db.get_session(id).await.map_err(ApiError::from)?;
    Ok(Json(session))
}

async fn get_session_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state.db.get_session(id).await.map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "id": session.id,
        "status": session.status,
        "commit_sha": session.commit_sha,
        "tokens_used": session.tokens_used,
    })))
}

// ─── API Key management ──────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub tenant_slug: String,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub api_key: String,
}

async fn create_api_key_route(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<CreateApiKeyResponse>, ApiError> {
    let tenant = state
        .db
        .get_or_create_tenant_by_slug(&req.tenant_slug)
        .await
        .map_err(ApiError::from)?;

    let raw_key = auth::create_api_key(&state, tenant.id, &req.label)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(CreateApiKeyResponse { api_key: raw_key }))
}

// ─── Error type ──────────────────────────────────────────────

#[derive(Debug)]
pub struct ApiError(pub ConductorError);

impl From<ConductorError> for ApiError {
    fn from(e: ConductorError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self.0 {
            ConductorError::LockBusy(_) => (StatusCode::CONFLICT, self.0.to_string()),
            ConductorError::Database(sqlx::Error::RowNotFound) => {
                (StatusCode::NOT_FOUND, self.0.to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()),
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}
