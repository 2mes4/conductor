//! Layer 3 — Contract Validation & Injection.
//!
//! The frontier where data is transformed and security limits are imposed.
//!
//! - [`manifest`] — Deserialize `AgentStackManifest` with strict serde rules
//! - [`sanitize`] — Path sanitisation (reject directory traversal)
//! - [`opencode`] — OpenCode Server lifecycle: spawn, inject payload, control

pub mod manifest;
pub mod opencode;
pub mod opencode_client;
pub mod sanitize;

pub use manifest::{AgentStackManifest, ToolDefinition};
pub use opencode::{BridgeConfig, InjectionPayload, OpenCodeBridge};
pub use opencode_client::{LogEvent, OpenCodeClient};
