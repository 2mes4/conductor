//! WebSocket handler for real-time agent log streaming.
//!
//! Clients connect to `ws://host:port/api/v1/sessions/:id/stream` to receive
//! a live feed of agent output via the [`EventBus`].

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
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

async fn handle_stream(socket: WebSocket, session_id: Uuid, state: AppState) {
    tracing::info!(%session_id, "websocket client connected");

    let Some(rx) = state.events.subscribe(session_id) else {
        // Session not found — close immediately.
        let _ = socket.close().await;
        return;
    };

    let (mut sender, mut receiver) = socket.split();
    let mut broadcast_rx = rx;

    // Forward broadcast events to the WebSocket client.
    loop {
        tokio::select! {
            // Event from the agent → send to client
            event = broadcast_rx.recv() => {
                match event {
                    Ok(session_event) => {
                        let json = match serde_json::to_string(&session_event) {
                            Ok(j) => j,
                            Err(_) => continue,
                        };
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(%session_id, lagged = n, "client lagged behind");
                        let _ = sender.send(Message::Text(
                            serde_json::json!({"type": "warning", "message": format!("missed {n} events")})
                                .to_string()
                        )).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            // Client message (ping/close) → handle
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::info!(%session_id, "websocket client disconnected");
}
