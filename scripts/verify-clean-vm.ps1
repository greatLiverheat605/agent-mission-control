[CmdletBinding()]
param(
    [string] $MsiPath,
    [string] $VmName,
    [switch] $Execute,
    [string] $OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-Report {
    param([Parameter(Mandatory)] $Value)
    $parent = Split-Path -Parent $OutputPath
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    $Value | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $OutputPath -Encoding utf8
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutputPath)) { $OutputPath = Join-Path $repositoryRoot 'artifacts\release-gate\clean-vm-report.json' }

$checks = @(
    [ordered]@{ name = 'vm-prerequisites'; description = 'Windows 11/2022+, WebView2, no developer toolchain'; status = 'blocked' },
    [ordered]@{ name = 'silent-install'; description = 'signed MSI installs without UI prompts'; status = 'blocked' },
    [ordered]@{ name = 'supervisor-ready'; description = 'supervisor.ready appears'; status = 'blocked' },
    [ordered]@{ name = 'named-pipe'; description = 'Mission Control named pipe exists'; status = 'blocked' },
    [ordered]@{ name = 'credential-manager'; description = 'MissionControl/DatabaseKey/* credential exists'; status = 'blocked' },
    [ordered]@{ name = 'ledger-quick-check'; description = 'ledger DB is created and quick_check passes'; status = 'blocked' },
    [ordered]@{ name = 'ui-connected'; description = 'first launch UI reports connected'; status = 'blocked' },
    [ordered]@{ name = 'uninstall-residue'; description = 'uninstall leaves no product residue'; status = 'blocked' },
    [ordered]@{ name = 'update-rollback'; description = 'replacement and rollback point restore successfully'; status = 'blocked' }
)
$status = 'blocked'
$reason = 'A fresh externally provisioned VM and explicit -Execute were not provided; local or simulated hosts are not clean-VM evidence.'
if ($Execute -and [string]::IsNullOrWhiteSpace($VmName)) { $reason = '-Execute requires -VmName for an externally provisioned clean VM.' }
if ($Execute -and -not [string]::IsNullOrWhiteSpace($VmName)) {
    $reason = 'Execution requested; checks must be completed by the remote VM runner before PASS is accepted.'
    foreach ($check in $checks) { $check.status = 'fail'; $check.error = 'Remote VM execution adapter is not configured in this environment.' }
}
$report = [ordered]@{
    schema = 'mission-control-clean-vm-report-v1'
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    status = $status
    result = if ($status -eq 'pass') { 'PASS' } elseif ($status -eq 'fail') { 'FAIL' } else { 'BLOCKED' }
    releaseReady = $false
    vmName = $VmName
    msiPath = $MsiPath
    certificateType = 'requires-production-signed-msi'
    checks = $checks
    reason = $reason
    execution = if ($Execute) { 'requested' } else { 'readiness-only' }
}
Write-Report $report
Write-Output ("clean-vm status=$status")
Write-Output $OutputPath
if ($status -ne 'pass') { exit 1 }
