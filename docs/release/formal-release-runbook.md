# Formal Release Runbook

This runbook supports two distribution policies. `oss-personal` is the default
for personal GitHub open-source distribution; `commercial` retains the full
production-signing and clean-VM prerequisites. Do not use a fixture, dry run,
test certificate, or simulated VM as a production signal under either policy.

For `oss-personal`, `releaseReady=true` requires a zero-failure gate and valid
`authorized-real` Provider evidence. Signing and clean-VM verification are
reported under `optionalEnhancements`, not `formalReleaseBlockedBy`. For
`commercial`, all three production evidence tracks below remain mandatory.

Run the selected policy explicitly:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-codex-release.ps1 -Distribution oss-personal -RealInvocationReport .\artifacts\real-invocation\<run-id>\real-invocation-report.json
```

## Open-source distribution signing enhancements

An unsigned GitHub build is permitted by the `oss-personal` policy, but code
signing improves SmartScreen reputation and gives users a verifiable publisher.
The following services do not require forming a company or owning a domain;
eligibility, identity checks, pricing, and availability must still be confirmed
with the provider before use:

- **SignPath Foundation** offers a free program for eligible open-source
  projects and performs signing through its hosted workflow.
- **Microsoft Trusted Signing** is a managed Azure signing service with
  identity validation and CI integration.
- **Certum Open Source Code Signing** offers certificates intended for
  qualifying open-source maintainers, including individuals.

Whichever route is selected, configure only the GitHub secrets named below,
then use the existing Windows release workflow and verification commands. Do
not commit a PFX, password, private key, or exported certificate bundle.

## 1. Production signing

Configure these GitHub Actions secrets in the repository settings:

| Secret | Format | Source |
| --- | --- | --- |
| `MISSION_CONTROL_SIGNING_PFX_BASE64` | Base64-encoded production PFX | Approved code-signing certificate export |
| `MISSION_CONTROL_SIGNING_PFX_PASSWORD` | PFX password text | Certificate owner secret vault |
| `MISSION_CONTROL_SIGNING_THUMBPRINT` | Hex thumbprint, with or without spaces | `Get-PfxCertificate` or certificate inventory |
| `TAURI_UPDATER_SIGNATURE` | Tauri updater signature text | Release signing service; use this or `TAURI_SIGNING_PRIVATE_KEY` |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri signer private key text | Release signing service; alternative to updater signature |

`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is also required when the configured
private key is password-protected. Never commit any of these values.

Trigger **Actions -> Codex Windows Preview -> Run workflow**, choose `preview`
or `internal`, and record the GitHub run id. The workflow imports the PFX into
`Cert:\CurrentUser\My`, runs `scripts/sign-release.ps1`, then runs the complete
`scripts/verify-codex-release.ps1` gate before artifact upload.

Expected verification on the runner:

```powershell
Get-AuthenticodeSignature .\path\to\artifact.msi | Select-Object Status,SignerCertificate
signtool.exe verify /pa /all .\path\to\artifact.msi
Get-Content .\artifacts\codex-release\preview\0.1.0\manifest.json | ConvertFrom-Json
```

`Status` must be `Valid`, the signer thumbprint must equal
`MISSION_CONTROL_SIGNING_THUMBPRINT`, and `artifactSha256` must be identical
in `manifest.json`, `provenance.json`, and `sbom.json`. The gate report must
show the real signing status; a missing secret intentionally reports
`signingStatus=deferred-ci` and uploads nothing.

Failure handling: inspect the retained `artifacts/release-gate/<run-id>/steps`
logs. Correct the secret format, certificate chain, or Tauri signing input and
rerun with a new run id. To roll back signing, revoke or remove the signing
secrets and rerun the workflow; it must return to `deferred-ci` and
`releaseReady=false`.

## 2. Authorized real Provider invocation

Before execution, review the ten controlled task categories in
`docs/preview/codex-task-matrix.md`, confirm the model and token/time caps, and
approve the printed cost and scope summary. The command is fail-closed by
default:

```powershell
pwsh -File .\scripts\run-real-invocation.ps1 -InvocationTransport app-server -OutputRoot .\artifacts\real-invocation -RunId <run-id> -TokenLimit 200 -DurationLimitSeconds 120
```

For a narrowly authorized run, pass the allowlisted category explicitly. The
Round 8 authorization is limited to category 01:

```powershell
pwsh -File .\scripts\run-real-invocation.ps1 -TaskCategories readonly_explanation -InvocationTransport exec -OutputRoot .\artifacts\real-invocation -RunId <run-id> -TokenLimit 20000 -DurationLimitSeconds 600
```

Expected output includes `real invocation deferred` and a reminder to obtain
explicit approval. No Provider call occurs. After explicit authorization, set
the user-owned `CODEX_REAL_INVOCATION_COMMAND` and run:

```powershell
pwsh -File .\scripts\run-real-invocation.ps1 -IAuthorizeRealInvocation -InvocationTransport app-server -OutputRoot .\artifacts\real-invocation -RunId <run-id> -TokenLimit 200 -DurationLimitSeconds 120
```

The authorized Round 8 form adds `-TaskCategories readonly_explanation` and
uses the user-approved `-TokenLimit 20000 -DurationLimitSeconds 600` values.
The report must serialize `taskCategories` as `["readonly_explanation"]`.

```powershell
pwsh -File .\scripts\run-real-invocation.ps1 -IAuthorizeRealInvocation -TaskCategories readonly_explanation -InvocationTransport exec -OutputRoot .\artifacts\real-invocation -RunId <run-id> -TokenLimit 20000 -DurationLimitSeconds 600
```

### Invocation transport

Use `-InvocationTransport app-server` for the authorized path (this is the
default). The app-server transport consumes `thread/tokenUsage/updated` while
the turn is running and, when the cumulative usage reaches `-TokenLimit`,
sends `turn/interrupt`, terminates the invocation process, and records
`budgetExceeded=true` before returning a non-zero result. Every report and gate
report must include `invocationTransport` with the value `app-server` or
`exec`.

Approval-capable categories and any run that may receive a server approval
request **must use app-server**. This includes requests such as
`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`,
`item/permissions/requestApproval`, and `item/tool/requestUserInput`. Verify
the selected transport explicitly when reviewing the report:

```powershell
pwsh -File .\scripts\run-real-invocation.ps1 -IAuthorizeRealInvocation -TaskCategories approval_required_action -InvocationTransport app-server -OutputRoot .\artifacts\real-invocation -RunId <run-id> -TokenLimit 20000 -DurationLimitSeconds 600
```

`-InvocationTransport exec` is retained only for tasks that do not require
approval handling and where post-hoc budget enforcement is acceptable. Exec
cannot observe usage mid-run: it checks usage only after `turn.completed`.
When that final usage exceeds the limit, the command exits non-zero and the
report must set `budgetExceeded=true` and visibly include
`violationReason="post-hoc violation (exec transport cannot abort mid-run)"`.
Do not treat such a report as compliant evidence. The archived Round 8
`r8-real-readonly-20260828` report is explicitly backfilled with
`invocationTransport="exec"`; no other historical fields or event evidence are
rewritten.

The harness applies a read-only sandbox, declines approval requests by
default, enforces the token/time limits and a Windows Job Object, and writes
redacted `events.jsonl` plus `real-invocation-report.json` under
`artifacts/real-invocation/<run-id>/`. Scan that directory with the repository
secret gate before accepting it. Upgrade `realInvocation` to
`authorized-real` only when the report says so, contains version/model,
duration and token usage, includes approval and restart/resume evidence when
the selected categories require those flows, and the scan passes. For a
narrowly scoped category, `approvalSampleCaptured=false` or
`restartResumeSample=false` is valid when that flow is not required, provided
the report records the value explicitly. A timeout, non-zero Provider exit,
or secret finding leaves the value `deferred`.

## 3. Clean VM verification

Provision a new Windows 11 or Windows Server 2022+ VM with WebView2 and no
developer toolchain. Copy the production-signed MSI to the VM. The readiness
check (which never claims a simulated host is clean) is:

```powershell
pwsh -File .\scripts\verify-clean-vm.ps1 -MsiPath C:\staging\mission-control.msi -VmName <fresh-vm-name> -OutputPath .\artifacts\release-gate\<run-id>\clean-vm-report.json
```

The script reports `BLOCKED` unless an external VM runner is wired. With an
approved runner, add `-Execute`; the report must have `status=pass` and all
nine checks pass:

1. VM prerequisites (Windows version, WebView2, no toolchain)
2. Silent MSI install
3. `supervisor.ready`
4. Mission Control named pipe
5. `MissionControl/DatabaseKey/*` credential
6. Ledger creation and SQLite `quick_check`
7. First-launch UI connected
8. Uninstall residue check
9. Update replacement and rollback point

Expected output is `clean-vm status=pass` and a JSON report at the supplied
`-OutputPath`. A blocked or failed check is not evidence; fix the VM or runner,
provision a fresh VM, and rerun. Archive the passing report under
`artifacts/release-gate/<run-id>/clean-vm-report.json`.

## 4. Final release decision

From the recorded gate run, verify:

```powershell
$gate = Get-Content .\artifacts\release-gate\<run-id>\codex-release-gate.json -Raw | ConvertFrom-Json
$gate.releaseDecision
$gate.releaseReady
$gate.signingStatus
$gate.realInvocation
```

For `oss-personal`, release is ready when the full gate has zero failures and
the real invocation report is valid `authorized-real` evidence. A missing
production signature and clean-VM report must appear only in
`optionalEnhancements`. For `commercial`, release is permitted only when the
full gate has zero failures, signing is backed by the production certificate,
the real invocation report is valid, the controlled evidence set is ready,
and the clean VM report is `PASS`.
