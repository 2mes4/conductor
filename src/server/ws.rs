//! WebSocket handler for real-time agent log streaming.
//!
//! Clients connect to `ws://host:port/api/v1/sessions/:id/stream` to receive
//! a live feed of agent output.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use super::AppState;

/// WebSocket upgrade handler for session log streaming.
pub async fn stream_session(
    ws: WebSocketUpgrade,
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_stream(socket, session_id, state))
}

async fn handle_stream(mut socket: WebSocket, session_id: Uuid, _state: AppState) {
    tracing::info!(%session_id, "websocket client connected");

    // TODO: subscribe to the session's log broadcast channel and forward
    //       messages to the client until the session ends or disconnects.
    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Close(_) => break,
            Message::Ping(_) => {
                let _ = socket.send(Message::Pong(Vec::new())).await;
            }
            _ => {}
        }
    }

    tracing::info!(%session_id, "websocket client disconnected");
}
