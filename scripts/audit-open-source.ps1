[CmdletBinding()]
param(
    [string] $RepositoryRoot = (Split-Path -Parent $PSScriptRoot),
    [string] $OutputRoot = '',
    [string] $RunId = ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')),
    [string[]] $UserNames = @([Environment]::UserName),
    [string[]] $MachineNames = @([Environment]::MachineName)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath $RepositoryRoot).Path
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path $repository 'artifacts\open-source-audit'
}
$runRoot = Join-Path $OutputRoot $RunId
$reportPath = Join-Path $runRoot 'open-source-audit.json'
$secretReportPath = Join-Path $runRoot 'secret-scan.json'

function Get-GitPaths {
    param([Parameter(Mandatory)][string[]] $Arguments)
    $output = & git.exe -C $repository @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') exited $LASTEXITCODE"
    }
    @($output | ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

function Get-DigestPrefix {
    param([Parameter(Mandatory)][string] $Value)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
        return (([BitConverter]::ToString($sha.ComputeHash($bytes))) -replace '-', '').Substring(0, 12).ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

$pendingFiles = @(
    Get-GitPaths @('diff', '--name-only')
    Get-GitPaths @('diff', '--cached', '--name-only')
    Get-GitPaths @('ls-files', '--others', '--exclude-standard')
) | Sort-Object -Unique

$patterns = [System.Collections.Generic.List[object]]::new()
$patterns.Add([ordered]@{ category = 'user-profile-path'; regex = '(?i)\b[A-Z]:\\Users\\[^\\\s"'']+' })
$patterns.Add([ordered]@{ category = 'non-system-drive-path'; regex = '(?i)\b[D-Z]:\\[^\r\n"'']+' })
$patterns.Add([ordered]@{ category = 'intranet-address'; regex = '\b(?:10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2})\b' })
foreach ($name in @($UserNames | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Unique)) {
    $patterns.Add([ordered]@{ category = 'user-name'; regex = "(?i)\b$([regex]::Escape($name))\b" })
}
foreach ($name in @($MachineNames | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Unique)) {
    $patterns.Add([ordered]@{ category = 'machine-name'; regex = "(?i)\b$([regex]::Escape($name))\b" })
}

$privacyFindings = [System.Collections.Generic.List[object]]::new()
foreach ($relativePath in $pendingFiles) {
    $path = Join-Path $repository ($relativePath -replace '/', [IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
    try {
        $lines = [IO.File]::ReadAllLines($path, [Text.Encoding]::UTF8)
    } catch {
        continue
    }
    for ($lineIndex = 0; $lineIndex -lt $lines.Count; $lineIndex++) {
        foreach ($pattern in $patterns) {
            foreach ($match in [regex]::Matches($lines[$lineIndex], [string]$pattern.regex)) {
                $privacyFindings.Add([pscustomobject][ordered]@{
                    path = $relativePath.Replace('\\', '/')
                    line = $lineIndex + 1
                    category = [string]$pattern.category
                    matchSha256Prefix = Get-DigestPrefix -Value $match.Value
                })
            }
        }
    }
}

$historyText = (& git.exe -C $repository log -p --all --no-color --no-ext-diff 2>$null) -join "`n"
$historyMatches = 0
if ($LASTEXITCODE -eq 0) {
    foreach ($pattern in $patterns) {
        $historyMatches += [regex]::Matches($historyText, [string]$pattern.regex).Count
    }
}

New-Item -ItemType Directory -Force -Path $runRoot | Out-Null
$secretScanner = Join-Path (Split-Path -Parent $PSScriptRoot) 'scripts\scan-secrets.ps1'
$corpus = Join-Path (Split-Path -Parent $PSScriptRoot) 'fixtures\agents\secret-corpus.json'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $secretScanner -Root $repository -Corpus $corpus -ReportPath $secretReportPath *> (Join-Path $runRoot 'secret-scan.log')
$secretExitCode = $LASTEXITCODE
$secretScanRaw = if (Test-Path -LiteralPath $secretReportPath -PathType Leaf) {
    Get-Content -LiteralPath $secretReportPath -Raw -Encoding UTF8 | ConvertFrom-Json
} else {
    [pscustomobject]@{ status = 'fail'; detection = 'corpus+patterns'; files_scanned = 0; matches = @([pscustomobject]@{ path = ''; detection = 'execution'; pattern = 'scanner-report-missing' }) }
}
$secretScan = [ordered]@{
    status = if ($secretExitCode -eq 0 -and [string]$secretScanRaw.status -eq 'pass') { 'pass' } else { 'fail' }
    detection = [string]$secretScanRaw.detection
    filesScanned = [int]$secretScanRaw.files_scanned
    matches = @($secretScanRaw.matches)
}
$status = if ($secretScan.status -eq 'fail') { 'fail' } elseif ($privacyFindings.Count -gt 0 -or $historyMatches -gt 0) { 'review-required' } else { 'pass' }
$report = [ordered]@{
    schema = 'mission-control-open-source-audit-v1'
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    status = $status
    pendingFiles = @($pendingFiles | ForEach-Object { $_.Replace('\\', '/') })
    privacyFindings = @($privacyFindings)
    history = [ordered]@{
        scan = 'git log -p --all --no-color --no-ext-diff'
        highRiskMatchCount = $historyMatches
    }
    reviewGuidance = @(
        'Review privacy findings by path and line; accepted fixtures may remain, but machine-specific production content should be rewritten.',
        'Inspect historical matches before the first public push and squash only when a match contains real private data.',
        'Any secret-scan match is fail-closed and must be removed from the pending tree and history before publication.'
    )
    secretScan = $secretScan
    sourceMutation = $false
}
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8
Write-Output $reportPath
if ($status -eq 'fail') { exit 1 }
exit 0
