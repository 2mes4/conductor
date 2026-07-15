# codebase-memory-mcp Integration

[codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp) converts
the repository into a local knowledge graph (tree-sitter + SQLite) that the
agent can query structurally. This reduces token consumption by up to **99%**
and prevents early session context overflow.

---

## How It Works in Conductor

### 1. Binary Location (The "Archivist")

Unlike skill-specific scripts that live in the dynamic `/skills` repository,
`codebase-memory-mcp` is a single static Go binary with no external dependencies.

**Strategy:** Bake the binary directly into the MicroVM rootfs image at
`/usr/local/bin/codebase-memory-mcp`. As a central infrastructure tool, it
should be pre-installed and ready for any agent that the OpenCode engine spins
up.

### 2. Dynamic Injection from Rust

OpenCode needs instructions on how to launch and connect to this MCP server.
The Rust orchestrator arbitrates this link:

1. After Git checkout of the code in `/workspace/{tenant}/{project}/{branch}/target`,
   Rust generates an `mcp_settings.json` file on the fly.
2. This JSON instructs OpenCode to launch the MCP server as a strict subprocess,
   passing the `/target` directory as the exclusive scan root.
3. The settings file is mounted read-only into the guest at
   `/config/mcp_settings.json`.

**Sandboxing:** Since the MCP binary runs inside the MicroVM, it inherits all
workspace isolation policies. It only has read access to the target tenant's
code and cannot index outside its boundaries.

### 3. Graph Strategy (Ephemeral vs. Persistent)

The MCP builds a small SQLite database to store the dependency graph and paths.
Given that the execution environment is destroyed on completion, Rust has two
paths for managing this database:

#### Ephemeral / RAM-first (Default, Recommended)

The MCP is so optimised that it indexes a medium-sized repository in
milliseconds. In most cases, the orchestrator should **not** persist the SQLite
database. Let the server build the graph directly in RAM (`:memory:`) from
scratch each time the MicroVM boots. When the container is destroyed at the end
of the event, memory is freed — guaranteeing Zero-Trust security and avoiding
code cache desynchronisation issues.

#### Persistent via Volumes (For Giant Monoliths)

If a client works on massive repositories, the orchestrator can create an
isolated directory `/workspace/{tenant}/{project}/.mcp_cache/` and mount it as a
persistent volume inside the MicroVM. By instructing the MCP to save its `.db`
there, future sessions on the same project only require an ultra-fast
incremental scan of files modified since the last commit.

### 4. Reduced Overhead for the Orchestrator

By delegating deep code understanding to this MCP server via structural queries
(e.g., "who calls function X?"), the LLM dramatically reduces kilometre-long
text tool responses. This means the compaction and context pruning logic
implemented in Rust (with `tiktoken-rs`) intervenes far less often. The Postgres
database stays lighter and AI inference costs plummet.

---

## Implementation in Code

The MCP integration lives in [`src/mcp/`](../src/mcp/):

```rust
use conductor::mcp::{CacheMode, CodebaseMemoryMcp};

// Ephemeral mode (default)
let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Ephemeral)?;
mcp.write_settings(&workspace.root.join("mcp_settings.json"))?;

// Persistent mode (for large repos)
let mcp = CodebaseMemoryMcp::new("/target", CacheMode::Persistent)?
    .with_cache_path(&workspace.mcp_cache);
mcp.ensure_cache_dir()?;
mcp.write_settings(&workspace.root.join("mcp_settings.json"))?;
```

### Generated `mcp_settings.json` (ephemeral example)

```json
{
  "mcpServers": {
    "codebase-memory": {
      "command": "/usr/local/bin/codebase-memory-mcp",
      "args": ["/target", "--memory", ":memory:"]
    }
  }
}
```

### Generated `mcp_settings.json` (persistent example)

```json
{
  "mcpServers": {
    "codebase-memory": {
      "command": "/usr/local/bin/codebase-memory-mcp",
      "args": ["/target", "--memory", "/target/.mcp_cache/graph.db"],
      "env": {
        "MCP_CACHE_DIR": "/target/.mcp_cache/graph.db"
      }
    }
  }
}
```

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `CONDUCTOR_MCP_CACHE_MODE` | `ephemeral` | `ephemeral` or `persistent` |
| `CONDUCTOR_MCP_BINARY` | `/usr/local/bin/codebase-memory-mcp` | Path to the MCP binary in the guest |

The orchestrator automatically selects persistent mode if a `.mcp_cache`
directory already exists for the project.
