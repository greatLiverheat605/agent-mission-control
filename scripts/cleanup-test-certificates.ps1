[CmdletBinding()]
param(
    [string] $Subject = 'CN=Agent Mission Control TEST self-signed',
    [string] $EvidenceRoot = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repositoryRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
    $EvidenceRoot = Join-Path $repositoryRoot 'artifacts\release-gate\r7-machine-hygiene-20260828'
}
New-Item -ItemType Directory -Force -Path $EvidenceRoot | Out-Null
$beforePath = Join-Path $EvidenceRoot 'store-before.json'
$afterPath = Join-Path $EvidenceRoot 'store-after.json'
$stepsPath = Join-Path $EvidenceRoot 'cleanup-steps.json'
$stores = @('Cert:\CurrentUser\My', 'Cert:\CurrentUser\Root', 'Cert:\LocalMachine\My', 'Cert:\LocalMachine\Root')

function Get-MatchingCertificates {
    $matches = [System.Collections.Generic.List[object]]::new()
    foreach ($store in $stores) {
        foreach ($cert in @(Get-ChildItem -Path $store -ErrorAction SilentlyContinue | Where-Object { $_.Subject -eq $Subject })) {
            $matches.Add([ordered]@{
                    store = $store
                    subject = [string]$cert.Subject
                    thumbprint = ([string]$cert.Thumbprint).ToUpperInvariant()
                    notAfter = $cert.NotAfter.ToUniversalTime().ToString('o')
                    path = [string]$cert.PSPath
                })
        }
    }
    return @($matches)
}

function Write-Snapshot([string]$Path, [object[]]$Certificates) {
    [ordered]@{
        generatedAtUtc = [DateTime]::UtcNow.ToString('o')
        subject = $Subject
        certificates = @($Certificates)
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Path -Encoding utf8
}

function Invoke-ExactRemoveItem {
    param([Parameter(Mandatory)][string]$Target)
    $result = [ordered]@{ step = 'Remove-Item'; target = $Target; status = 'blocked' }
    $stdoutPath = Join-Path $EvidenceRoot ("remove-item-$([Guid]::NewGuid().ToString('N')).out")
    $stderrPath = Join-Path $EvidenceRoot ("remove-item-$([Guid]::NewGuid().ToString('N')).err")
    $command = "try { Remove-Item -LiteralPath '$Target' -Force -ErrorAction Stop; exit 0 } catch { [Console]::Error.WriteLine(`$_.Exception.Message); exit 1 }"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = New-Object System.Diagnostics.ProcessStartInfo
    $process.StartInfo.FileName = 'powershell.exe'
    $process.StartInfo.Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encoded"
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    try {
        [void]$process.Start()
        if (-not $process.WaitForExit(5000)) {
            try { $process.Kill() } catch { }
            $result.status = 'timeout'
        } else {
            $result.status = if ($process.ExitCode -eq 0) { 'pass' } else { 'fail' }
            $result.exitCode = $process.ExitCode
        }
        $result.output = (($process.StandardOutput.ReadToEnd() + $process.StandardError.ReadToEnd()).Trim())
    } catch {
        $result.status = 'error'
        $result.error = $_.Exception.Message
    } finally {
        $process.Dispose()
    }
    return [pscustomobject]$result
}

function Invoke-CertutilDelete {
    param([Parameter(Mandatory)][string]$StoreName, [Parameter(Mandatory)][string]$Thumbprint)
    $result = [ordered]@{ step = 'certutil'; store = $StoreName; thumbprint = $Thumbprint; status = 'blocked' }
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = New-Object System.Diagnostics.ProcessStartInfo
    $process.StartInfo.FileName = 'certutil.exe'
    $process.StartInfo.Arguments = "-user -f -delstore $StoreName $Thumbprint"
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    try {
        [void]$process.Start()
        if (-not $process.WaitForExit(10000)) {
            try { $process.Kill() } catch { }
            $result.status = 'timeout'
        } else {
            $result.status = if ($process.ExitCode -eq 0) { 'pass' } else { 'fail' }
            $result.exitCode = $process.ExitCode
        }
        $result.output = (($process.StandardOutput.ReadToEnd() + $process.StandardError.ReadToEnd()).Trim())
    } catch {
        $result.status = 'error'
        $result.error = $_.Exception.Message
    } finally {
        $process.Dispose()
    }
    return [pscustomobject]$result
}

$allBefore = if (Test-Path -LiteralPath $beforePath -PathType Leaf) {
    @((Get-Content -LiteralPath $beforePath -Raw -Encoding utf8 | ConvertFrom-Json).certificates)
} else {
    @(Get-MatchingCertificates)
}
$before = @($allBefore | Where-Object { $_.store -like 'Cert:\CurrentUser\*' })
if (-not (Test-Path -LiteralPath $beforePath -PathType Leaf)) {
    Write-Snapshot -Path $beforePath -Certificates $allBefore
}
$steps = [System.Collections.Generic.List[object]]::new()
$steps.Add([ordered]@{ step = 'locate'; status = 'pass'; count = @($before).Count; snapshot = 'store-before.json' })

foreach ($entry in @($before)) {
    $target = Join-Path $entry.store $entry.thumbprint
    $step = Invoke-ExactRemoveItem -Target $target
    $step | Add-Member -NotePropertyName store -NotePropertyValue $entry.store
    $step | Add-Member -NotePropertyName thumbprint -NotePropertyValue $entry.thumbprint
    $steps.Add($step)
}

$remaining = Get-MatchingCertificates
foreach ($entry in @($remaining)) {
    $storeName = Split-Path -Leaf $entry.store
    $steps.Add((Invoke-CertutilDelete -StoreName $storeName -Thumbprint $entry.thumbprint))
}

$remaining = Get-MatchingCertificates
foreach ($entry in @($remaining)) {
    $registryPath = "HKCU:\Software\Microsoft\SystemCertificates\$((Split-Path -Leaf $entry.store))\Certificates\$($entry.thumbprint.ToLowerInvariant())"
    $step = [ordered]@{ step = 'HKCU-registry'; registryPath = $registryPath; thumbprint = $entry.thumbprint; status = 'blocked' }
    try {
        $subKey = "Software\Microsoft\SystemCertificates\$((Split-Path -Leaf $entry.store))\Certificates\$($entry.thumbprint.ToLowerInvariant())"
        $existing = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($subKey)
        if ($null -ne $existing) {
            $existing.Close()
            [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($subKey)
            $step.status = 'pass'
        } else {
            $step.status = 'already-absent'
        }
    } catch {
        $step.status = 'error'
        $step.error = $_.Exception.Message
    }
    $steps.Add([pscustomobject]$step)
}

$allAfter = Get-MatchingCertificates
$after = @($allAfter | Where-Object { $_.store -like 'Cert:\CurrentUser\*' })
Write-Snapshot -Path $afterPath -Certificates $allAfter
$pfxFiles = @(Get-ChildItem -LiteralPath (Join-Path $repositoryRoot 'artifacts') -Recurse -File -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '(?i)\.pfx$|password|passwd' -and $_.FullName -match '(?i)r6|test-signing' } |
        ForEach-Object { [ordered]@{ path = $_.FullName; action = 'not-found-or-not-deleted-by-this-run' } })
$steps.Add([ordered]@{ step = 'pfx-password-files'; status = if (@($pfxFiles).Count -eq 0) { 'pass-none-found' } else { 'review-required' }; files = @($pfxFiles) })
$steps.Add([ordered]@{ step = 'post-restart-recheck'; status = 'user-required'; command = 'Run this script again after restarting Explorer or Windows.' })

[ordered]@{
    schema = 'mission-control-test-certificate-cleanup-v1'
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    subject = $Subject
    beforeSnapshot = 'store-before.json'
    afterSnapshot = 'store-after.json'
    matchedBefore = @($before).Count
    matchedAfter = @($after).Count
    localMachineMatchesAfter = @($allAfter | Where-Object { $_.store -like 'Cert:\LocalMachine\*' }).Count
    status = if (@($after).Count -eq 0) { 'pass' } else { 'blocked' }
    steps = @($steps)
} | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $stepsPath -Encoding utf8

Write-Output $stepsPath
if (@($after).Count -ne 0) { exit 1 }
exit 0
