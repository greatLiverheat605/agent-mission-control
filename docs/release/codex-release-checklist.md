# Codex-First Release Checklist

This checklist is the final internal technical-preview gate. `verify-codex-release.ps1` runs every item and writes a redacted report with test status, artifact hashes, signing state, task-matrix state and preview thresholds. A failed item returns non-zero and keeps the build internal.

- Security: Codex prompt/IPC/event fuzz and secret persistence scan, with zero P0/P1.
- Recovery: crash, ledger integrity, key-unavailable and owned-process recovery with no overwrite or implicit resume.
- Soak: three-Mission Codex resource and sequence isolation under the short controlled soak.
- Data lifecycle: budget, archive, export, delete and diagnostic previews with impact hashes and audit receipts.
- Signed update: artifact SHA-256, SBOM/provenance, Authenticode/Tauri updater signature boundary, active Mission guard and rollback data preservation.
- Task matrix: ten controlled Codex task categories with immutable evidence and `real invocation deferred` unless separately authorized.
- Preview: five-person controlled pilot thresholds of 80% first launch and 90% state recognition within 10 seconds, with zero P0/P1 and telemetry off.
- Remote allowlist: staged and remote trees contain no `.codex`, `AGENTS.md`, handoff, prompt, screenshot, evidence or raw log artifacts; `origin/main` matches `HEAD`.

Every release decision must cite the concrete `verify-codex-release.ps1` run id and its immutable gate directory (including finding matrix and review-bundle hashes). A decision without a run-id binding is fail-closed.

Claude is deferred in this release. No Claude client or Claude handoff is marked complete by this gate. Under the default `oss-personal` distribution, a zero-failure gate with valid `authorized-real` Codex evidence may report `releaseReady: true`; missing signing credentials and clean-VM evidence remain visible under `optionalEnhancements`. Under `commercial`, those two tracks remain hard blockers and a local run without CI signing credentials must keep `releaseReady: false`.
