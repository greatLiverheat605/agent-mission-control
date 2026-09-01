[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $GateRoot,
    [string] $MatrixPath = '',
    [string] $BundlePath = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string] $Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes($Path))) -replace '-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

if (-not (Test-Path -LiteralPath $GateRoot -PathType Container)) { throw 'GATE_ROOT_MISSING' }
$gateReportPath = Join-Path $GateRoot 'codex-release-gate.json'
if (-not (Test-Path -LiteralPath $gateReportPath -PathType Leaf)) { throw 'GATE_REPORT_MISSING' }
if ([string]::IsNullOrWhiteSpace($MatrixPath)) { $MatrixPath = Join-Path $GateRoot 'finding-matrix.json' }
if ([string]::IsNullOrWhiteSpace($BundlePath)) { $BundlePath = Join-Path $GateRoot 'review-bundle.json' }
if (Test-Path -LiteralPath $MatrixPath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite finding matrix.' }
if (Test-Path -LiteralPath $BundlePath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite review bundle.' }

$gate = Get-Content -LiteralPath $gateReportPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($gate.schema -ne 'mission-control-codex-release-gate-v1') { throw 'GATE_SCHEMA_INVALID' }
$repositoryRoot = Split-Path -Parent $PSScriptRoot

$artifactNames = @('codex-release-gate.json', 'crash-matrix.json', 'soak.json', 'task-matrix-report.json', 'preview-report.json', 'sbom.json')
$artifactRefs = [System.Collections.Generic.List[object]]::new()
foreach ($name in $artifactNames) {
    $path = Join-Path $GateRoot $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "EVIDENCE_ARTIFACT_MISSING_$name" }
    $artifactRefs.Add([ordered]@{ name = $name; sha256 = Get-Sha256Hex -Path $path })
}
foreach ($name in @('test-signing-chain.json', 'real-invocation-report.json', 'events.jsonl', 'clean-vm-report.json')) {
    $path = Join-Path $GateRoot $name
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        $artifactRefs.Add([ordered]@{ name = $name; sha256 = Get-Sha256Hex -Path $path })
    }
}
$realReportProperty = $gate.artifacts.PSObject.Properties['realInvocationReport']
if ($null -ne $realReportProperty -and -not [string]::IsNullOrWhiteSpace([string]$realReportProperty.Value)) {
    $realReportPath = [string]$realReportProperty.Value
    $gateLocalReportPath = Join-Path $GateRoot 'real-invocation-report.json'
    if ((Test-Path -LiteralPath $gateLocalReportPath -PathType Leaf) -and (Test-Path -LiteralPath $realReportPath -PathType Leaf) -and
        ((Get-Sha256Hex -Path $gateLocalReportPath) -ne (Get-Sha256Hex -Path $realReportPath))) {
        throw 'same-name evidence mismatch: gate real-invocation-report.json differs from referenced report'
    }
    if ((Test-Path -LiteralPath $realReportPath -PathType Leaf) -and (@($artifactRefs | Where-Object { $_.name -eq 'real-invocation-report.json' }).Count -eq 0)) {
        $artifactRefs.Add([ordered]@{ name = 'real-invocation-report.json'; sha256 = Get-Sha256Hex -Path $realReportPath })
        $realReport = Get-Content -LiteralPath $realReportPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $evidenceName = [string]$realReport.evidencePath
        if (-not [string]::IsNullOrWhiteSpace($evidenceName)) {
            $evidencePath = Join-Path (Split-Path -Parent $realReportPath) $evidenceName
            $gateEvidencePath = Join-Path $GateRoot (Split-Path -Leaf $evidenceName)
            if ((Test-Path -LiteralPath $evidencePath -PathType Leaf) -and (Test-Path -LiteralPath $gateEvidencePath -PathType Leaf) -and
                ((Get-Sha256Hex -Path $evidencePath) -ne (Get-Sha256Hex -Path $gateEvidencePath))) {
                throw "same-name evidence mismatch: $evidenceName differs from referenced evidence"
            }
            if ((Test-Path -LiteralPath $evidencePath -PathType Leaf) -and (@($artifactRefs | Where-Object { $_.name -eq $evidenceName }).Count -eq 0)) {
                $artifactRefs.Add([ordered]@{ name = $evidenceName; sha256 = Get-Sha256Hex -Path $evidencePath })
            }
        }
    }
}
foreach ($name in @('store-before.json', 'store-after.json', 'cleanup-steps.json')) {
    $path = Join-Path $repositoryRoot "artifacts\release-gate\r7-machine-hygiene-20260828\$name"
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        $artifactRefs.Add([ordered]@{ name = "r7-machine-hygiene-20260828/$name"; sha256 = Get-Sha256Hex -Path $path })
    }
}

$findings = @(
    [ordered]@{
        findingId = 'F-001'; priority = 'P1'; title = 'Codex process tree termination';
        codePaths = @('crates/supervisor/src/process_tree.rs', 'crates/adapter-codex/src/process.rs');
        testCommands = @('cargo test -p mission-supervisor process_tree --offline', 'cargo test -p adapter-codex --tests --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'soak.json'); remainingDeferred = @('real Codex process invocation')
    },
    [ordered]@{
        findingId = 'F-002'; priority = 'P1'; title = 'Safe pause and force terminate token';
        codePaths = @('crates/supervisor/src/pause.rs', 'apps/desktop/src-tauri/src/supervisor_bridge.rs');
        testCommands = @('cargo test -p mission-supervisor pause --offline', 'cargo test -p mission-control-desktop --test supervisor_bridge --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'soak.json'); remainingDeferred = @('real desktop/agent process confirmation')
    },
    [ordered]@{
        findingId = 'F-003'; priority = 'P1'; title = 'Approval resolution and policy gate';
        codePaths = @('crates/policy/src/approval.rs', 'crates/supervisor/src/mission_actor.rs', 'apps/desktop/src/features/approval/ApprovalDock.tsx');
        testCommands = @('cargo test -p mission-policy approval --offline', 'cargo test -p mission-supervisor --test approval --offline', 'node --test tests/contract/protocol-bindings.test.mjs');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @('GLM approval-flow re-review')
    },
    [ordered]@{
        findingId = 'F-004'; priority = 'P1'; title = 'Recovery lifecycle and explicit decision';
        codePaths = @('crates/domain/src/event.rs', 'crates/supervisor/src/mission_actor.rs', 'apps/desktop/src/features/recovery/RecoveryReviewPanel.tsx');
        testCommands = @('cargo test -p mission-supervisor recovery --offline', 'node --test tests/recovery/recovery-matrix.test.mjs');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'task-matrix-report.json'); remainingDeferred = @('clean-VM restart evidence')
    },
    [ordered]@{
        findingId = 'F-005'; priority = 'P1'; title = 'IPC deadlines heartbeat and watchdog ordering';
        codePaths = @('apps/desktop/src-tauri/src/supervisor_bridge.rs', 'crates/supervisor/src/ipc.rs', 'crates/supervisor/src/mission_service.rs');
        testCommands = @('cargo test -p mission-supervisor ipc --offline', 'cargo test -p mission-supervisor --test ipc_smoke --offline', 'cargo test -p mission-control-desktop --test supervisor_bridge --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'soak.json'); remainingDeferred = @('real cold-start timing on clean VM')
    },
    [ordered]@{
        findingId = 'F-006'; priority = 'P1'; title = 'Truthful Codex executable capability probe';
        codePaths = @('crates/adapter-codex/src/process.rs', 'crates/adapter-codex/src/exec_probe.rs');
        testCommands = @('cargo test -p adapter-codex --test process --offline', 'cargo test -p adapter-codex --test exec_probe --offline');
        evidenceLevel = 'production-code-controlled-executable'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-controlled';
        evidenceRefs = @('soak.json', 'task-matrix-report.json'); remainingDeferred = @('real Codex executable and version/hash evidence')
    },
    [ordered]@{
        findingId = 'F-007'; priority = 'P1'; title = 'Release evidence fail-closed gate';
        codePaths = @('scripts/run-codex-soak.ps1', 'scripts/run-crash-matrix.ps1', 'scripts/summarize-codex-matrix.ps1', 'scripts/summarize-codex-preview.ps1', 'scripts/verify-codex-release.ps1');
        testCommands = @('powershell -File scripts/verify-codex-release.ps1 -SkipPackageBuild', 'node --test tests/release/codex-release-gate.test.mjs');
        evidenceLevel = 'controlled-deferred'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-controlled-deferred';
        evidenceRefs = @('codex-release-gate.json', 'crash-matrix.json', 'soak.json', 'task-matrix-report.json', 'preview-report.json'); remainingDeferred = @('real Provider evidence', 'signed release evidence')
    },
    [ordered]@{
        findingId = 'F-008'; priority = 'P1'; title = 'Ledger append and event consumer safe pause';
        codePaths = @('crates/supervisor/src/mission_service.rs', 'crates/ledger/src/lib.rs');
        testCommands = @('cargo test -p mission-supervisor --test restart_replay --offline', 'cargo test -p mission-supervisor --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'task-matrix-report.json'); remainingDeferred = @('fault injection against production ledger backend')
    },
    [ordered]@{
        findingId = 'F-009'; priority = 'P2'; title = 'Recovery UI backend accepted/error semantics';
        codePaths = @('apps/desktop/src/App.tsx', 'apps/desktop/src/features/recovery/RecoveryReviewPanel.tsx', 'crates/supervisor/src/mission_service.rs');
        testCommands = @('node --test tests/recovery/recovery-matrix.test.mjs', 'cargo test -p mission-supervisor recovery --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'task-matrix-report.json'); remainingDeferred = @('GLM recovery UX re-review')
    },
    [ordered]@{
        findingId = 'F-030'; priority = 'P1'; title = 'Codex app-server initialize uses the official clientInfo/capabilities shape';
        codePaths = @('crates/adapter-codex/src/process.rs');
        testCommands = @('cargo test -p adapter-codex --test process --offline', 'node --test tests/security/codex-protocol-consistency.test.mjs');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('soak.json', 'task-matrix-report.json'); remainingDeferred = @('real Codex 0.147.0 invocation')
    },
    [ordered]@{
        findingId = 'F-031'; priority = 'P1'; title = 'Codex turn/start delivers the mission goal on the persisted thread';
        codePaths = @('crates/adapter-codex/src/process.rs', 'fixtures/agents/bin/fake-codex-app-server.ps1');
        testCommands = @('cargo test -p adapter-codex --test process --offline', 'node --test tests/soak/three-mission-codex.test.mjs');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('soak.json', 'task-matrix-report.json'); remainingDeferred = @('real Codex goal delivery')
    },
    [ordered]@{
        findingId = 'F-032'; priority = 'P1'; title = 'Codex turn/interrupt carries threadId/turnId and records send failures';
        codePaths = @('crates/adapter-codex/src/process.rs');
        testCommands = @('cargo test -p adapter-codex --test process --offline', 'cargo test -p adapter-codex --test app_server --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'soak.json'); remainingDeferred = @('real Codex interrupt acknowledgement')
    },
    [ordered]@{
        findingId = 'F-033'; priority = 'P1'; title = 'Codex event normalization and fixtures use slash/camelCase app-server names';
        codePaths = @('crates/adapter-codex/src/normalize.rs', 'crates/adapter-codex/src/native.rs', 'fixtures/agents/bin/fake-codex-app-server.ps1');
        testCommands = @('cargo test -p adapter-codex --test fixtures --offline', 'node --test tests/security/codex-protocol-consistency.test.mjs tests/soak/three-mission-codex.test.mjs');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('soak.json', 'task-matrix-report.json'); remainingDeferred = @('real Codex event corpus')
    },
    [ordered]@{
        findingId = 'F-034'; priority = 'P1'; title = 'Codex recovery uses thread/resume with a persisted threadId';
        codePaths = @('crates/adapter-codex/src/process.rs', 'crates/supervisor/src/mission_service.rs', 'crates/supervisor/src/mission_actor.rs');
        testCommands = @('cargo test -p mission-supervisor recovery --offline', 'cargo test -p adapter-codex --test process --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('crash-matrix.json', 'task-matrix-report.json'); remainingDeferred = @('real Codex restart/resume')
    },
    [ordered]@{
        findingId = 'F-035'; priority = 'P1'; title = 'Codex server requests flow through approval events and id-matched responses';
        codePaths = @('crates/adapter-codex/src/app_server.rs', 'crates/adapter-codex/src/process.rs', 'crates/supervisor/src/mission_service.rs');
        testCommands = @('cargo test -p adapter-codex --test app_server --offline', 'cargo test -p mission-supervisor --test approval --offline', 'node --test tests/soak/three-mission-codex.test.mjs');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('soak.json', 'task-matrix-report.json'); remainingDeferred = @('real Codex approval request corpus')
    },
    [ordered]@{
        findingId = 'F-010'; priority = 'P2'; title = 'Mission creation checks durable existence before atomic event append';
        codePaths = @('crates/supervisor/src/mission_service.rs', 'crates/ledger/src/sqlcipher.rs');
        testCommands = @('cargo test -p mission-supervisor duplicate_create_is_rejected_before_any_ledger_append --offline', 'cargo test -p mission-ledger --test sqlcipher append_batch_rolls_back_all_events_when_a_later_event_fails --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-015'; priority = 'P2'; title = 'Adapter and ledger redaction use replace-all and normalized secret keys';
        codePaths = @('crates/adapter-codex/src/normalize.rs', 'crates/ledger/src/redaction.rs');
        testCommands = @('cargo test -p adapter-codex --tests --offline', 'cargo test -p mission-ledger --test redaction --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-016'; priority = 'P2'; title = 'Mission budget tracker consumes Codex token usage and pauses safely';
        codePaths = @('crates/supervisor/src/mission_service.rs', 'crates/supervisor/src/mission_actor.rs');
        testCommands = @('cargo test -p mission-supervisor budget --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('soak.json', 'task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-017'; priority = 'P2'; title = 'App-server line reads are bounded before newline consumption';
        codePaths = @('crates/adapter-codex/src/app_server.rs');
        testCommands = @('cargo test -p adapter-codex --test app_server --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-018'; priority = 'P2'; title = 'Ledger integrity and replay recompute persisted payload hashes';
        codePaths = @('crates/ledger/src/sqlcipher.rs');
        testCommands = @('cargo test -p mission-ledger sqlcipher::tests::integrity_and_replay_reject_payload_tampering_with_location --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-019'; priority = 'P2'; title = 'Mission archive/delete/export lifecycle is wired through IPC and UI';
        codePaths = @('crates/supervisor/src/ipc.rs', 'crates/supervisor/src/mission_service.rs', 'apps/desktop/src/features/storage');
        testCommands = @('cargo test -p mission-ledger --test data_lifecycle --offline', 'npm.cmd run test --workspace @mission-control/desktop -- --run');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'APPROVAL-DERIVATION'; priority = 'P2'; title = 'Approval action class and contract version derive from protocol method and mission context';
        codePaths = @('crates/adapter-codex/src/process.rs');
        testCommands = @('cargo test -p adapter-codex --test process --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-025a'; priority = 'P3'; title = 'Force termination records failure without entering Terminated when kill fails';
        codePaths = @('crates/supervisor/src/mission_actor.rs', 'crates/supervisor/src/process_tree.rs');
        testCommands = @('cargo test -p mission-supervisor failed_process_kill_keeps_force_state_and_records_failure --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-025b'; priority = 'P3'; title = 'Malformed approval replay fails closed with event sequence';
        codePaths = @('crates/supervisor/src/mission_actor.rs');
        testCommands = @('cargo test -p mission-supervisor malformed_approval_event_fails_recovery_with_sequence --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-025c'; priority = 'P3'; title = 'Credential read captures GetLastError immediately after CredReadW failure';
        codePaths = @('crates/ledger/src/key_store.rs');
        testCommands = @('cargo test -p mission-ledger --lib key_store --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-026a'; priority = 'P3'; title = 'Emergency pause and command palette trap keyboard focus';
        codePaths = @('apps/desktop/src/features/mission/EmergencyPause.tsx', 'apps/desktop/src/interaction/CommandPalette.tsx');
        testCommands = @('npm.cmd run test --workspace @mission-control/desktop -- --run src/interaction/interaction.test.tsx');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-026b'; priority = 'P3'; title = 'Mission scene probes WebGL2 before selecting the 2D fallback';
        codePaths = @('apps/desktop/src/scene/MissionScene.tsx');
        testCommands = @('npm.cmd run test --workspace @mission-control/desktop -- --run src/scene/MissionScene.test.tsx');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-027'; priority = 'P3'; title = 'Retention project usage groups missions by MissionCreated project_root';
        codePaths = @('crates/ledger/src/retention.rs', 'crates/supervisor/src/mission_service.rs');
        testCommands = @('cargo test -p mission-ledger --test data_lifecycle retention_project_usage_groups_missions_by_project_root --offline');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-029'; priority = 'P3'; title = 'Windows setup documents complete Strawberry Perl prerequisite for vendored OpenSSL';
        codePaths = @('docs/development/windows-setup.md');
        testCommands = @('node --test tests/release/codex-release-gate.test.mjs');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('task-matrix-report.json'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-036'; priority = 'P1'; title = 'Invocation token limits are enforced with streaming app-server interruption and exec post-hoc marking';
        codePaths = @('scripts/run-real-invocation.ps1');
        testCommands = @('node --test tests/release/codex-release-gate.test.mjs --test-name-pattern="budget"');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('scripts/run-real-invocation.ps1', 'task-matrix-report.json'); remainingDeferred = @('future authorized app-server run')
    },
    [ordered]@{
        findingId = 'F-037'; priority = 'P3'; title = 'Same-name evidence copies are hash-consistent or fail closed';
        codePaths = @('scripts/verify-codex-release.ps1', 'scripts/build-p1-finding-matrix.ps1');
        testCommands = @('node --test tests/release/codex-release-gate.test.mjs --test-name-pattern="same-name evidence"');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('scripts/verify-codex-release.ps1', 'artifacts/release-gate/r8-gate-final-20260828/superseded'); remainingDeferred = @()
    },
    [ordered]@{
        findingId = 'F-038'; priority = 'P3'; title = 'Invocation reports and release gates explicitly label exec or app-server transport';
        codePaths = @('scripts/run-real-invocation.ps1', 'scripts/verify-codex-release.ps1', 'docs/release/formal-release-runbook.md');
        testCommands = @('node --test tests/release/codex-release-gate.test.mjs --test-name-pattern="transport"');
        evidenceLevel = 'production-code-offline'; productionPathEvidence = $true; realProviderEvidence = $false; status = 'verified-offline';
        evidenceRefs = @('scripts/run-real-invocation.ps1', 'scripts/verify-codex-release.ps1', 'docs/release/formal-release-runbook.md'); remainingDeferred = @()
    }
)

$protocolFindingIds = @('F-030', 'F-031', 'F-032', 'F-033', 'F-034', 'F-035')
$protocolFindings = @($findings | Where-Object { $protocolFindingIds -contains $_.findingId })
$protocolClosureComplete = $protocolFindings.Count -eq $protocolFindingIds.Count -and @($protocolFindings | Where-Object { $_.status -ne 'verified-offline' -or $_.productionPathEvidence -ne $true }).Count -eq 0
$residualItems = @(
    [ordered]@{ id = 'F-004'; title = 'turn/completed terminal events clear recovery after normal completion'; status = 'verified-offline'; evidenceRefs = @('crash-matrix.json', 'task-matrix-report.json') },
    [ordered]@{ id = 'F-021'; title = 'failed state styling and needsResync alert are visible in the desktop UI'; status = 'verified-offline'; evidenceRefs = @('task-matrix-report.json', 'soak.json') },
    [ordered]@{ id = 'SOAK-RESIDUAL'; title = 'three-mission soak is driven by isolated fake app-server sessions with real assertions'; status = 'verified-offline'; evidenceRefs = @('soak.json') }
)
$releaseEngineeringItems = @(
    [ordered]@{ id = 'R4-1'; findingIds = @('F-030', 'F-031', 'F-032', 'F-033', 'F-034', 'F-035'); title = 'Vendored Codex 0.147.0 schema snapshot and drift gate'; status = 'verified-offline'; evidenceRefs = @('tests/security/codex-protocol-consistency.test.mjs', 'fixtures/protocol/codex-schema-0.147.0/snapshot-manifest.json'); remainingDeferred = @('regenerate and review against a future Codex baseline') },
    [ordered]@{ id = 'R4-2'; findingIds = @('F-011', 'F-022'); title = 'Windows release/CI workflow gates, signing defer, fixture selfcheck, unsigned rejection and fresh E2E build'; status = 'verified-offline'; evidenceRefs = @('.github/workflows/windows-release.yml', '.github/workflows/windows-ci.yml'); remainingDeferred = @('CI run with production signing secrets', 'clean VM execution') },
    [ordered]@{ id = 'R4-3'; findingIds = @('F-012'); title = 'Full-lock SBOM checksums and post-sign artifact hash synchronization'; status = 'verified-offline'; evidenceRefs = @('scripts/generate-sbom.ps1', 'scripts/sign-release.ps1', 'scripts/verify-codex-release.ps1'); remainingDeferred = @('production-signed artifact hash evidence') },
    [ordered]@{ id = 'R4-4'; findingIds = @('F-013'); title = 'Synthetic fixture signature is isolated and explicitly labeled'; status = 'verified-offline'; evidenceRefs = @('artifacts/codex-release/preview/0.1.0/unsigned-fixture-synthetic/manifest.json'); remainingDeferred = @() },
    [ordered]@{ id = 'R4-5'; findingIds = @('F-014'); title = 'Secret persistence scan combines corpus and redaction patterns'; status = 'verified-offline'; evidenceRefs = @('scripts/scan-secrets.ps1', 'scripts/security-gate.ps1'); remainingDeferred = @('none') },
    [ordered]@{ id = 'R4-6'; findingIds = @('F-024'); title = 'Release gate substep output and run-id binding are retained'; status = 'verified-offline'; evidenceRefs = @('scripts/verify-codex-release.ps1', 'docs/release/codex-release-checklist.md'); remainingDeferred = @('none') }
)
$round6Items = @(
    [ordered]@{ id = 'R6-A'; blocker = 'signing-credentials'; title = 'TEST self-signed import, sign, verify and hash-chain mechanical check'; status = 'ready-and-paused'; evidenceRefs = @('test-signing-chain.json', 'scripts/test-signing-chain.ps1'); certificateType = 'TEST-self-signed'; productionStatus = 'deferred-ci'; releaseReady = $false },
    [ordered]@{ id = 'R6-B'; blocker = 'real-provider-invocation'; title = 'Fixture-first ten-task real invocation harness with approval and resume samples'; status = 'ready-and-paused'; evidenceRefs = @('real-invocation-report.json', 'events.jsonl', 'scripts/run-real-invocation.ps1'); realInvocation = 'deferred'; releaseReady = $false },
    [ordered]@{ id = 'R6-C'; blocker = 'clean-vm'; title = 'Clean Windows VM install, credential, ledger, UI and rollback verification'; status = 'blocked'; evidenceRefs = @('clean-vm-report.json', 'scripts/verify-clean-vm.ps1', 'docs/release/clean-vm-verification.md'); cleanVm = 'blocked'; releaseReady = $false },
    [ordered]@{ id = 'R6-D1'; blocker = $null; title = 'R1-R5 completed plans archived without moving active handoff plans'; status = 'verified-offline'; evidenceRefs = @('.codex/plans/archive') }
)
$round7Items = @(
    [ordered]@{ id = 'C-1'; title = 'TEST self-signed certificate stores cleaned with exact thumbprint matching'; status = 'verified-offline'; evidenceRefs = @('r7-machine-hygiene-20260828/store-before.json', 'r7-machine-hygiene-20260828/store-after.json', 'r7-machine-hygiene-20260828/cleanup-steps.json') },
    [ordered]@{ id = 'C-2'; title = 'Optional P3 findings F-025a/b/c, F-026a/b, F-027 and F-029 closed'; status = 'verified-offline'; findingIds = @('F-025a', 'F-025b', 'F-025c', 'F-026a', 'F-026b', 'F-027', 'F-029'); evidenceRefs = @('task-matrix-report.json') },
    [ordered]@{ id = 'C-3'; title = 'Formal release runbook matches signing, invocation and clean VM script parameters'; status = 'verified-offline'; evidenceRefs = @('docs/release/formal-release-runbook.md') }
)
$gateInvocationTransportProperty = $gate.PSObject.Properties['invocationTransport']
$gateInvocationTransport = if ($null -eq $gateInvocationTransportProperty) { 'app-server' } else { [string]$gateInvocationTransportProperty.Value }
$round8Items = @(
    [ordered]@{
        id = 'R8-A'; blocker = 'signing-credentials'; title = 'Production certificate signing and Authenticode verification';
        status = if ([string]$gate.signingStatus -eq 'signed-ci') { 'executed' } else { 'awaiting-user' };
        signingStatus = [string]$gate.signingStatus; evidenceRefs = @('codex-release-gate.json')
    },
    [ordered]@{
        id = 'R8-B'; blocker = 'real-provider-invocation'; title = 'Authorized real invocation for the scoped readonly_explanation task';
        status = if ([string]$gate.realInvocation -eq 'authorized-real') { 'executed' } else { 'awaiting-user' };
        realInvocation = [string]$gate.realInvocation; invocationTransport = $gateInvocationTransport; evidenceRefs = @('real-invocation-report.json', 'events.jsonl')
    },
    [ordered]@{
        id = 'R8-C'; blocker = 'clean-vm'; title = 'Clean VM install and rollback verification';
        status = 'awaiting-user'; cleanVm = 'blocked'; evidenceRefs = @('scripts/verify-clean-vm.ps1', 'docs/release/clean-vm-verification.md')
    }
)
$round9Items = @(
    [ordered]@{ id = 'F-036'; title = 'Token limits enforced by app-server streaming interruption or exec post-hoc violation'; status = 'verified-offline'; evidenceRefs = @('scripts/run-real-invocation.ps1', 'tests/release/codex-release-gate.test.mjs'); invocationTransport = @('app-server', 'exec') },
    [ordered]@{ id = 'F-037'; title = 'Evidence hygiene rejects same-name divergent copies and archives superseded reports'; status = 'verified-offline'; evidenceRefs = @('scripts/verify-codex-release.ps1', 'artifacts/release-gate/r8-gate-final-20260828/superseded', 'tests/release/codex-release-gate.test.mjs') },
    [ordered]@{ id = 'F-038'; title = 'Invocation transport is explicit in reports and approval runbook guidance'; status = 'verified-offline'; evidenceRefs = @('docs/release/formal-release-runbook.md', 'artifacts/real-invocation/r8-real-readonly-20260828/real-invocation-report.json') }
)
$round10Items = @(
    [ordered]@{ id = 'F-039'; title = 'Bound evidence remains immutable after transport annotation backfill'; status = 'verified-offline'; evidenceRefs = @('docs/release/evidence-bindings.json', 'scripts/verify-evidence-bindings.ps1', 'artifacts/real-invocation/r8-real-readonly-20260828/transport-annotation.json', 'tests/release/codex-release-gate.test.mjs') }
)
$round12Items = @(
    [ordered]@{ id = 'F-040'; title = 'Codex request parameter shapes are validated against the vendored 0.147.0 schema'; status = 'verified-offline'; evidenceRefs = @('tests/security/codex-protocol-consistency.test.mjs', 'fixtures/agents/bin/fake-codex-app-server.ps1', 'crates/adapter-codex/src/process.rs', 'scripts/run-real-invocation.ps1'); invocationTransport = 'app-server' }
)
$round14Items = @(
    [ordered]@{ id = 'R14-LICENSE'; title = 'MIT license is declared in the repository and Rust/npm manifests'; status = 'verified-offline'; evidenceRefs = @('LICENSE', 'Cargo.toml', 'package.json') },
    [ordered]@{ id = 'R14-CONTENT'; title = 'Generated evidence and local state are ignored while scripts, docs and fixtures remain publishable'; status = 'verified-offline'; evidenceRefs = @('.gitignore', 'fixtures/agents/README.md') },
    [ordered]@{ id = 'R14-AUDIT'; title = 'Pending source, history and persisted secrets receive a report-only open-source audit'; status = 'verified-offline'; evidenceRefs = @('scripts/audit-open-source.ps1', 'open-source-audit.json') },
    [ordered]@{ id = 'R14-DOCS'; title = 'Open-source build, unsigned Windows install, security reporting and disclaimer guidance is published'; status = 'verified-offline'; evidenceRefs = @('README.md', 'SECURITY.md', 'docs/release/formal-release-runbook.md') },
    [ordered]@{ id = 'R14-DISTRIBUTION'; title = 'oss-personal release readiness requires zero failures and valid authorized real evidence'; status = 'verified-offline'; evidenceRefs = @('scripts/release-distribution.ps1', 'tests/release/open-source-go-live.test.mjs') },
    [ordered]@{ id = 'R14-COMMITS'; title = 'Repository changes are prepared as local atomic commits without push'; status = 'prepared-no-push'; evidenceRefs = @(); remainingDeferred = @('user-owned push') }
)
$p1Findings = @($findings | Where-Object { $_.priority -eq 'P1' })
$p1ProductionPathComplete = @($p1Findings | Where-Object { $_.productionPathEvidence -ne $true }).Count -eq 0
$p1RealEvidenceComplete = @($p1Findings | Where-Object { $_.realProviderEvidence -ne $true }).Count -eq 0
$gatePassed = @($gate.gates | Where-Object { $_.status -eq 'fail' }).Count -eq 0 -and $gate.failures.Count -eq 0
$gateDecisionProperty = $gate.PSObject.Properties['releaseDecision']
$gateDecision = if ($null -eq $gateDecisionProperty) { $null } else { [string]$gateDecisionProperty.Value }
$releaseDecision = if ($gateDecision -in @('PASS_INTERNAL_PREVIEW', 'BLOCK_INTERNAL_PREVIEW')) { $gateDecision } elseif ($gatePassed -and $gate.controlledInternalPreview -eq $true -and $protocolClosureComplete) { 'PASS_INTERNAL_PREVIEW' } else { 'BLOCK_INTERNAL_PREVIEW' }
$gateDistributionProperty = $gate.PSObject.Properties['distribution']
$gateDistribution = if ($null -eq $gateDistributionProperty) { 'commercial' } else { [string]$gateDistributionProperty.Value }
$gateReleaseReadyProperty = $gate.PSObject.Properties['releaseReady']
$gateReleaseReady = if ($null -eq $gateReleaseReadyProperty) { $false } else { [bool]$gateReleaseReadyProperty.Value }
$gateOptionalEnhancementsProperty = $gate.PSObject.Properties['optionalEnhancements']
$gateOptionalEnhancements = if ($null -eq $gateOptionalEnhancementsProperty) { @() } else { @($gateOptionalEnhancementsProperty.Value) }
$parent = Split-Path -Parent $MatrixPath
New-Item -ItemType Directory -Force -Path $parent | Out-Null
$matrix = [ordered]@{
    schema = 'mission-control-p1-finding-matrix-v1'
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    gateRunId = (Split-Path -Leaf $GateRoot)
    distribution = $gateDistribution
    releaseDecision = $releaseDecision
    releaseReady = $gateReleaseReady
    formalReleaseBlockedBy = @($gate.formalReleaseBlockedBy)
    optionalEnhancements = @($gateOptionalEnhancements)
    gatePassed = $gatePassed
    controlledInternalPreview = [bool]$gate.controlledInternalPreview
    evidenceStatus = [string]$gate.evidenceStatus
    p1Count = $p1Findings.Count
    p1ProductionPathComplete = $p1ProductionPathComplete
    p1RealEvidenceComplete = $p1RealEvidenceComplete
    protocolClosureComplete = $protocolClosureComplete
    glmReviewEligible = ($releaseDecision -ne 'BLOCK_INTERNAL_PREVIEW')
    sourceRedacted = $true
    artifactRefs = @($artifactRefs)
    findings = @($findings)
    residualItems = @($residualItems)
    releaseEngineeringItems = @($releaseEngineeringItems)
    round6Items = @($round6Items)
    round7Items = @($round7Items)
    round8Items = @($round8Items)
    round9Items = @($round9Items)
    round10Items = @($round10Items)
    round12Items = @($round12Items)
    round14Items = @($round14Items)
    round6Verification = $gate.round6Verification
    remainingDeferred = @($gate.formalReleaseBlockedBy)
}
$matrix | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $MatrixPath -Encoding UTF8

$bundleParent = Split-Path -Parent $BundlePath
New-Item -ItemType Directory -Force -Path $bundleParent | Out-Null
$bundle = [ordered]@{
    schema = 'mission-control-p1-review-bundle-v1'
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    gateRunId = (Split-Path -Leaf $GateRoot)
    distribution = $gateDistribution
    scope = 'read-only production code paths and redacted controlled/deferred evidence'
    releaseDecision = $releaseDecision
    releaseReady = $gateReleaseReady
    formalReleaseBlockedBy = @($gate.formalReleaseBlockedBy)
    optionalEnhancements = @($gateOptionalEnhancements)
    evidenceStatus = [string]$gate.evidenceStatus
    matrixSha256 = Get-Sha256Hex -Path $MatrixPath
    artifacts = @($artifactRefs)
    residualItems = @($residualItems)
    releaseEngineeringItems = @($releaseEngineeringItems)
    round6Items = @($round6Items)
    round7Items = @($round7Items)
    round8Items = @($round8Items)
    round9Items = @($round9Items)
    round10Items = @($round10Items)
    round12Items = @($round12Items)
    round14Items = @($round14Items)
    round6Verification = $gate.round6Verification
    excluded = @('source code secrets', 'certificates and private keys', 'PFX passwords', 'real Provider credentials', 'raw provider payloads', 'user source')
    glmReview = [ordered]@{ eligible = ($releaseDecision -ne 'BLOCK_INTERNAL_PREVIEW'); requiredDecision = 'explicit P1 closure before any release/real-preview plan' }
    findingMatrix = (Split-Path -Leaf $MatrixPath)
}
$bundle | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $BundlePath -Encoding UTF8
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repositoryRoot 'scripts/verify-evidence-bindings.ps1') -BindingsPath (Join-Path $repositoryRoot 'docs/release/evidence-bindings.json') -RepositoryRoot $repositoryRoot -ReviewBundlePath $BundlePath *> (Join-Path $bundleParent 'evidence-bindings-check.log')
if ($LASTEXITCODE -ne 0) { throw "IMMUTABLE_EVIDENCE_BINDINGS_FAILED: $LASTEXITCODE" }
Write-Output $MatrixPath
Write-Output $BundlePath
