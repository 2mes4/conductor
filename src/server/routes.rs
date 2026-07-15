//! REST API routes.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::AppState;
use crate::error::ConductorError;
use crate::models::{AgentTask, Session, SessionStatus};

/// Build the v1 API router.
pub fn build() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/sessions", post(create_session))
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id/status", get(get_session_status))
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

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

    state.db.create_session(&session).await.map_err(ApiError::from)?;

    let task = AgentTask {
        tenant_id: tenant.id,
        project_id: req.project_id,
        branch: req.branch,
        skills_repo: req.skills_repo,
        instruction: req.instruction,
        resume_from: req.resume_from,
    };

    // Spawn the orchestration lifecycle as a background task.
    let app_state = state.clone();
    let session_id = session.id;
    tokio::spawn(async move {
        if let Err(e) = crate::orchestrator::run_session(app_state, session_id, task).await {
            tracing::error!(session_id = %session_id, error = %e, "session failed");
            let _ = app_state
                .db
                .update_session_status(session_id, SessionStatus::Failed)
                .await;
        }
    });

    Ok(Json(CreateSessionResponse {
        session_id: session.id,
        status: "queued",
    }))
}

async fn get_session(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<Session>, ApiError> {
    let session = state.db.get_session(id).await.map_err(ApiError::from)?;
    Ok(Json(session))
}

async fn get_session_status(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state.db.get_session(id).await.map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "id": session.id,
        "status": session.status,
        "commit_sha": session.commit_sha,
        "tokens_used": session.tokens_used,
    })))
}

/// Error type for API handlers.
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
