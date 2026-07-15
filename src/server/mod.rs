//! HTTP + WebSocket server (axum).
//!
//! Exposes REST endpoints for task submission and a WebSocket for real-time
//! log streaming from the agent to a lightweight UI.

pub mod routes;
pub mod ws;

use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::error::Result;
use crate::state::{Database, LockManager};

/// Shared application state injected into all handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub locks: LockManager,
    pub config: Config,
}

/// Build and run the server.
pub async fn run(config: Config) -> Result<()> {
    let db = Database::connect(&config.database_url).await?;
    let locks = LockManager::new(db.pool().clone());

    let state = AppState {
        db,
        locks,
        config: config.clone(),
    };

    let app = Router::new()
        .nest("/api/v1", routes::build())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr())
        .await
        .map_err(|e| crate::error::ConductorError::Other(format!("bind failed: {e}")))?;

    tracing::info!("listening on {}", config.bind_addr());
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::ConductorError::Other(format!("server error: {e}")))?;

    Ok(())
}
