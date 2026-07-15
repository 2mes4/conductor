# Conductor

**Rust orchestration control plane for remote AI agents.**

Conductor governs the full lifecycle of AI agent sessions using a **Backend For Frontend (BFF)** architecture. It separates orchestration (Rust) from execution (OpenCode) and solves the five critical structural bottlenecks of working with agents at scale.

---

## Problems Solved

| Problem | Solution |
|---|---|
| **Concurrency collisions** | Distributed locks via PostgreSQL `pg_advisory_lock`. Prevents two agents from mutating the same branch simultaneously. |
| **Data leakage (multi-tenant)** | Strict ephemeral sandboxing. Each agent runs in an isolated workspace with only its project mounted. |
| **Context overflow (LLM)** | External compaction with `tiktoken-rs`. Rust controls history and prunes dense tool outputs when tokens exceed 80K. |
| **Tool coupling** | **Dual-Checkout**. Business code (`/target`) is separated from agent tools (`/skills`). Update tools independently of client code. |
| **Zombie executions** | Managed teardown. Hard timeout kills runaway agents, rolls back, and frees resources. |

---

## Architecture

```
  ┌───────────┐    ┌────────────┐    ┌──────────┐    ┌─────────────┐
  │ 1. State  │───▶│ 2. Checkout│───▶│ 3.Bridge │───▶│ 4. Teardown │
  │ Lock + DB │    │ target +   │    │ OpenCode │    │ compact +   │
  │           │◀───│  skills    │◀───│ inject   │◀───│ persist     │
  └───────────┘    └────────────┘    └──────────┘    └─────────────┘
```

### The Four Layers

1. **State & Concurrency** — PostgreSQL is the single source of truth. Advisory locks serialise work per branch. `axum` + `tokio` expose WebSockets for real-time log streaming.
2. **Dual-Checkout Engine** — Native Git operations via `git2-rs` (never shell calls). Clones or fetches `/target` and `/skills` into isolated workspaces.
3. **Contract Validation & Injection** — Deserialises `AgentStackManifest` with `#[serde(deny_unknown_fields)]`. Sanitises paths against traversal. Spawns OpenCode with correct mounts and injects the session payload.
4. **Teardown & Persistence** — Extracts session output, compacts with `tiktoken-rs`, commits and pushes via `git2`, stores to Postgres, releases the lock, and cleans up.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full specification.

---

## Quick Start

### Prerequisites

- **Rust** 1.75+ (`rustup`)
- **PostgreSQL** 16+
- **OpenCode** CLI installed

### Setup

```bash
# Clone
git clone https://github.com/2mes4/conductor.git
cd conductor

# Start PostgreSQL
docker compose up -d

# Copy environment config
cp .env.example .env
# Edit .env with your values

# Run migrations & start
cargo run
```

### Create a Session

```bash
curl -X POST http://localhost:7878/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_slug": "acme",
    "project_id": "00000000-0000-0000-0000-000000000001",
    "branch": "feature/auth",
    "skills_repo": "https://github.com/myorg/agent-skills.git",
    "instruction": "Add input validation to the login form"
  }'
```

---

## Project Structure

```
conductor/
├── src/
│   ├── main.rs              # Entry point
│   ├── lib.rs               # Crate root
│   ├── config.rs            # Environment configuration
│   ├── error.rs             # Unified error types
│   ├── models/              # Domain models (Tenant, Project, Session)
│   ├── state/               # Layer 1: DB + advisory locks
│   ├── checkout/            # Layer 2: Dual-Checkout (git2)
│   ├── bridge/              # Layer 3: Manifest + sanitize + OpenCode
│   ├── runtime/             # Execution backends (MicroVM / Local)
│   ├── mcp/                 # codebase-memory-mcp integration
│   ├── teardown/            # Layer 4: Compaction + persist + cleanup
│   ├── orchestrator/        # Lifecycle tying all layers together
│   └── server/              # axum HTTP + WebSocket server
├── migrations/              # PostgreSQL schema migrations
├── examples/                # Example manifest.json
├── docs/                    # Documentation
│   ├── microvm.md           # MicroVM justification & host sizing
│   └── mcp-integration.md   # codebase-memory-mcp strategy
└── docker-compose.yml       # Local PostgreSQL
```

---

## Configuration

All configuration is via environment variables (see [`.env.example`](.env.example)):

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | — | PostgreSQL connection string (required) |
| `CONDUCTOR_HOST` | `0.0.0.0` | Server bind address |
| `CONDUCTOR_PORT` | `7878` | Server port |
| `CONDUCTOR_WORKSPACE_ROOT` | `/workspace` | Root for ephemeral workspaces |
| `CONDUCTOR_MAX_CONTEXT_TOKENS` | `80000` | Token threshold for compaction |
| `CONDUCTOR_SESSION_TIMEOUT_SECS` | `3600` | Hard agent timeout |
| `OPENCODE_PATH` | `opencode` | Path to OpenCode CLI binary |
| `OPENCODE_API_KEY` | — | API key injected into agent env |

---

## API Reference

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/health` | Health check |
| `POST` | `/api/v1/sessions` | Create and queue a new agent session |
| `GET` | `/api/v1/sessions/:id` | Get session details |
| `GET` | `/api/v1/sessions/:id/status` | Get session status |

---

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## Documentation

- [Architecture](ARCHITECTURE.md) — Foundational specification
- [MicroVM Execution](docs/microvm.md) — Why MicroVMs & host sizing
- [MCP Integration](docs/mcp-integration.md) — codebase-memory-mcp strategy
- [Getting Started](docs/getting-started.md) — Local setup guide
- [Configuration](docs/configuration.md) — All environment variables
- [API Reference](docs/api-reference.md) — REST & WebSocket API

## License

[MIT](LICENSE)
