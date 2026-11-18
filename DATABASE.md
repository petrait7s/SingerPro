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

## 0002_users_roles_settings_backup

| Table                                                    | Purpose                                                                                                                                          |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `users`                                                  | Argon2id hash, active flag, must-change-password, failed attempts / lockout, last login. Unique case-insensitive username.                       |
| `roles`, `permissions`, `role_permissions`, `user_roles` | RBAC. `user_roles` uses `ON DELETE RESTRICT`: roles/users in use cannot be removed. System roles seeded once; Owner always has every permission. |
| `binary_assets`                                          | Shop logo (PNG/JPEG) so backups carry it.                                                                                                        |
| `backup_history`                                         | Backups, pre-restore copies, staged/applied/failed restores.                                                                                     |

`app_settings` is now a closed, validated catalogue (see `crates/core/src/settings.rs`).
Permissions catalogue (34 keys) lives in `crates/core/src/authz.rs` and is synchronised at startup.
