# Build

Local: `pnpm install && pnpm tauri build --bundles nsis` on Windows produces `target/release/bundle/nsis/*.exe`.

CI: `.github/workflows/build-windows.yml` — Linux job (format, lint, typecheck, tests, clippy), then Windows job builds the NSIS installer, uploads it as the `singer-pro-windows-nsis` artifact, and attaches it to a GitHub Release on tags `v*`.

The installer is **unsigned**; Windows SmartScreen may warn. Code signing needs a purchased certificate (not configured). No secrets are committed or required.
