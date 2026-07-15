//! Conductor — Rust orchestration control plane for remote AI agents.
//!
//! This crate implements a [Backend For Frontend (BFF)] that governs the
//! lifecycle of AI agent sessions. It solves five critical structural problems:
//!
//! | Problem | Solution |
//! |---------|----------|
//! | Concurrency collisions | Distributed locks via PostgreSQL `pg_advisory_lock` |
//! | Data leakage (multi-tenant) | Strict ephemeral sandboxing |
//! | Context overflow (LLM) | External compaction with `tiktoken-rs` |
//! | Tool coupling | Dual-Checkout: `/target` (code) + `/skills` (tools) |
//! | Zombie executions | Managed teardown with hard timeout & rollback |
//!
//! The four architectural layers:
//!
//! 1. [`state`] — State & Concurrency (PostgreSQL, advisory locks)
//! 2. [`checkout`] — Dual-Checkout Engine (native Git via `git2`)
//! 3. [`bridge`] — Contract Validation & Injection (OpenCode bridge)
//! 4. [`teardown`] — Teardown & Persistence (compaction, commit, cleanup)

pub mod bridge;
pub mod checkout;
pub mod config;
pub mod error;
pub mod mcp;
pub mod models;
pub mod orchestrator;
pub mod runtime;
pub mod server;
pub mod state;
pub mod teardown;

pub use config::Config;
pub use error::{ConductorError, Result};
