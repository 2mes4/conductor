//! Session event bus — broadcast system for real-time log streaming.
//!
//! Each agent session gets a `tokio::sync::broadcast` channel. The orchestrator
//! publishes events (logs, status changes, tool calls) and WebSocket clients
//! subscribe to receive them in real-time.

use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bridge::opencode_client::LogEvent;

/// Default capacity for broadcast channels. Subscribers slower than this will
/// miss events (by design — we don't want slow clients to block the agent).
const CHANNEL_CAPACITY: usize = 256;

/// A serializable event that can be sent over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub session_id: Uuid,
    pub event: LogEvent,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Central event bus managing per-session broadcast channels.
#[derive(Clone)]
pub struct EventBus {
    channels: Arc<DashMap<Uuid, broadcast::Sender<SessionEvent>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
        }
    }

    /// Create a channel for a new session and return the sender.
    pub fn create_session(&self, session_id: Uuid) -> broadcast::Sender<SessionEvent> {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        self.channels.insert(session_id, tx.clone());
        tracing::debug!(%session_id, "event channel created");
        tx
    }

    /// Subscribe to a session's events. Returns `None` if the session doesn't exist.
    pub fn subscribe(&self, session_id: Uuid) -> Option<broadcast::Receiver<SessionEvent>> {
        self.channels.get(&session_id).map(|tx| tx.subscribe())
    }

    /// Publish an event to a session's channel.
    /// Returns `false` if no one is listening (which is fine — events are lost).
    pub fn publish(&self, session_id: Uuid, event: LogEvent) -> bool {
        let session_event = SessionEvent {
            session_id,
            event,
            timestamp: chrono::Utc::now(),
        };

        if let Some(tx) = self.channels.get(&session_id) {
            match tx.send(session_event) {
                Ok(n) => {
                    tracing::trace!(%session_id, delivered = n, "event published");
                    true
                }
                Err(_) => {
                    // No receivers — this is expected when no WS client is connected.
                    false
                }
            }
        } else {
            tracing::warn!(%session_id, "publish to unknown session");
            false
        }
    }

    /// Remove a session's channel (called on teardown).
    pub fn remove_session(&self, session_id: Uuid) {
        if self.channels.remove(&session_id).is_some() {
            tracing::debug!(%session_id, "event channel removed");
        }
    }

    /// Check if a session has an active channel.
    pub fn has_session(&self, session_id: Uuid) -> bool {
        self.channels.contains_key(&session_id)
    }

    /// Get the number of active sessions.
    pub fn active_count(&self) -> usize {
        self.channels.len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_publish() {
        let bus = EventBus::new();
        let session_id = Uuid::new_v4();

        let _tx = bus.create_session(session_id);
        assert!(bus.has_session(session_id));

        let published = bus.publish(
            session_id,
            LogEvent::Log {
                level: "info".into(),
                message: "hello".into(),
            },
        );
        // No subscriber, so publish returns false.
        assert!(!published);
    }

    #[tokio::test]
    async fn subscribe_receives_events() {
        let bus = EventBus::new();
        let session_id = Uuid::new_v4();

        let _tx = bus.create_session(session_id);
        let mut rx = bus.subscribe(session_id).unwrap();

        bus.publish(
            session_id,
            LogEvent::Log {
                level: "info".into(),
                message: "test message".into(),
            },
        );

        let event = rx.recv().await.unwrap();
        assert_eq!(event.session_id, session_id);
        match event.event {
            LogEvent::Log { message, .. } => assert_eq!(message, "test message"),
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn subscribe_returns_none_for_unknown_session() {
        let bus = EventBus::new();
        assert!(bus.subscribe(Uuid::new_v4()).is_none());
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_events() {
        let bus = EventBus::new();
        let session_id = Uuid::new_v4();

        let _tx = bus.create_session(session_id);
        let mut rx1 = bus.subscribe(session_id).unwrap();
        let mut rx2 = bus.subscribe(session_id).unwrap();

        bus.publish(
            session_id,
            LogEvent::Status {
                status: "running".into(),
            },
        );

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1.session_id, session_id);
        assert_eq!(e2.session_id, session_id);
    }

    #[test]
    fn remove_session_cleans_up() {
        let bus = EventBus::new();
        let session_id = Uuid::new_v4();

        bus.create_session(session_id);
        assert!(bus.has_session(session_id));

        bus.remove_session(session_id);
        assert!(!bus.has_session(session_id));
    }

    #[test]
    fn active_count_tracks_sessions() {
        let bus = EventBus::new();
        assert_eq!(bus.active_count(), 0);

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let _tx1 = bus.create_session(id1);
        let _tx2 = bus.create_session(id2);
        assert_eq!(bus.active_count(), 2);

        bus.remove_session(id1);
        assert_eq!(bus.active_count(), 1);
    }
}
