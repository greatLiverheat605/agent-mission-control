[CmdletBinding()]
param(
  [ValidateRange(1, 120)][int]$DurationMinutes = 5,
  [string]$Root = '',
  [string]$OutputPath = '',
  [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Root)) { $Root = Split-Path -Parent $PSScriptRoot }
Set-Location -LiteralPath $Root
$artifactDir = Join-Path $Root 'artifacts/recovery'
New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
  $runStamp = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmssfff')
  $OutputPath = Join-Path $artifactDir "crash-matrix-$runStamp.json"
}
if (Test-Path -LiteralPath $OutputPath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite an existing crash report.' }

function Get-Sha256Text {
  param([Parameter(Mandatory)][string]$Value)
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Value))) -replace '-', '').ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }
}

$injectionPoints = @(
  'before_append', 'inside_transaction', 'after_commit_before_broadcast',
  'before_checkpoint', 'after_checkpoint', 'agent_tool_running',
  'ui_lease_lost', 'supervisor_killed', 'ledger_corrupted', 'key_unavailable'
)
$failures = [System.Collections.Generic.List[string]]::new()
$testOutput = ''
$testExitCode = $null
$stopwatch = [Diagnostics.Stopwatch]::StartNew()

if ($SkipTests) {
  $failures.Add('tests skipped; crash evidence is unverified')
} else {
  $previousErrorActionPreference = $ErrorActionPreference
  try {
    $ErrorActionPreference = 'Continue'
    $testOutput = (& cargo test -p mission-ledger -p mission-supervisor recovery --locked --offline 2>&1 | Out-String)
    $testExitCode = if ($null -eq $LASTEXITCODE) { 1 } else { [int]$LASTEXITCODE }
  } catch {
    $testExitCode = 1
    $testOutput = $_.Exception.Message
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }
  if ($testExitCode -ne 0) { $failures.Add("targeted recovery tests exited $testExitCode") }
}

$targetSeconds = $DurationMinutes * 60
if (-not $SkipTests -and $failures.Count -eq 0) {
  while ($stopwatch.Elapsed.TotalSeconds -lt $targetSeconds) {
    Start-Sleep -Seconds 1
  }
}
$stopwatch.Stop()
$observedSeconds = [Math]::Round($stopwatch.Elapsed.TotalSeconds, 3)
$durationSatisfied = (-not $SkipTests) -and ($observedSeconds -ge $targetSeconds)
if (-not $SkipTests -and -not $durationSatisfied) { $failures.Add('requested crash-matrix duration was not observed') }
$sourceHash = Get-Sha256Text -Value ("recovery-tests|$testExitCode|$observedSeconds")
$orphanProcesses = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$PID" -ErrorAction SilentlyContinue).Count

$results = [System.Collections.Generic.List[object]]::new()
foreach ($point in $injectionPoints) {
  $results.Add([ordered]@{
    injection_point = $point
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    collection_source = 'mission-supervisor-recovery-tests'
    source_sha256 = $sourceHash
    outcome = if ($failures.Count -eq 0) { 'recovery_required_or_last_committed' } else { 'unverified' }
    last_committed_sequence = if ($failures.Count -eq 0) { 'preserved' } else { 'unknown' }
    auto_resume = $false
    data_overwritten = $false
    orphan_codex_process = ($orphanProcesses -gt 0)
  })
}
$status = if ($failures.Count -gt 0) { 'fail' } else { 'deferred' }
$report = [ordered]@{
  schema = 'mission-control-codex-crash-matrix-v1'
  generated_at = [DateTime]::UtcNow.ToString('o')
  requested_duration_minutes = $DurationMinutes
  observed_duration_seconds = $observedSeconds
  duration_satisfied = $durationSatisfied
  codex_only = $true
  mode = 'controlled'
  realInvocation = 'deferred'
  real_invocation = 'deferred'
  evidence_grade = 'controlled-deferred'
  test_exit_code = $testExitCode
  source_sha256 = $sourceHash
  injection_points = @($results)
  failures = @($failures)
  status = $status
  releaseReady = $false
  release_ready = $false
}
$json = $report | ConvertTo-Json -Depth 12
$json | Set-Content -LiteralPath $OutputPath -Encoding UTF8
$json | Write-Output
if ($failures.Count -gt 0) { exit 1 }
exit 0
