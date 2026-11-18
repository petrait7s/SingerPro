# Architecture

```
React UI (src)  --typed invoke-->  Tauri commands (src-tauri)  -->  singerpro-core (Rust)  -->  SQLite
```

- UI never touches SQLite or SQL. Commands are narrow and typed; errors cross as `{code, message}`.
- Business rules live in `singerpro-core` services; commands only translate.
- Every multi-record operation takes a `&mut SqliteConnection` from the caller's transaction (audit, sequences already follow this).
- Data lives in the OS app-data dir (`<app_data>/data/singerpro.db`), never the install dir; upgrades only run additive migrations.
- Future PostgreSQL: keep SQL inside repositories, UUID ids and integer money are already portable. Copying the SQLite file between PCs is NOT multi-user sync; shared access needs a server DB/API.
