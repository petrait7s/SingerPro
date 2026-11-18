-- P0 foundation. All IDs are UUIDv7 TEXT, timestamps UTC RFC3339 TEXT, money INTEGER paisa.

-- Branch concept kept from day one so multi-branch later is additive, not a rewrite.
CREATE TABLE branches (
    id         TEXT PRIMARY KEY,
    code       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at TEXT NOT NULL
);
-- Exactly one default branch.
CREATE UNIQUE INDEX ux_branches_single_default ON branches (is_default) WHERE is_default = 1;

CREATE TABLE app_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    updated_by TEXT
);

-- Append-only. actor_id deliberately has no FK: audit history must outlive user records.
CREATE TABLE audit_logs (
    id          TEXT PRIMARY KEY,
    at          TEXT NOT NULL,
    actor_id    TEXT,
    action      TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id   TEXT,
    detail      TEXT CHECK (detail IS NULL OR json_valid(detail)),
    reason      TEXT
);
CREATE INDEX ix_audit_at ON audit_logs (at);
CREATE INDEX ix_audit_entity ON audit_logs (entity_type, entity_id);
CREATE INDEX ix_audit_actor ON audit_logs (actor_id, at);

CREATE TRIGGER trg_audit_no_update BEFORE UPDATE ON audit_logs
BEGIN SELECT RAISE(ABORT, 'audit_logs is append-only'); END;
CREATE TRIGGER trg_audit_no_delete BEFORE DELETE ON audit_logs
BEGIN SELECT RAISE(ABORT, 'audit_logs is append-only'); END;

-- Gap-free-per-commit document numbering (INV-000001 etc.). Incremented inside the caller's transaction.
CREATE TABLE document_sequences (
    name       TEXT PRIMARY KEY,
    prefix     TEXT NOT NULL DEFAULT '',
    padding    INTEGER NOT NULL DEFAULT 6 CHECK (padding BETWEEN 1 AND 12),
    next_value INTEGER NOT NULL DEFAULT 1 CHECK (next_value >= 1)
);
