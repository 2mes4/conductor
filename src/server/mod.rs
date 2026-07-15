//! HTTP + WebSocket server (axum).
//!
//! Exposes REST endpoints for task submission, authentication via API keys,
//! real-time log streaming via WebSocket, and Prometheus metrics.

pub mod auth;
pub mod events;
pub mod metrics_recorder;
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
    pub events: events::EventBus,
}

/// Build and run the server with graceful shutdown.
pub async fn run(config: Config) -> Result<()> {
    metrics_recorder::init();

    let db = Database::connect(&config.database_url).await?;
    let locks = LockManager::new(db.pool().clone());
    let events = events::EventBus::new();

    let state = AppState {
        db,
        locks,
        config: config.clone(),
        events,
    };

    let app = Router::new()
        .route("/health", axum::routing::get(routes::health))
        .route("/metrics", axum::routing::get(metrics_handler))
        .nest("/api/v1", routes::build())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr())
        .await
        .map_err(|e| crate::error::ConductorError::Other(format!("bind failed: {e}")))?;

    tracing::info!("listening on {}", config.bind_addr());

    // Graceful shutdown: wait for SIGTERM/SIGINT, then let active sessions drain.
    let shutdown = async {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }

        tracing::info!("shutdown signal received, draining active sessions...");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| crate::error::ConductorError::Other(format!("server error: {e}")))?;

    tracing::info!("server shut down gracefully");
    Ok(())
}

/// Prometheus metrics endpoint.
async fn metrics_handler() -> impl axum::response::IntoResponse {
    let body = metrics_recorder::render();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}
