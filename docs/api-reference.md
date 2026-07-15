# API Reference

Base URL: `http://{CONDUCTOR_HOST}:{CONDUCTOR_PORT}/api/v1`

---

## Health

### `GET /health`

Returns `200 OK` with body `"ok"`.

---

## Sessions

### Create Session

```
POST /sessions
```

**Request body:**

```json
{
  "tenant_slug": "acme",
  "project_id": "550e8400-e29b-41d4-a716-446655440000",
  "branch": "feature/auth",
  "skills_repo": "https://github.com/myorg/agent-skills.git",
  "instruction": "Add input validation to the login form",
  "resume_from": null
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `tenant_slug` | string | ✅ | Tenant identifier |
| `project_id` | UUID | ✅ | Project to operate on |
| `branch` | string | ✅ | Git branch to work on |
| `skills_repo` | string | ✅ | URL of the skills/tools repository |
| `instruction` | string | ✅ | Natural-language task for the agent |
| `resume_from` | UUID | — | Previous session ID to resume from |

**Response `202 Accepted`:**

```json
{
  "session_id": "660e8400-e29b-41d4-a716-446655440000",
  "status": "queued"
}
```

**Errors:**

| Status | Condition |
|---|---|
| `404` | Tenant or project not found |
| `409` | Branch is locked by another session |
| `500` | Internal error |

---

### Get Session

```
GET /sessions/:id
```

**Response `200 OK`:**

```json
{
  "id": "660e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "...",
  "project_id": "...",
  "branch": "feature/auth",
  "status": "completed",
  "instruction": "Add input validation...",
  "history": { "messages": [...] },
  "commit_sha": "abc1234",
  "tokens_used": 45200,
  "created_at": "2025-07-14T10:00:00Z",
  "updated_at": "2025-07-14T10:05:00Z"
}
```

---

### Get Session Status

```
GET /sessions/:id/status
```

**Response `200 OK`:**

```json
{
  "id": "660e8400-...",
  "status": "running",
  "commit_sha": null,
  "tokens_used": 0
}
```

### Session Status Values

| Status | Description |
|---|---|
| `queued` | Waiting for advisory lock |
| `preparing` | Lock acquired, workspace being mounted |
| `running` | OpenCode agent executing |
| `tearingdown` | Compacting, committing, cleaning up |
| `completed` | Finished successfully |
| `failed` | Error or timeout |

---

## WebSocket — Live Log Stream

```
WS /sessions/:id/stream
```

Connect via WebSocket to receive real-time agent output. Messages are JSON
objects with a `type` field:

```json
{"type": "log", "level": "info", "message": "..."}
{"type": "status", "status": "running"}
{"type": "tool_call", "tool": "lint", "args": {...}}
{"type": "done", "commit_sha": "abc1234"}
```
