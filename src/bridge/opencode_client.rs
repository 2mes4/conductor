//! OpenCode Server API client — HTTP client for injecting payloads and
//! streaming agent logs.
//!
//! This client communicates with the OpenCode Server HTTP API running inside
//! the execution environment (local process or MicroVM). It handles:
//!
//! 1. Starting a session (POST the injection payload)
//! 2. Streaming log events via Server-Sent Events (SSE)
//! 3. Polling for session completion and retrieving the final result

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bridge::InjectionPayload;
use crate::error::{ConductorError, Result};

/// HTTP client for the OpenCode Server API.
#[derive(Clone)]
pub struct OpenCodeClient {
    base_url: String,
    http: Client,
}

/// Response from starting a session.
#[derive(Debug, Deserialize)]
pub struct SessionStartResponse {
    pub session_id: Uuid,
    pub status: String,
}

/// A single log event streamed from OpenCode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogEvent {
    Log {
        level: String,
        message: String,
    },
    ToolCall {
        tool: String,
        #[serde(default)]
        args: serde_json::Value,
    },
    ToolResult {
        tool: String,
        #[serde(default)]
        result: serde_json::Value,
    },
    Status {
        status: String,
    },
    Done {
        #[serde(default)]
        commit_sha: Option<String>,
    },
    Error {
        message: String,
    },
}

/// Final session result retrieved from OpenCode.
#[derive(Debug, Deserialize)]
pub struct SessionResult {
    pub session_id: Uuid,
    pub history: serde_json::Value,
    pub tokens_used: u64,
    pub success: bool,
    #[serde(default)]
    pub commit_sha: Option<String>,
}

impl OpenCodeClient {
    /// Create a new client pointing at the given base URL.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| ConductorError::OpenCode(format!("HTTP client build failed: {e}")))?;

        Ok(Self {
            base_url: base_url.into(),
            http,
        })
    }

    /// Start an agent session by POSTing the injection payload.
    pub async fn start_session(&self, payload: &InjectionPayload) -> Result<SessionStartResponse> {
        let url = format!("{}/sessions", self.base_url);
        tracing::debug!(url = %url, session_id = %payload.session_id, "POST start session");

        let resp = self
            .http
            .post(&url)
            .json(payload)
            .send()
            .await
            .map_err(|e| ConductorError::OpenCode(format!("start_session request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ConductorError::OpenCode(format!(
                "start_session returned {status}: {body}"
            )));
        }

        resp.json::<SessionStartResponse>()
            .await
            .map_err(|e| ConductorError::OpenCode(format!("failed to parse response: {e}")))
    }

    /// Retrieve the final result of a completed session.
    pub async fn get_result(&self, session_id: Uuid) -> Result<SessionResult> {
        let url = format!("{}/sessions/{}/result", self.base_url, session_id);
        tracing::debug!(url = %url, "GET session result");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ConductorError::OpenCode(format!("get_result request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ConductorError::OpenCode(format!(
                "get_result returned {status}: {body}"
            )));
        }

        resp.json::<SessionResult>()
            .await
            .map_err(|e| ConductorError::OpenCode(format!("failed to parse result: {e}")))
    }

    /// Poll for session completion with a configurable interval.
    ///
    /// Returns the final result once the session is done.
    pub async fn wait_for_completion(
        &self,
        session_id: Uuid,
        poll_interval: Duration,
        timeout: Duration,
    ) -> Result<SessionResult> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(ConductorError::SessionTimeout(timeout.as_secs()));
            }

            match self.get_result(session_id).await {
                Ok(result) if result.success => {
                    tracing::info!(session_id = %session_id, "session completed");
                    return Ok(result);
                }
                Ok(result) => {
                    tracing::info!(session_id = %session_id, "session finished (not success)");
                    return Ok(result);
                }
                Err(ConductorError::OpenCode(msg)) if msg.contains("404") => {
                    // Session not ready yet — keep polling.
                }
                Err(e) => {
                    tracing::warn!(session_id = %session_id, error = %e, "poll error, retrying");
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Check if the OpenCode server is reachable.
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/health", self.base_url);
        let resp = self.http.get(&url).send().await;
        Ok(resp.map(|r| r.status().is_success()).unwrap_or(false))
    }

    /// Get the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation_works() {
        let client = OpenCodeClient::new("http://localhost:9999");
        assert!(client.is_ok());
        assert_eq!(client.unwrap().base_url(), "http://localhost:9999");
    }

    #[test]
    fn log_event_deserializes_log() {
        let json = r#"{"type":"log","level":"info","message":"Starting work"}"#;
        let event: LogEvent = serde_json::from_str(json).unwrap();
        match event {
            LogEvent::Log { level, message } => {
                assert_eq!(level, "info");
                assert_eq!(message, "Starting work");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn log_event_deserializes_done() {
        let json = r#"{"type":"done","commit_sha":"abc123"}"#;
        let event: LogEvent = serde_json::from_str(json).unwrap();
        match event {
            LogEvent::Done { commit_sha } => {
                assert_eq!(commit_sha, Some("abc123".into()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn log_event_deserializes_done_without_sha() {
        let json = r#"{"type":"done"}"#;
        let event: LogEvent = serde_json::from_str(json).unwrap();
        match event {
            LogEvent::Done { commit_sha } => assert!(commit_sha.is_none()),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn log_event_deserializes_tool_call() {
        let json = r#"{"type":"tool_call","tool":"lint","args":{"path":"src/"}}"#;
        let event: LogEvent = serde_json::from_str(json).unwrap();
        match event {
            LogEvent::ToolCall { tool, args } => {
                assert_eq!(tool, "lint");
                assert_eq!(args["path"], "src/");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn log_event_deserializes_error() {
        let json = r#"{"type":"error","message":"Something went wrong"}"#;
        let event: LogEvent = serde_json::from_str(json).unwrap();
        match event {
            LogEvent::Error { message } => assert_eq!(message, "Something went wrong"),
            _ => panic!("wrong variant"),
        }
    }
}
