# Architecture

```
React UI (src)  --typed invoke-->  Tauri commands (src-tauri)  -->  singerpro-core (Rust)  -->  SQLite
```

- UI never touches SQLite or SQL. Commands are narrow and typed; errors cross as `{code, message}`.
- Business rules live in `singerpro-core` services; commands only translate.
- Every multi-record operation takes a `&mut SqliteConnection` from the caller's transaction (audit, sequences already follow this).
- Data lives in the OS app-data dir (`<app_data>/data/singerpro.db`), never the install dir; upgrades only run additive migrations.
- Future PostgreSQL: keep SQL inside repositories, UUID ids and integer money are already portable. Copying the SQLite file between PCs is NOT multi-user sync; shared access needs a server DB/API.

## Security model (P1)

- **Session**: created by `auth::login` / `setup_owner`; token kept only in the Rust `AppState`. Every command calls `AppState::principal()` which authenticates the token and loads roles/permissions fresh from the DB.
- **Authorization**: each core service takes a `&Principal` and calls `principal.require(perm)` itself. Accounts with a pending password change fail every check.
- **Audit**: every user/role/settings/backup/restore/auth event writes an append-only `audit_logs` row in the same transaction as the change.
- **Recovery**: `recovery::apply_pending` runs before `db::open` at startup (see BACKUP_RESTORE.md).
- **Webview**: CSP `default-src 'self'`; capabilities `core:default` + `dialog:allow-open`; no fs/shell/http plugins.
