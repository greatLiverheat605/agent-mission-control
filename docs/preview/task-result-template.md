# Codex Task Result Template

Store one UTF-8 JSON object per task. The file is an immutable evidence receipt: never edit a failed result in place and never replace a report at the same output path.

```json
{
  "schema": "mission-control-codex-task-result-v1",
  "taskId": "codex-matrix-01",
  "category": "readonly_explanation",
  "mode": "controlled",
  "provider": "codex",
  "realInvocation": "deferred",
  "status": "passed",
  "modelVersion": "fake-codex-app-server-v1",
  "loadoutFingerprint": "fixture-loadout-v1",
  "contractFingerprint": "fixture-contract-v1",
  "drivingMode": "read_only",
  "events": [{ "sequence": 1, "kind": "agent_run_started" }],
  "approvals": [],
  "diff": { "status": "none", "paths": [] },
  "validation": { "passed": true, "commands": ["fixture-check"] },
  "recovery": { "status": "not_required", "checkpointId": null },
  "userUnderstandingSeconds": 6,
  "defectIds": [],
  "secretsPresent": false,
  "sourceRedacted": true
}
```

Do not add raw source, provider payloads, prompts, headers, cookies, credentials, keys, hidden reasoning, or screenshots. Use hashes, allowlisted paths and redacted summaries instead. `defectIds` may identify a P2 issue, but any `P0` or `P1` entry makes the matrix fail.
