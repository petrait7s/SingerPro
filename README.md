# Singer Pro

Offline-first dealer ERP for Windows (retail, wholesale, installments, purchasing, inventory, accounting).
Developed by PETRA Software Solutions.

Status: **P0 foundation complete** (see `CHANGELOG.md`, `docs/CHECKLIST.md`).

## Develop

Prerequisites: Node 22, pnpm 10, Rust stable. Linux also needs `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libssl-dev libxdo-dev`.

```bash
pnpm install
pnpm tauri dev
```

## Verify

```bash
pnpm format:check && pnpm lint && pnpm typecheck && pnpm test
cargo test -p singerpro-core
```

## Layout

- `crates/core` — money, ids, time, SQLite, migrations, audit, sequences (no UI dependency)
- `src-tauri` — Tauri shell and typed commands
- `src` — React UI; all backend calls go through `src/lib/api.ts`
- `.github/workflows/build-windows.yml` — CI and NSIS installer

See `ARCHITECTURE.md`, `DATABASE.md`, `BUILD.md`, `PROJECT_DECISIONS.md`.
