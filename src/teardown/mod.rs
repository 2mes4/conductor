//! Layer 4 — Teardown & Persistence.
//!
//! When the OpenCode session emits its completion event, Rust collects the
//! work and cleans up:
//!
//! - [`compact`] — Extract session JSON, count tokens with `tiktoken-rs`,
//!   prune the densest `tool_result` entries when over the limit.
//! - [`persist`] — Commit + push via `git2`, store the compacted JSON,
//!   release the advisory lock.

pub mod compact;
pub mod persist;
