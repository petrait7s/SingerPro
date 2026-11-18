-- P1: identity, RBAC, binary assets (logo), backup history.

CREATE TABLE users (
    id                   TEXT PRIMARY KEY,
    username             TEXT NOT NULL UNIQUE COLLATE NOCASE,
    full_name            TEXT NOT NULL,
    password_hash        TEXT NOT NULL,              -- Argon2id PHC string; never plaintext
    is_active            INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    must_change_password INTEGER NOT NULL DEFAULT 0 CHECK (must_change_password IN (0, 1)),
    failed_attempts      INTEGER NOT NULL DEFAULT 0,
    locked_until         TEXT,
    last_login_at        TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);

CREATE TABLE roles (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT NOT NULL DEFAULT '',
    is_system   INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0, 1)),
    created_at  TEXT NOT NULL
);

CREATE TABLE permissions (
    key         TEXT PRIMARY KEY,
    category    TEXT NOT NULL,
    description TEXT NOT NULL
);

CREATE TABLE role_permissions (
    role_id  TEXT NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    perm_key TEXT NOT NULL REFERENCES permissions (key) ON DELETE CASCADE,
    PRIMARY KEY (role_id, perm_key)
);

CREATE TABLE user_roles (
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    role_id TEXT NOT NULL REFERENCES roles (id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, role_id)
);
CREATE INDEX ix_user_roles_role ON user_roles (role_id);

-- Small binary assets (shop logo). Lives in the DB so backups include it.
CREATE TABLE binary_assets (
    key        TEXT PRIMARY KEY,
    mime       TEXT NOT NULL,
    data       BLOB NOT NULL,
    sha256     TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE backup_history (
    id             TEXT PRIMARY KEY,
    at             TEXT NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('backup', 'pre_restore', 'restore_staged', 'restore_applied', 'restore_failed')),
    path           TEXT NOT NULL,
    size_bytes     INTEGER,
    sha256         TEXT,
    schema_version INTEGER,
    status         TEXT NOT NULL,
    actor_id       TEXT,
    note           TEXT
);
CREATE INDEX ix_backup_history_at ON backup_history (at);
