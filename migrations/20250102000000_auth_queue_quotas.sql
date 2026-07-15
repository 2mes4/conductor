-- ════════════════════════════════════════════════════════════
-- Conductor — Phase 2 & 3 schema additions
-- Auth, job queue, resource quotas
-- ════════════════════════════════════════════════════════════

-- ─── API Keys (Auth) ─────────────────────────────────────────

CREATE TABLE api_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    key_hash     TEXT NOT NULL UNIQUE,
    label        TEXT NOT NULL DEFAULT 'default',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at   TIMESTAMPTZ
);

CREATE INDEX idx_api_keys_hash ON api_keys(key_hash) WHERE revoked_at IS NULL;
CREATE INDEX idx_api_keys_tenant ON api_keys(tenant_id);

-- ─── Job Queue ───────────────────────────────────────────────

CREATE TYPE job_priority AS ENUM ('low', 'normal', 'high');

CREATE TABLE job_queue (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id    UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tenant_id     UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id    UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    branch        TEXT NOT NULL,
    payload       JSONB NOT NULL,
    priority      job_priority NOT NULL DEFAULT 'normal',
    attempts      INT NOT NULL DEFAULT 0,
    max_attempts  INT NOT NULL DEFAULT 3,
    status        TEXT NOT NULL DEFAULT 'pending',
    locked_by     TEXT,
    locked_at     TIMESTAMPTZ,
    available_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at  TIMESTAMPTZ
);

CREATE INDEX idx_jobs_claim ON job_queue(status, available_at)
    WHERE status = 'pending';
CREATE INDEX idx_jobs_session ON job_queue(session_id);

-- ─── Resource Quotas ─────────────────────────────────────────

CREATE TABLE tenant_quotas (
    tenant_id              UUID PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    max_concurrent_sessions  INT NOT NULL DEFAULT 5,
    max_session_timeout_secs  INT NOT NULL DEFAULT 3600,
    max_context_tokens       INT NOT NULL DEFAULT 80000,
    microvm_memory_mb        INT NOT NULL DEFAULT 512,
    microvm_vcpus            INT NOT NULL DEFAULT 1,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Default quota row for existing tenants.
INSERT INTO tenant_quotas (tenant_id)
    SELECT id FROM tenants
    ON CONFLICT (tenant_id) DO NOTHING;

-- Auto-create quota when a new tenant is inserted.
CREATE OR REPLACE FUNCTION ensure_tenant_quota()
RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO tenant_quotas (tenant_id) VALUES (NEW.id)
    ON CONFLICT (tenant_id) DO NOTHING;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tenants_quota
    AFTER INSERT ON tenants
    FOR EACH ROW
    EXECUTE FUNCTION ensure_tenant_quota();

-- Notify channel for LISTEN/NOTIFY-based job dispatch.
CREATE OR REPLACE FUNCTION notify_job_available()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('job_available', NEW.id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER job_queue_notify
    AFTER INSERT ON job_queue
    FOR EACH ROW
    WHEN (NEW.status = 'pending')
    EXECUTE FUNCTION notify_job_available();
