# Getting Started

This guide walks you through running Conductor locally for development.

## Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| [Rust](https://rustup.rs) | 1.75+ | Compiling Conductor |
| [PostgreSQL](https://postgresql.org) | 16+ | State & locks |
| [Docker](https://docker.com) | any | Running PostgreSQL easily |
| [OpenCode](https://opencode.ai) | latest | The agent runtime |

## Step 1 — Start PostgreSQL

```bash
docker compose up -d
```

This starts PostgreSQL on `localhost:5432` with user `conductor`, password
`conductor`, database `conductor`.

## Step 2 — Configure Environment

```bash
cp .env.example .env
```

Edit `.env` if your PostgreSQL credentials differ. The defaults match the
`docker-compose.yml` setup.

## Step 3 — Build & Run

```bash
cargo run
```

Conductor will:
1. Connect to PostgreSQL
2. Run migrations automatically
3. Start the HTTP server on `0.0.0.0:7878`

Verify:
```bash
curl http://localhost:7878/api/v1/health
# → "ok"
```

## Step 4 — Create a Tenant & Project

```sql
-- Connect to PostgreSQL
psql postgres://conductor:conductor@localhost:5432/conductor

INSERT INTO tenants (name, slug) VALUES ('Acme Corp', 'acme');
INSERT INTO projects (tenant_id, name, repo_url, default_branch)
  SELECT gen_random_uuid(), 'webapp',
         'https://github.com/myorg/webapp.git', 'main'
  WHERE NOT EXISTS (SELECT 1 FROM projects WHERE name = 'webapp');
```

## Step 5 — Start an Agent Session

```bash
curl -X POST http://localhost:7878/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_slug": "acme",
    "project_id": "<your-project-uuid>",
    "branch": "feature/test",
    "skills_repo": "https://github.com/myorg/agent-skills.git",
    "instruction": "Write unit tests for the auth module"
  }'
```

Check status:
```bash
curl http://localhost:7878/api/v1/sessions/<session-id>/status
```

## Troubleshooting

### Migration errors
Migrations run automatically on startup. If they fail, ensure PostgreSQL is
running and `DATABASE_URL` is correct.

### OpenCode not found
Set `OPENCODE_PATH` in `.env` to the full path of your OpenCode binary.

### Workspace permissions
Ensure `CONDUCTOR_WORKSPACE_ROOT` is writable by the Conductor process.
