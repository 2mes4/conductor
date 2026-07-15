//! Layer 1 — State & Concurrency.
//!
//! Establishes the single source of truth so the system can scale without
//! losing control. Provides:
//!
//! - [`db`] — PostgreSQL connection pool and queries (sqlx)
//! - [`locks`] — Distributed advisory locks (`pg_advisory_lock`) to prevent
//!   concurrent mutations on the same branch

pub mod db;
pub mod locks;

pub use db::Database;
pub use locks::LockManager;
