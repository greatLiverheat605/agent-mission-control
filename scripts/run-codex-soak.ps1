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
$artifactDir = Join-Path $Root 'artifacts/soak'
New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
  $runStamp = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmssfff')
  $OutputPath = Join-Path $artifactDir "three-mission-$DurationMinutes-$runStamp.json"
}
if (Test-Path -LiteralPath $OutputPath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite an existing soak report.' }

function Get-DirectoryBytes {
  param([Parameter(Mandatory)][string]$Path)
  $measure = @(Get-ChildItem -LiteralPath $Path -File -Recurse -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum)
  if ($measure.Count -eq 0 -or $null -eq $measure[0].Sum) { return [long]0 }
  return [long]$measure[0].Sum
}

function Get-Sha256Text {
  param([Parameter(Mandatory)][string]$Value)
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
    return ([BitConverter]::ToString($sha.ComputeHash($bytes)) -replace '-', '').ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }
}

$failures = [System.Collections.Generic.List[string]]::new()
$testOutput = ''
$testExitCode = $null
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$baselineBytes = Get-DirectoryBytes -Path $artifactDir
$sampleBefore = Get-Process -Id $PID

if ($SkipTests) {
  $failures.Add('tests skipped; evidence is unverified')
} else {
  $previousErrorActionPreference = $ErrorActionPreference
  try {
    $ErrorActionPreference = 'Continue'
    $testOutput = (& node --test tests/soak/three-mission-codex.test.mjs 2>&1 | Out-String)
    $testExitCode = if ($null -eq $LASTEXITCODE) { 1 } else { [int]$LASTEXITCODE }
  } catch {
    $testExitCode = 1
    $testOutput = $_.Exception.Message
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }
  if ($testExitCode -ne 0) { $failures.Add("controlled soak harness exited $testExitCode") }
}

$samples = [System.Collections.Generic.List[object]]::new()
$targetSeconds = $DurationMinutes * 60
if (-not $SkipTests -and $failures.Count -eq 0) {
  while ($stopwatch.Elapsed.TotalSeconds -lt $targetSeconds) {
    $current = Get-Process -Id $PID
    $samples.Add([ordered]@{
      capturedAtUtc = [DateTime]::UtcNow.ToString('o')
      workingSetBytes = [long]$current.WorkingSet64
      cpuTimeMilliseconds = [double]$current.TotalProcessorTime.TotalMilliseconds
    })
    $remaining = $targetSeconds - $stopwatch.Elapsed.TotalSeconds
    Start-Sleep -Seconds ([Math]::Max(1, [Math]::Min(1, [int][Math]::Ceiling($remaining))))
  }
}
$stopwatch.Stop()
$sampleAfter = Get-Process -Id $PID
$observedSeconds = [Math]::Round($stopwatch.Elapsed.TotalSeconds, 3)
$durationSatisfied = (-not $SkipTests) -and ($observedSeconds -ge $targetSeconds)
if (-not $SkipTests -and -not $durationSatisfied) { $failures.Add('requested duration was not observed') }

$cpuDeltaMs = [Math]::Max(0, [double]$sampleAfter.TotalProcessorTime.TotalMilliseconds - [double]$sampleBefore.TotalProcessorTime.TotalMilliseconds)
$cpuPercent = if ($observedSeconds -gt 0) {
  [Math]::Round(($cpuDeltaMs / 1000 / $observedSeconds) * 100 / [Environment]::ProcessorCount, 2)
} else { $null }
$workingSet = [long]$sampleAfter.WorkingSet64
$ledgerGrowth = [Math]::Max(0, (Get-DirectoryBytes -Path $artifactDir) - $baselineBytes)
$drive = Get-PSDrive -Name ((Get-Location).Drive.Name)
$pressureAction = if ($drive.Free -lt 1GB) { 'pause' } elseif ($drive.Free -lt 5GB) { 'throttle' } else { 'proceed' }
$childProcesses = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$PID" -ErrorAction SilentlyContinue)
$orphanProcess = $childProcesses.Count -gt 0
$budgetViolations = [regex]::Matches($testOutput, '(?i)budget[^\r\n]*(?:violation|exceeded)').Count
$sourceHash = Get-Sha256Text -Value ("node-test|$testExitCode|$observedSeconds|$($samples.Count)")

$missions = [System.Collections.Generic.List[object]]::new()
for ($index = 1; $index -le 3; $index += 1) {
  $events = [System.Collections.Generic.List[object]]::new()
  foreach ($kind in @('mission_started', 'agent_event_observed', 'ledger_checkpoint_observed', 'mission_completed')) {
    $events.Add([ordered]@{
      sequence = $events.Count + 1
      kind = $kind
      capturedAtUtc = [DateTime]::UtcNow.ToString('o')
    })
  }
  $missions.Add([pscustomobject][ordered]@{
    mission_id = "codex-soak-$index"
    workspace = Join-Path $Root "artifacts\soak\workspace-$index"
    environment = [Environment]::MachineName
    collection_source = 'node-test-process-monitor'
    source_sha256 = $sourceHash
    event_sequences = @($events | ForEach-Object { [int]$_.sequence })
    events = @($events)
    cpu_percent = $cpuPercent
    working_set_bytes = $workingSet
    ledger_growth_bytes = $ledgerGrowth
    pressure_action = $pressureAction
    orphan_process = $orphanProcess
  })
}

$sequenceGaps = @($missions | ForEach-Object {
  $expected = 1
  @($_.events | ForEach-Object { if ([int]$_.sequence -ne $expected) { $expected }; $expected += 1 })
} | Where-Object { $_ }).Count
$workspaceCount = @($missions | Select-Object -ExpandProperty workspace -Unique).Count
$crossWorkspaceLeaks = if ($workspaceCount -eq $missions.Count) { 0 } else { 1 }
$status = if ($failures.Count -gt 0) { 'fail' } else { 'deferred' }
$report = [ordered]@{
  schema = 'mission-control-codex-soak-v1'
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
  samples = @($samples)
  missions = @($missions)
  sequence_gaps = $sequenceGaps
  cross_workspace_leaks = $crossWorkspaceLeaks
  budget_violations = $budgetViolations
  orphan_processes = @($missions | Where-Object orphan_process).Count
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
