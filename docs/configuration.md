# Configuration Reference

All configuration is via environment variables. See [`.env.example`](../.env.example)
for a template.

## Database

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | ✅ | — | PostgreSQL connection string |

**Example:**
```
DATABASE_URL=postgres://conductor:password@db.example.com:5432/conductor
```

## Server

| Variable | Required | Default | Description |
|---|---|---|---|
| `CONDUCTOR_HOST` | — | `0.0.0.0` | HTTP/WS bind address |
| `CONDUCTOR_PORT` | — | `7878` | HTTP/WS port |

## Workspace

| Variable | Required | Default | Description |
|---|---|---|---|
| `CONDUCTOR_WORKSPACE_ROOT` | — | `/workspace` | Root directory for ephemeral agent workspaces |

Each session creates:
```
{WORKSPACE_ROOT}/{tenant_slug}/{project_name}/{branch}/
├── target/   ← business code (read-write)
└── skills/   ← agent tools   (read-only)
```

## Agent Defaults

| Variable | Required | Default | Description |
|---|---|---|---|
| `CONDUCTOR_MAX_CONTEXT_TOKENS` | — | `80000` | Token threshold before compaction |
| `CONDUCTOR_SESSION_TIMEOUT_SECS` | — | `3600` | Hard timeout per session |

### Context Compaction

When a session's history exceeds `CONDUCTOR_MAX_CONTEXT_TOKENS` tokens,
Conductor prunes the densest `tool_result` entries (measured with
`tiktoken-rs`) while preserving agent responses and user instructions. The
compacted history is stored in PostgreSQL and re-injected on the next session.

### Session Timeout

If an agent doesn't complete within `CONDUCTOR_SESSION_TIMEOUT_SECS`:
1. The OpenCode process is killed
2. The session is marked `failed`
3. The workspace is cleaned up
4. The advisory lock is released

## OpenCode Integration

| Variable | Required | Default | Description |
|---|---|---|---|
| `OPENCODE_PATH` | — | `opencode` | Path to OpenCode CLI binary |
| `OPENCODE_API_KEY` | — | — | API key injected into the agent environment |

## Logging

| Variable | Required | Default | Description |
|---|---|---|---|
| `RUST_LOG` | — | `info` | Log filter (tracing-subscriber) |

**Debug example:**
```
RUST_LOG=conductor=debug,sqlx=warn,tower_http=debug
```
