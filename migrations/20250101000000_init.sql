-- ════════════════════════════════════════════════════════════
-- Conductor — Initial schema
-- Hierarchy: Tenant → Project → Session
-- ════════════════════════════════════════════════════════════

-- ─── Enums ───────────────────────────────────────────────────

CREATE TYPE session_status AS ENUM (
    'queued',
    'preparing',
    'running',
    'tearingdown',
    'completed',
    'failed'
);

-- ─── Tenants ─────────────────────────────────────────────────

CREATE TABLE tenants (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ─── Projects ────────────────────────────────────────────────

CREATE TABLE projects (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    repo_url        TEXT NOT NULL,
    default_branch  TEXT NOT NULL DEFAULT 'main',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, name)
);

CREATE INDEX idx_projects_tenant ON projects(tenant_id);

-- ─── Sessions ────────────────────────────────────────────────

CREATE TABLE sessions (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id   UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    branch       TEXT NOT NULL,
    status       session_status NOT NULL DEFAULT 'queued',
    instruction  TEXT NOT NULL,
    history      JSONB NOT NULL DEFAULT '{"messages": []}'::jsonb,
    commit_sha   TEXT,
    tokens_used  BIGINT NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sessions_tenant_project ON sessions(tenant_id, project_id);
CREATE INDEX idx_sessions_branch         ON sessions(branch);
CREATE INDEX idx_sessions_status         ON sessions(status);

-- Auto-update updated_at on row change.
CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sessions_updated_at
    BEFORE UPDATE ON sessions
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at();
