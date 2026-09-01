# Codex Controlled Task Matrix

This matrix is the Codex-only evidence set for the technical preview. Every run uses the fixed `fake-codex-app-server.cmd` fixture unless a separate user authorization records a real model invocation, project, model, and cost. The current release has no such authorization: `real invocation deferred`.

## Sample projects

- TypeScript sample: a small package with one deterministic unit test and one intentionally isolated defect.
- Rust sample: a Cargo crate with a checked-in test fixture and a read-only inspection route.
- Mixed sample: a TypeScript client plus Rust Supervisor contract, with no credentials, network endpoints, or customer source.

## Ten categories

| Category | Required evidence | Safety boundary |
|---|---|---|
| `readonly_explanation` | Contract, read-only mode, event sequence, explanation and validation result | No write or network action |
| `single_file_bug_fix` | One-file diff, approval state, test result and checkpoint | Workspace allowlist is one file |
| `multi_file_feature` | Contract change, multi-file diff, approval and verification | Explicit path scope and diff review |
| `test_failure_repair` | Failing test evidence, repair diff, rerun result | Test command is allowlisted |
| `approval_required_action` | Pending approval, user decision, action digest and resulting event | No action before approval |
| `dirty_workspace` | Baseline hash, dirty status, preservation decision and final hash | Never overwrite user changes |
| `long_context_compression` | Context pack hash, excluded evidence IDs and replay result | Hidden reasoning and raw provider payload stay out |
| `safe_pause` | Pause request, safe boundary, owned process state and resume token | Pause is fail-closed |
| `restart_recovery` | Last committed sequence, recovery package and restart decision | No implicit resume or Completed state |
| `evidence_export_redaction` | Export preview, redaction result, blob/hash references and audit receipt | No source, secret, cookie, header or key |

## Evidence rules

Each result follows `task-result-template.md` and is written once. A failed sample is retained under its own evidence root; reruns use a new root and output path. Results must be `controlled` Codex runs with `realInvocation: deferred`, `sourceRedacted: true`, and `secretsPresent: false`. Event sequences must be contiguous and every result must include Contract, Loadout, approval, Diff, validation and Recovery fields, even when a field is explicitly `not_required`.

The summarizer refuses missing or duplicate categories, P0/P1 defect IDs, non-redacted fields, real invocation claims, and output overwrite. Any P0/P1 security, approval or recovery issue keeps the preview internal and blocks expansion.
