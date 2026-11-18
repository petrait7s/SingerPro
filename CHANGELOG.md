# Changelog

## Unreleased — P1 users, settings, backup

- First-run Owner setup, Argon2id, lockout, in-memory sessions held in Rust.
- RBAC with 34 permissions and 7 system roles, enforced in core services; privilege-escalation guards.
- Typed, audited settings; logo upload; EN/বাংলা UI with bundled fonts.
- Verified backups with manifest; staged, reversible restore applied at startup.
- 56 Rust + 32 UI tests.

## 0.1.0 — P0 foundation

- Cargo workspace: `singerpro-core` + Tauri 2 shell; React/TS/Tailwind UI shell with bundled fonts.
- Money, UUIDv7, Dhaka time, SQLite (WAL, FK), migration 0001, audit trail, document sequences.
- CI workflow for Windows NSIS installer.
