# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Phase 1 — Connect the Stubs

### Added
- **OpenCode API client** (`src/bridge/opencode_client.rs`) — HTTP client for
  session injection, log event streaming, result polling, and completion waiting.
- **Firecracker REST client** (`src/runtime/firecracker.rs`) — Raw HTTP/1.1 over
  UNIX domain socket for machine config, boot source, drives, and VM lifecycle.
- **Event broadcast system** (`src/server/events.rs`) — `tokio::sync::broadcast`
  per-session channels with `dashmap`-backed event bus. WebSocket clients
  subscribe in real-time.

### Changed
- WebSocket handler (`src/server/ws.rs`) now consumes real broadcast events
  with lag detection and graceful disconnect.
- `OpenCodeBridge` integrates the runtime abstraction end-to-end.

### Phase 2 — Production Hardening

### Added
- **API key authentication** (`src/server/auth.rs`) — SHA-256 hashed keys per
  tenant, `Authorization: Bearer` middleware, key creation/revocation endpoints.
- **Persistent job queue** (`src/state/queue.rs`) — PostgreSQL `FOR UPDATE SKIP
  LOCKED` for atomic claiming, exponential backoff retries, dead-letter queue,
  `LISTEN/NOTIFY` trigger on insert.
- **Resource quotas** (`src/state/quotas.rs`) — Per-tenant limits on concurrent
  sessions, timeout, context tokens, and MicroVM sizing.
- **Git credential callbacks** (`src/checkout/credentials.rs`) — Ephemeral HTTPS
  token and SSH key injection into `git2` RemoteCallbacks.
- **Prometheus metrics** (`src/server/metrics_recorder.rs`) — Sessions, tokens,
  jobs, MicroVM provisioning, lock acquisition. `/metrics` endpoint.
- **Graceful shutdown** — SIGTERM/SIGINT handling with `axum::serve(
  ).with_graceful_shutdown()`.
- Database migration for `api_keys`, `job_queue`, `tenant_quotas` tables.

### Changed
- `POST /sessions` now enqueues a job instead of spawning a task directly.
- Session creation checks tenant quota before accepting.

### Phase 3 — Scale

### Added
- **Distributed worker pool** (`src/worker/`) — Workers claim jobs from the
  shared queue; horizontal scaling across nodes without double-processing.
- **Smart MCP pre-indexing** (`src/mcp/preindex.rs`) — Repo analysis (file count,
  HEAD SHA), automatic ephemeral/persistent cache selection, commit-SHA-based
  cache invalidation, `meta.json` tracking.
- **E2B SDK foundation** — `MicroVmBackend` supports E2B provider configuration
  alongside Firecracker.

### Tests
- **84 unit tests** (up from 6) covering all new modules.
