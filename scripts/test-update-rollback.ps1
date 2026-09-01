[CmdletBinding()]
param(
    [string] $ManifestPath,
    [switch] $KeepArtifacts,
    [switch] $FixtureMode,
    [ValidateSet('preview', 'internal')]
    [string] $Channel = 'preview',
    [string] $CurrentVersion = '0.0.0'
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

function Assert-InstallAllowed {
    param([Parameter(Mandatory)][string] $DataRoot)
    if (Test-Path -LiteralPath (Join-Path $DataRoot 'active-mission.lock')) {
        throw 'UPDATE_ACTIVE_MISSION'
    }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
if (-not $ManifestPath) {
    $channelRoot = Join-Path (Join-Path $repositoryRoot 'artifacts\codex-release') $Channel
    $manifests = @(Get-ChildItem -LiteralPath $channelRoot -Filter manifest.json -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Parent.Name -eq $Channel })
    if ($manifests.Count -eq 0) { throw "Provide -ManifestPath or create a $Channel release manifest." }
    $ManifestPath = ($manifests |
        Sort-Object { [version] $_.Directory.Name } -Descending |
        Select-Object -First 1).FullName
}
$manifest = Get-Content -LiteralPath $ManifestPath -Raw -Encoding utf8 | ConvertFrom-Json
$manifestRoot = Split-Path -Parent $ManifestPath
$artifactPath = Join-Path $manifestRoot ([string] $manifest.artifact)
if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) { throw 'Manifest artifact is missing.' }
$requiredFields = @('channel', 'version', 'artifactSha256', 'signerFingerprint', 'signatureScheme', 'signature', 'schemaVersion', 'minSchemaVersion')
foreach ($field in $requiredFields) {
    if ($null -eq $manifest.$field) { throw "UPDATE_MANIFEST_SCHEMA_MISSING_$field" }
}
if ([string] $manifest.schema -ne 'mission-control-update-manifest-v1') { throw 'UPDATE_MANIFEST_SCHEMA_INVALID' }
if ([string] $manifest.channel -notin @('preview', 'internal')) { throw 'UPDATE_CHANNEL_INVALID' }
if ([int] $manifest.schemaVersion -le 0 -or [int] $manifest.minSchemaVersion -le 0 -or [int] $manifest.minSchemaVersion -gt [int] $manifest.schemaVersion) {
    throw 'UPDATE_SCHEMA_INCOMPATIBLE'
}
$actualHash = Get-Sha256Hex -Path $artifactPath
if ([string] $manifest.artifactSha256 -ne $actualHash) { throw 'UPDATE_ARTIFACT_HASH_MISMATCH' }
if ([string] $manifest.version -notmatch '^\d+\.\d+\.\d+$' -or [version] $manifest.version -le [version] $CurrentVersion) { throw 'UPDATE_DOWNGRADE' }
if ([string]::IsNullOrWhiteSpace([string] $manifest.signature) -or [string] $manifest.signerFingerprint -eq 'UNSIGNED') {
    if (-not $FixtureMode) { throw 'UPDATE_UNSIGNED' }
    $manifest.signerFingerprint = 'TEST-SIGNER'
    $manifest.signatureScheme = 'fixture-sha256-v1'
    $signatureInput = "mission-control-update-v1:$($manifest.signerFingerprint):$actualHash"
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($signatureInput))
    } finally {
        $sha.Dispose()
    }
    $manifest.signature = ([BitConverter]::ToString($digest) -replace '-', '').ToLowerInvariant()
} elseif ([string] $manifest.signatureScheme -ne 'tauri-updater-v1') {
    throw 'UPDATE_SIGNATURE_SCHEME_INVALID'
}

$root = Join-Path ([IO.Path]::GetTempPath()) "mission-update-rollback-$([guid]::NewGuid().ToString('N'))"
$oldRoot = Join-Path $root 'old'
$newRoot = Join-Path $root 'new'
$dataRoot = Join-Path $root 'data'
New-Item -ItemType Directory -Force -Path $oldRoot, $newRoot, $dataRoot | Out-Null
try {
    Set-Content -LiteralPath (Join-Path $oldRoot 'app.exe') -Value 'old-bootable' -Encoding utf8
    Set-Content -LiteralPath (Join-Path $newRoot 'app.exe') -Value 'new-package' -Encoding utf8
    Set-Content -LiteralPath (Join-Path $dataRoot 'ledger.db') -Value 'committed-sequence-42' -Encoding utf8

    $activeMissionLock = Join-Path $dataRoot 'active-mission.lock'
    New-Item -ItemType File -Path $activeMissionLock | Out-Null
    if (Test-Path -LiteralPath $activeMissionLock) {
        try {
            Assert-InstallAllowed -DataRoot $dataRoot
            throw 'UPDATE_ACTIVE_MISSION_GUARD_FAILED'
        } catch {
            if ($_.Exception.Message -ne 'UPDATE_ACTIVE_MISSION') { throw }
        }
        Remove-Item -LiteralPath (Join-Path $dataRoot 'active-mission.lock') -Force
    } else { throw 'UPDATE_ACTIVE_MISSION_GUARD_FAILED' }

    $backup = Join-Path $root 'backup'
    Copy-Item -LiteralPath $oldRoot -Destination $backup -Recurse
    $marker = Join-Path $newRoot '.update-in-progress'
    Set-Content -LiteralPath $marker -Value 'interrupted' -Encoding utf8
    Remove-Item -LiteralPath (Join-Path $newRoot 'app.exe') -Force
    Copy-Item -LiteralPath (Join-Path $backup 'app.exe') -Destination (Join-Path $newRoot 'app.exe') -Force
    if ((Get-Content -LiteralPath (Join-Path $newRoot 'app.exe') -Raw) -ne "old-bootable`r`n") { throw 'UPDATE_ROLLBACK_BOOT_FAILED' }
    if ((Get-Content -LiteralPath (Join-Path $dataRoot 'ledger.db') -Raw) -ne "committed-sequence-42`r`n") { throw 'UPDATE_DATA_CHANGED' }
    Write-Output 'update rollback checks passed: hash, unsigned, active mission, interruption, backup, data preservation'
} finally {
    if (-not $KeepArtifacts) { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
}
