[CmdletBinding()]
param(
    [string] $EvidenceRoot = (Join-Path (Get-Location) 'artifacts\codex-matrix\evidence'),
    [string] $OutputPath = (Join-Path (Get-Location) 'artifacts\codex-matrix\codex-matrix-report.json'),
    [switch] $FixtureMode
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$categories = @(
    'readonly_explanation',
    'single_file_bug_fix',
    'multi_file_feature',
    'test_failure_repair',
    'approval_required_action',
    'dirty_workspace',
    'long_context_compression',
    'safe_pause',
    'restart_recovery',
    'evidence_export_redaction'
)

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string] $Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash([IO.File]::ReadAllBytes($Path))
    } finally {
        $sha.Dispose()
    }
    return ([BitConverter]::ToString($digest) -replace '-', '').ToLowerInvariant()
}

function Get-Sha256Text {
    param([Parameter(Mandatory)][string] $Value)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Value))) -replace '-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Parse-UtcTimestamp {
    param([Parameter(Mandatory)][object] $Value, [Parameter(Mandatory)][string] $Context)
    try {
        if ($Value -is [DateTime]) {
            $parsed = [DateTime]$Value
        } else {
            $parsed = [DateTime]::Parse([string]$Value, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind)
        }
        if ($parsed.Kind -eq [DateTimeKind]::Unspecified) { throw 'timestamp has no UTC offset' }
        return $parsed.ToUniversalTime()
    } catch {
        throw "TIMESTAMP_INVALID_$Context"
    }
}

function New-FixtureEvidence {
    param([Parameter(Mandatory)][string] $Root)
    if ((Test-Path -LiteralPath $Root -PathType Container) -and @(Get-ChildItem -LiteralPath $Root -File -Recurse -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'EVIDENCE_EXISTS: fixture evidence is immutable; choose a new EvidenceRoot.'
    }
    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $baseTime = [DateTime]::UtcNow
    for ($index = 0; $index -lt $categories.Count; $index += 1) {
        $category = $categories[$index]
        $taskId = 'codex-matrix-{0:d2}' -f ($index + 1)
        $sourceHash = Get-Sha256Text -Value ("controlled-fixture|$taskId|$category")
        $evidence = [ordered]@{
            schema = 'mission-control-codex-task-result-v1'
            taskId = $taskId
            category = $category
            mode = 'controlled'
            provider = 'codex'
            realInvocation = 'deferred'
            collectionSource = 'controlled-fixture'
            sourceSha256 = $sourceHash
            collectedAtUtc = $baseTime.AddSeconds($index).ToString('o')
            status = 'passed'
            modelVersion = 'fake-codex-app-server-v1'
            loadoutFingerprint = 'fixture-loadout-v1'
            contractFingerprint = 'fixture-contract-v1'
            drivingMode = if ($category -eq 'readonly_explanation') { 'read_only' } else { 'controlled' }
            events = @(
                [ordered]@{ sequence = 1; kind = 'agent_run_started'; capturedAtUtc = $baseTime.AddSeconds($index).ToString('o') },
                [ordered]@{ sequence = 2; kind = 'evidence_recorded'; capturedAtUtc = $baseTime.AddSeconds($index).AddMilliseconds(10).ToString('o') },
                [ordered]@{ sequence = 3; kind = 'agent_run_completed'; capturedAtUtc = $baseTime.AddSeconds($index).AddMilliseconds(20).ToString('o') }
            )
            approvals = @()
            diff = [ordered]@{ status = if ($category -in @('readonly_explanation', 'evidence_export_redaction')) { 'none' } else { 'reviewed' }; paths = @('fixture/redacted-path') }
            validation = [ordered]@{ passed = $true; commands = @('fixture-check') }
            recovery = [ordered]@{ status = if ($category -in @('safe_pause', 'restart_recovery')) { 'verified' } else { 'not_required' }; checkpointId = if ($category -eq 'restart_recovery') { 'fixture-checkpoint-01' } else { $null } }
            userUnderstandingSeconds = 6 + $index
            defectIds = @()
            secretsPresent = $false
            sourceRedacted = $true
        }
        $path = Join-Path $Root ('{0:d2}-{1}.json' -f ($index + 1), $category)
        $evidence | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $path -Encoding UTF8
    }
}

if (Test-Path -LiteralPath $OutputPath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite an existing matrix report.' }
if ($FixtureMode) { New-FixtureEvidence -Root $EvidenceRoot }
if (-not (Test-Path -LiteralPath $EvidenceRoot -PathType Container)) { throw 'EVIDENCE_ROOT_MISSING' }

$files = @(Get-ChildItem -LiteralPath $EvidenceRoot -Filter '*.json' -File | Sort-Object Name)
if ($files.Count -ne $categories.Count) { throw "MATRIX_COUNT_INVALID: expected $($categories.Count), found $($files.Count)." }

$records = @()
$p0p1 = @()
$taskIds = [System.Collections.Generic.HashSet[string]]::new()
$collectedTimestamps = [System.Collections.Generic.HashSet[string]]::new()
$lastCollectedTimestamp = $null
foreach ($file in $files) {
    $record = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($field in @('schema', 'taskId', 'category', 'mode', 'provider', 'realInvocation', 'collectionSource', 'sourceSha256', 'collectedAtUtc', 'status', 'modelVersion', 'loadoutFingerprint', 'contractFingerprint', 'drivingMode', 'events', 'approvals', 'diff', 'validation', 'recovery', 'userUnderstandingSeconds', 'defectIds', 'secretsPresent', 'sourceRedacted')) {
        if (-not ($record.PSObject.Properties.Name -contains $field)) { throw "EVIDENCE_SCHEMA_MISSING_$field in $($file.Name)" }
    }
    if (-not $taskIds.Add([string]$record.taskId)) { throw "TASK_ID_DUPLICATE in $($file.Name)" }
    if ([string]::IsNullOrWhiteSpace([string]$record.taskId)) { throw "TASK_ID_MISSING in $($file.Name)" }
    if ($record.schema -ne 'mission-control-codex-task-result-v1') { throw "EVIDENCE_SCHEMA_INVALID in $($file.Name)" }
    if ($record.mode -ne 'controlled' -or $record.provider -ne 'codex' -or $record.realInvocation -ne 'deferred') { throw "REAL_INVOCATION_NOT_DEFERRED in $($file.Name)" }
    if ([string]::IsNullOrWhiteSpace([string]$record.collectionSource) -or [string]$record.collectionSource -eq 'unknown') { throw "COLLECTION_SOURCE_MISSING in $($file.Name)" }
    if ([string]$record.sourceSha256 -notmatch '^[a-f0-9]{64}$') { throw "SOURCE_HASH_INVALID in $($file.Name)" }
    $collectedTimestamp = Parse-UtcTimestamp -Value $record.collectedAtUtc -Context $file.Name
    if (-not $collectedTimestamps.Add($collectedTimestamp.ToString('o'))) { throw "COLLECTED_TIMESTAMP_DUPLICATE in $($file.Name)" }
    if ($null -ne $lastCollectedTimestamp -and $collectedTimestamp -le $lastCollectedTimestamp) { throw "COLLECTED_TIMESTAMP_NOT_MONOTONIC in $($file.Name)" }
    if ($collectedTimestamp -gt [DateTime]::UtcNow.AddMinutes(5)) { throw "COLLECTED_TIMESTAMP_IN_FUTURE in $($file.Name)" }
    $lastCollectedTimestamp = $collectedTimestamp
    if ($record.status -ne 'passed') { throw "MATRIX_RESULT_NOT_PASSED in $($file.Name)" }
    if ($record.secretsPresent -ne $false -or $record.sourceRedacted -ne $true) { throw "EVIDENCE_NOT_REDACTED in $($file.Name)" }
    if ($null -eq $record.validation -or $record.validation.passed -ne $true) { throw "VALIDATION_NOT_PASSED in $($file.Name)" }
    $serialized = $record | ConvertTo-Json -Depth 16 -Compress
    if ($serialized -match '(?i)BEGIN (?:RSA |EC )?PRIVATE KEY|Bearer\s+[A-Za-z0-9._~-]+|mission-control-fixture-secret|providerPayload|rawSource|cookie|authorization') { throw "EVIDENCE_SECRET_OR_RAW_PAYLOAD in $($file.Name)" }
    $events = @($record.events)
    if ($events.Count -eq 0) { throw "EVIDENCE_EVENTS_EMPTY in $($file.Name)" }
    $expected = 1
    $previousEventTime = $null
    foreach ($event in $events) {
        foreach ($field in @('sequence', 'kind', 'capturedAtUtc')) { if (-not ($event.PSObject.Properties.Name -contains $field)) { throw "EVENT_SCHEMA_MISSING_$field in $($file.Name)" } }
        if ([int]$event.sequence -ne $expected) { throw "EVIDENCE_SEQUENCE_INVALID in $($file.Name)" }
        $eventTime = Parse-UtcTimestamp -Value $event.capturedAtUtc -Context $file.Name
        if ($null -ne $previousEventTime -and $eventTime -le $previousEventTime) { throw "EVENT_TIMESTAMP_NOT_MONOTONIC in $($file.Name)" }
        if ($eventTime -gt [DateTime]::UtcNow.AddMinutes(5)) { throw "EVENT_TIMESTAMP_IN_FUTURE in $($file.Name)" }
        $previousEventTime = $eventTime
        $expected += 1
    }
    $p0p1 += @($record.defectIds | Where-Object { [string]$_ -match '^P[01](?:\b|[-_:])' })
    $records += [ordered]@{
        taskId = [string]$record.taskId
        category = [string]$record.category
        status = [string]$record.status
        collectionSource = [string]$record.collectionSource
        collectedAtUtc = [string]$record.collectedAtUtc
        evidenceFile = $file.Name
        evidenceSha256 = Get-Sha256Hex -Path $file.FullName
        userUnderstandingSeconds = [int]$record.userUnderstandingSeconds
        defectIds = @($record.defectIds | ForEach-Object { [string]$_ })
    }
}

$actualCategories = @($records | ForEach-Object { $_.category })
$missing = @($categories | Where-Object { $_ -notin $actualCategories })
$duplicates = @($actualCategories | Group-Object | Where-Object Count -gt 1)
if ($missing.Count -gt 0 -or $duplicates.Count -gt 0) { throw 'MATRIX_CATEGORIES_INVALID' }
if ($p0p1.Count -gt 0) { throw "P0_P1_BLOCKING_DEFECTS: $($p0p1 -join ', ')" }

$parent = Split-Path -Parent $OutputPath
New-Item -ItemType Directory -Force -Path $parent | Out-Null
$report = [ordered]@{
    schema = 'mission-control-codex-matrix-report-v1'
    mode = 'controlled'
    realInvocation = 'deferred'
    evidenceGrade = 'controlled-deferred'
    releaseReady = $false
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    totalTasks = $records.Count
    categories = $actualCategories
    p0p1Count = $p0p1.Count
    allEvidenceRedacted = $true
    evidence = $records
}
$report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
Write-Output $OutputPath
