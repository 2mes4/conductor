# Architecture — Conductor

> This document is the foundational specification for the Conductor control plane.
> It details exactly which structural problems this architecture solves and the
> step-by-step guide for implementing each layer.

---

## Critical Problems Solved

Separating orchestration from execution using a **Backend For Frontend (BFF)** pattern
resolves the five major bottlenecks of working with agents remotely:

| Structural Problem | Architectural Solution |
|---|---|
| **Concurrency Collisions** | A Distributed Locks model (PostgreSQL). Prevents two agents from trying to modify the same branch of the same project simultaneously, avoiding Git corruption. |
| **Data Leakage (Multi-tenant)** | Strict Sandboxing. The agent runs in an ephemeral environment (OpenCode) where only the specific project directory is mounted. It is physically isolated from other tenants. |
| **Context Overflow (LLM)** | External Compaction. Because OpenCode is amnesic, Rust controls the history. Using `tiktoken-rs`, it prunes unnecessary logs when the 80K token limit is exceeded before re-injecting the session. |
| **Tool Coupling** | Dual-Checkout. Separates business code (`/target`) from tools (`/skills`). You can update agent tools independently of the client's project. |
| **Uncontrolled Executions (Zombies)** | Managed Teardown. Rust enforces a strict timeout. If an agent loops, the process is killed, the directory is rolled back, and the space is freed. |

---

## System Overview

```
                    ┌──────────────────────────────────────────────────┐
                    │                   Conductor (Rust)                │
                    │                                                  │
  ┌────────┐        │  ┌─────────┐  ┌──────────┐  ┌────────┐  ┌─────┐ │
  │  HTTP  │◀──────▶│  │ Server  │  │Orchestra-│  │  State │  │ DB  │ │
  │  / WS  │        │  │ (axum)  │─▶│   tor    │─▶│ (Locks)│─▶│(PG) │ │
  └────────┘        │  └─────────┘  └──────────┘  └────────┘  └─────┘ │
                    │       │              │           │               │
                    │       │         ┌────┴────┐      │               │
                    │       │         ▼         ▼      │               │
                    │  ┌─────────┐ ┌──────┐ ┌────────┐│               │
                    │  │Checkout │ │Bridge│ │Teardown││               │
                    │  │(git2)   │ │(OC)  │ │(tiktok)││               │
                    │  └────┬────┘ └──┬───┘ └────────┘│               │
                    └───────┼─────────┼──────────────┼───────────────┘
                            │         │              │
                     ┌──────▼──┐ ┌────▼─────┐  ┌─────▼──────┐
                     │ /target │ │ OpenCode │  │ /workspace │
                     │ (R/W)   │ │  Server  │  │ (ephemeral)│
                     ├─────────┤ │ (process)│  └────────────┘
                     │ /skills │ └──────────┘
                     │ (R/O)   │
                     └─────────┘
```

---

## Implementation Layers

### Layer 1 — State & Concurrency (The Rust Core)

The first step is establishing the single source of truth so the system can
scale without losing control.

- **Database Connection**: Configure `sqlx` with PostgreSQL to store the
  `Tenant → Project → Session` hierarchy and the JSON session history.
- **Lock Manager**: Implement a function that executes `pg_advisory_lock`. When
  an event arrives to act on Project A / Branch feature-1, Rust must acquire
  this lock. If it fails, the request is queued.
- **Async Server**: Stand up a web server with `axum` and `tokio` to expose
  WebSockets. This enables pushing agent logs to a lightweight UI in real time.

**Key source files:**
- [`src/state/db.rs`](src/state/db.rs) — Connection pool and data-access queries
- [`src/state/locks.rs`](src/state/locks.rs) — Advisory lock manager
- [`src/server/mod.rs`](src/server/mod.rs) — axum server setup

### Layer 2 — Dual-Checkout Engine (Filesystem)

Here the workspace mounting is implemented. Rust must manipulate Git directly
in memory for security.

- **Native Git**: Use the `git2-rs` library (never bash calls).
- **Mount Code (`/target`)**: Rust checks if
  `/workspace/{tenant}/{project}/{branch}/target` exists. If not, clone the
  repository; if it exists, force a `git fetch` and `git reset --hard` to
  guarantee a clean state before the agent touches it.
- **Mount Stack (`/skills`)**: Rust downloads the requested agent's tool
  repository into a parallel subdirectory.

**Key source files:**
- [`src/checkout/mod.rs`](src/checkout/mod.rs) — Workspace structure
- [`src/checkout/target.rs`](src/checkout/target.rs) — Target code checkout
- [`src/checkout/skills.rs`](src/checkout/skills.rs) — Skills repo checkout

### Layer 3 — Contract Validation & Injection (The OpenCode Bridge)

This is the frontier where data is transformed and security limits are imposed.

- **Manifest Deserialisation**: Using `serde`, read the `AgentStackManifest`
  file from the tools repository. Apply `#[serde(deny_unknown_fields)]` to
  reject any illicit configuration.
- **Path Sanitisation**: Ensure the `executable` field of tools does not contain
  directory traversal (e.g. `../`).
- **Sandboxed Startup**: Rust generates ephemeral environment variables (API
  keys) and starts the OpenCode Server process. It is imperative to mount
  `/target` as read-write and `/skills` as read-only.
- **Payload Injection**: Rust makes a POST to the OpenCode API, delivering the
  recovered history from the database, the JSON schemas of the tools, and the
  user instruction.

**Key source files:**
- [`src/bridge/manifest.rs`](src/bridge/manifest.rs) — Manifest deserialisation
- [`src/bridge/sanitize.rs`](src/bridge/sanitize.rs) — Path sanitisation
- [`src/bridge/opencode.rs`](src/bridge/opencode.rs) — OpenCode lifecycle

### Layer 4 — Teardown & Persistence

When the OpenCode session emits its completion event, Rust must collect the
work and clean up.

1. **Extraction & Pruning**: Extract the session JSON. Use `tiktoken-rs` to
   count tokens; if they exceed the allowed limit, run an algorithm to remove
   the densest `tool_result` entries, preserving agent responses and
   instructions. Store this compacted JSON in Postgres.
2. **Code Saving**: Using `git2-rs`, check the working tree of `/target`. If
   there are changes, do a `git add`, create a commit (e.g.
   `chore(ai): updates by agent`), and `git push` to the original branch.
3. **Destruction**: Remove the OpenCode instance, erase secrets from memory,
   and release the distributed lock in Postgres to make way for the next job.

**Key source files:**
- [`src/teardown/compact.rs`](src/teardown/compact.rs) — Token counting & compaction
- [`src/teardown/persist.rs`](src/teardown/persist.rs) — Commit, push & cleanup

---

## Data Model

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│  Tenant   │────▶│ Project  │────▶│ Session  │
│           │     │          │     │          │
│ id        │     │ id       │     │ id       │
│ name      │     │ tenant_id│     │ proj_id  │
│ slug      │     │ name     │     │ branch   │
└──────────┘     │ repo_url │     │ status   │
                 │ branch   │     │ history  │
                 └──────────┘     │ commit   │
                                  │ tokens   │
                                  └──────────┘
```

The **Session** table stores:
- Lifecycle status (`queued → preparing → running → tearingdown → completed/failed`)
- Compacted JSON history (the LLM context window)
- Git commit SHA (if changes were pushed)
- Token usage (for cost tracking)

---

## Lock Key Derivation

The advisory lock key is derived deterministically from `(project_id, branch)`:

```
key = (uuid_hi_32 << 32) | crc32(branch_name)
```

This ensures:
- Different projects never collide
- Different branches of the same project don't block each other
- The same branch is always serialised

---

## Design Principles

1. **Microagentic Stacking** — Atomic agents governed by strict schemas.
2. **Never Shell Out** — All Git operations use `git2-rs`, never bash.
3. **Amnesic Execution** — The agent process holds no state; Rust owns all
   history and context management.
4. **Strict Deny-by-Default** — Manifests use `deny_unknown_fields`; paths are
   validated against traversal; workspaces are isolated.
5. **Deterministic Teardown** — Every session ends with the same cleanup
   sequence regardless of success or failure.
