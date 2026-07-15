//! MCP (Model Context Protocol) server integration.
//!
//! Integrates [codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp)
//! to give agents structured access to the codebase via a local knowledge graph
//! (tree-sitter + SQLite), reducing token consumption by up to 99%.
//!
//! ## Integration strategy
//!
//! The MCP binary is pre-baked into the MicroVM rootfs at
//! `/usr/local/bin/codebase-memory-mcp`. At session start, Rust generates a
//! dynamic `mcp_settings.json` that instructs OpenCode to launch the MCP server
//! as a subprocess scoped exclusively to the `/target` directory.
//!
//! ## Cache modes
//!
//! - **Ephemeral** (default): The SQLite graph is built in RAM from scratch
//!   each session. Zero persistence, zero trust surface. Fast enough for
//!   medium repos (milliseconds).
//!
//! - **Persistent**: For massive monorepos, the graph is cached at
//!   `/workspace/{tenant}/{project}/.mcp_cache/` and mounted as a volume.
//!   Subsequent sessions only re-index changed files (incremental).

pub mod codebase_memory;
pub mod preindex;

pub use codebase_memory::{CacheMode, CodebaseMemoryMcp, McpServerConfig};
