# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **MicroVM runtime abstraction** (`runtime/`) — pluggable execution backends:
  - `MicroVmBackend` for Firecracker / E2B sandboxed execution
  - `LocalProcessBackend` for development without virtualisation
- **MCP server integration** (`mcp/`) — codebase-memory-mcp configuration:
  - Dynamic `mcp_settings.json` generation at runtime
  - Ephemeral (RAM-first) and persistent cache modes
  - Automatic path scoping to `/target` directory
- **Workspace cache volumes** — persistent MCP SQLite caches per project
- Comprehensive unit tests for locks, sanitization, compaction, manifest validation,
  MCP config generation, and runtime trait compliance
- `CHANGELOG.md`
- `docs/microvm.md` — MicroVM architecture justification and sizing guide
- `docs/mcp-integration.md` — codebase-memory-mcp integration strategy

### Changed
- Refactored `OpenCodeBridge` to use the `AgentRuntime` trait instead of spawning
  a bare process — enabling MicroVM provisioning
- Orchestrator lifecycle now sets up MCP servers before agent injection
- `Workspace` struct includes optional `.mcp_cache` directory for persistent graphs

### Security
- MCP server root scoped exclusively to `/target` — no cross-tenant index access
- MicroVM backend provides hardware-level isolation (separate kernel per agent)
