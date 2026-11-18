# Database

Migrations: `crates/core/migrations/*.sql`, embedded and applied at startup by sqlx. Never edit an applied migration; add a new one.

## 0001_foundation

| Table                | Purpose                                                                          |
| -------------------- | -------------------------------------------------------------------------------- |
| `branches`           | Branch concept; single default branch (partial unique index).                    |
| `app_settings`       | Key/value configuration (defaults seeded by `db::bootstrap`, never overwritten). |
| `audit_logs`         | Append-only (UPDATE/DELETE blocked by triggers).                                 |
| `document_sequences` | Prefix/padding/next number per document type.                                    |

Conventions: UUIDv7 TEXT ids, UTC RFC3339 timestamps, money as INTEGER paisa, FK enforced on every connection.
Health: `db::status` reports schema version, WAL, FK, `integrity_check`, `foreign_key_check`.
