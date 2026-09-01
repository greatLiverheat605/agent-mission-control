[CmdletBinding()]
param(
  [string]$Root = '',
  [switch]$SkipRust,
  [switch]$SkipNode,
  [switch]$SkipE2E
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Root)) { $Root = Split-Path -Parent $PSScriptRoot }
Set-Location -LiteralPath $Root
$artifactDir = Join-Path $Root 'artifacts/security'
New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null
$failures = [System.Collections.Generic.List[string]]::new()

function Invoke-Gate([string]$Name, [scriptblock]$Action) {
  Write-Host "[security] $Name"
  try { & $Action; if ($LASTEXITCODE -ne 0) { throw "exit code $LASTEXITCODE" } }
  catch { $failures.Add("${Name}: $($_.Exception.Message)") }
}

if (-not $SkipRust) {
  Invoke-Gate 'adapter-codex protocol/security tests' { cargo test -p adapter-codex -p mission-protocol -p mission-policy --locked --offline }
}
if (-not $SkipNode) {
  Invoke-Gate 'Node security regression tests' { npm.cmd run test:security }
}
Invoke-Gate 'Codex protocol/schema consistency' { node.exe --test tests/security/codex-protocol-consistency.test.mjs }
Invoke-Gate 'Open-source go-live contracts' { node.exe --test tests/release/open-source-go-live.test.mjs }
Invoke-Gate 'secret persistence scan (corpus+patterns)' {
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Root 'scripts/scan-secrets.ps1') -Root $Root -ReportPath (Join-Path $artifactDir 'secret-scan.json')
}
if (-not $SkipE2E) {
  $e2e = Join-Path $Root 'tests/e2e/codex-restart-recovery.spec.ts'
  if (Test-Path -LiteralPath $e2e) {
    # Playwright resolves file arguments relative to testDir. Passing the
    # absolute path here makes it look outside testDir on Windows and yields
    # a misleading "No tests found" result.
    Invoke-Gate 'Codex recovery security E2E' { npx.cmd playwright test 'tests/e2e/codex-restart-recovery.spec.ts' --config (Join-Path $Root 'playwright.config.ts') }
  }
}

$summary = [pscustomobject]@{
  generated_at = [DateTime]::UtcNow.ToString('o')
  status = if ($failures.Count -eq 0) { 'pass' } else { 'fail' }
  p0_p1_failures = @($failures)
  codex_only = $true
  real_invocation = 'deferred'
}
$summary | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $artifactDir 'security-gate.json') -Encoding UTF8
$summary | ConvertTo-Json -Depth 5 | Write-Output
if ($failures.Count -gt 0) { exit 1 }
exit 0
