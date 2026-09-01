# Codex app-server schema 0.147.0

This directory is a vendored, read-only test snapshot of the official Codex CLI
app-server schema used by the protocol consistency gate.

- Baseline: Codex CLI `0.147.0`
- Source command: `codex app-server generate-json-schema --out <directory> --experimental`
- Source snapshot: generated from the local Codex 0.147.0 installation and copied into this repository.
- Integrity: `snapshot-manifest.json` records every file SHA256 and the aggregate hash. The security test fails on drift.

Set `CODEX_SCHEMA_ROOT` only when comparing this snapshot with an explicitly
provided schema directory. CI and local tests use this vendored copy by default.
