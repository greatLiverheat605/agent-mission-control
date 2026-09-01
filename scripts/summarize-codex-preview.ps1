[CmdletBinding()]
param(
    [string] $SamplesPath = (Join-Path (Get-Location) 'artifacts\codex-preview\samples'),
    [string] $OutputPath = (Join-Path (Get-Location) 'artifacts\codex-preview\codex-preview-report.json'),
    [switch] $FixtureMode
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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

function New-FixtureSamples {
    param([Parameter(Mandatory)][string] $Root)
    if ((Test-Path -LiteralPath $Root -PathType Container) -and @(Get-ChildItem -LiteralPath $Root -File -Recurse -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'SAMPLES_EXISTS: fixture samples are immutable; choose a new SamplesPath.'
    }
    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $baseTime = [DateTime]::UtcNow
    for ($index = 1; $index -le 5; $index += 1) {
        $participantId = 'pilot-{0:d2}' -f $index
        $sourceHash = Get-Sha256Text -Value ("controlled-fixture|$participantId|$index")
        $sample = [ordered]@{
            schema = 'mission-control-codex-preview-sample-v1'
            participantId = $participantId
            projectType = @('typescript', 'rust', 'mixed')[(($index - 1) % 3)]
            hardwareProfile = if ($index -le 3) { 'preview-minimum' } else { 'preview-recommended' }
            collectedAtUtc = $baseTime.AddSeconds($index).ToString('o')
            collectionSource = 'controlled-fixture'
            sourceSha256 = $sourceHash
            firstLaunchCompleted = $index -ne 5
            stateRecognitionCorrect = $true
            stateRecognitionSeconds = 5 + (($index - 1) % 5)
            guided = $false
            issueSeverity = $null
            failureReason = if ($index -eq 5) { 'first_launch_timeout' } else { $null }
            telemetryEnabled = $false
            sourceIncluded = $false
            secretIncluded = $false
            redactedPreviewReviewed = $true
            exportConfirmed = $index -eq 1
            realInvocation = 'deferred'
        }
        $path = Join-Path $Root ('participant-{0:d2}.json' -f $index)
        $sample | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $path -Encoding UTF8
    }
}

if (Test-Path -LiteralPath $OutputPath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite an existing preview report.' }
if ($FixtureMode) { New-FixtureSamples -Root $SamplesPath }
if (-not (Test-Path -LiteralPath $SamplesPath -PathType Container)) { throw 'SAMPLES_PATH_MISSING' }

$files = @(Get-ChildItem -LiteralPath $SamplesPath -Filter '*.json' -File | Sort-Object Name)
if ($files.Count -lt 5) { throw "PILOT_COUNT_INVALID: expected at least 5 participants, found $($files.Count)." }
$records = @()
$p0p1 = @()
$participantIds = [System.Collections.Generic.HashSet[string]]::new()
$timestamps = [System.Collections.Generic.HashSet[string]]::new()
$recognitionTimes = [System.Collections.Generic.HashSet[int]]::new()
$lastTimestamp = $null
foreach ($file in $files) {
    $record = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($field in @('schema', 'participantId', 'projectType', 'hardwareProfile', 'collectedAtUtc', 'collectionSource', 'sourceSha256', 'firstLaunchCompleted', 'stateRecognitionCorrect', 'stateRecognitionSeconds', 'guided', 'issueSeverity', 'failureReason', 'telemetryEnabled', 'sourceIncluded', 'secretIncluded', 'redactedPreviewReviewed', 'exportConfirmed', 'realInvocation')) {
        if (-not ($record.PSObject.Properties.Name -contains $field)) { throw "SAMPLE_SCHEMA_MISSING_$field in $($file.Name)" }
    }
    if (-not $participantIds.Add([string]$record.participantId)) { throw "PARTICIPANT_ID_DUPLICATE in $($file.Name)" }
    if ($record.schema -ne 'mission-control-codex-preview-sample-v1') { throw "SAMPLE_SCHEMA_INVALID in $($file.Name)" }
    if ($record.realInvocation -ne 'deferred') { throw "REAL_INVOCATION_NOT_DEFERRED in $($file.Name)" }
    if ([string]::IsNullOrWhiteSpace([string]$record.collectionSource) -or [string]$record.collectionSource -eq 'unknown') { throw "COLLECTION_SOURCE_MISSING in $($file.Name)" }
    if ([string]$record.sourceSha256 -notmatch '^[a-f0-9]{64}$') { throw "SOURCE_HASH_INVALID in $($file.Name)" }
    $timestamp = Parse-UtcTimestamp -Value $record.collectedAtUtc -Context $file.Name
    if ($timestamp -gt [DateTime]::UtcNow.AddMinutes(5)) { throw "TIMESTAMP_IN_FUTURE in $($file.Name)" }
    if (-not $timestamps.Add($timestamp.ToString('o'))) { throw "TIMESTAMP_DUPLICATE in $($file.Name)" }
    if ($null -ne $lastTimestamp -and $timestamp -le $lastTimestamp) { throw "TIMESTAMP_NOT_MONOTONIC in $($file.Name)" }
    $lastTimestamp = $timestamp
    if ($record.guided -ne $false -or $record.telemetryEnabled -ne $false) { throw "PILOT_NOT_UNGUIDED_OR_TELEMETRY_ON in $($file.Name)" }
    if ($record.sourceIncluded -ne $false -or $record.secretIncluded -ne $false -or $record.redactedPreviewReviewed -ne $true) { throw "SAMPLE_PRIVACY_INVALID in $($file.Name)" }
    if ([int]$record.stateRecognitionSeconds -lt 0 -or [int]$record.stateRecognitionSeconds -gt 600) { throw "STATE_TIME_INVALID in $($file.Name)" }
    [void]$recognitionTimes.Add([int]$record.stateRecognitionSeconds)
    $serialized = $record | ConvertTo-Json -Depth 12 -Compress
    if ($serialized -match '(?i)BEGIN (?:RSA |EC )?PRIVATE KEY|Bearer\s+[A-Za-z0-9._~-]+|mission-control-fixture-secret|rawSource|providerPayload|cookie|authorization') { throw "SAMPLE_SECRET_OR_RAW_PAYLOAD in $($file.Name)" }
    if ([string]$record.issueSeverity -match '^P[01](?:\b|[-_:])') { $p0p1 += [string]$record.issueSeverity }
    $records += [ordered]@{
        participantId = [string]$record.participantId
        projectType = [string]$record.projectType
        hardwareProfile = [string]$record.hardwareProfile
        collectedAtUtc = [string]$record.collectedAtUtc
        firstLaunchCompleted = [bool]$record.firstLaunchCompleted
        stateRecognitionCorrect = [bool]$record.stateRecognitionCorrect
        stateRecognitionSeconds = [int]$record.stateRecognitionSeconds
        failureReason = if ($null -eq $record.failureReason) { $null } else { [string]$record.failureReason }
        issueSeverity = if ($null -eq $record.issueSeverity) { $null } else { [string]$record.issueSeverity }
        sampleFile = $file.Name
        sampleSha256 = Get-Sha256Hex -Path $file.FullName
    }
}
if ($recognitionTimes.Count -lt 2) { throw 'STATE_TIME_NOT_SAMPLED: recognition timing appears hard-coded.' }

$firstLaunchRate = [math]::Round((@($records | Where-Object firstLaunchCompleted).Count / $records.Count) * 100, 0)
$stateRecognitionRate = [math]::Round((@($records | Where-Object { $_.stateRecognitionCorrect -and $_.stateRecognitionSeconds -le 10 }).Count / $records.Count) * 100, 0)
$slices = @($records | Group-Object { "$($_.projectType)|$($_.hardwareProfile)" } | ForEach-Object {
    [ordered]@{ projectType = $_.Group[0].projectType; hardwareProfile = $_.Group[0].hardwareProfile; participants = $_.Count; firstLaunchCompleted = @($_.Group | Where-Object firstLaunchCompleted).Count; stateRecognized = @($_.Group | Where-Object { $_.stateRecognitionCorrect -and $_.stateRecognitionSeconds -le 10 }).Count }
})
if ($p0p1.Count -gt 0) { throw "P0_P1_BLOCKING_FINDINGS: $($p0p1 -join ', ')" }

$parent = Split-Path -Parent $OutputPath
New-Item -ItemType Directory -Force -Path $parent | Out-Null
$thresholdsMet = ($firstLaunchRate -ge 80 -and $stateRecognitionRate -ge 90 -and $p0p1.Count -eq 0)
$report = [ordered]@{
    schema = 'mission-control-codex-preview-report-v1'
    mode = 'controlled'
    realInvocation = 'deferred'
    evidenceGrade = 'controlled-deferred'
    releaseReady = $false
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    participants = $records.Count
    firstLaunchRate = [int]$firstLaunchRate
    stateRecognitionRate = [int]$stateRecognitionRate
    p0p1Count = $p0p1.Count
    telemetryEnabled = $false
    allSamplesRedacted = $true
    thresholdsMet = $thresholdsMet
    expansionGate = $false
    slices = $slices
    samples = $records
}
$report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
Write-Output $OutputPath
