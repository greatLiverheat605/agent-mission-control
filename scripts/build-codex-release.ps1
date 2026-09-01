[CmdletBinding()]
param(
    [ValidateSet('preview', 'internal')]
    [string] $Channel = 'preview',

    [string] $Version,

    [string] $ArtifactPath,

    [switch] $SkipBuild
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

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$configPath = Join-Path $repositoryRoot 'apps\desktop\src-tauri\tauri.conf.json'
$config = Get-Content -LiteralPath $configPath -Raw -Encoding utf8 | ConvertFrom-Json
if (-not $Version) { $Version = [string] $config.version }
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Version must use MAJOR.MINOR.PATCH.' }

$outputRoot = Join-Path $repositoryRoot "artifacts\codex-release\$Channel\$Version"
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null

if (-not $SkipBuild) {
    Push-Location $repositoryRoot
    try {
        & npm.cmd run build --workspace @mission-control/desktop
        if ($LASTEXITCODE -ne 0) { throw 'Desktop frontend build failed.' }
        & npm.cmd run tauri:build --workspace @mission-control/desktop
        if ($LASTEXITCODE -ne 0) { throw 'Tauri bundle build failed.' }
    } finally {
        Pop-Location
    }
}

if (-not $ArtifactPath) {
    $bundleRoot = Join-Path $repositoryRoot 'apps\desktop\src-tauri\target\release\bundle'
    if (-not (Test-Path -LiteralPath $bundleRoot)) {
        $bundleRoot = Join-Path $repositoryRoot 'target\release\bundle'
    }
    $candidates = @(Get-ChildItem -LiteralPath $bundleRoot -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Extension -in @('.msi', '.exe') -and $_.Name -notmatch 'smoke|supervisor' })
    if ($candidates.Count -ne 1) { throw "Expected exactly one release artifact, found $($candidates.Count)." }
    $ArtifactPath = $candidates[0].FullName
}
if (-not (Test-Path -LiteralPath $ArtifactPath -PathType Leaf)) { throw 'Release artifact does not exist.' }
$ArtifactPath = [IO.Path]::GetFullPath($ArtifactPath)
if ([IO.Path]::GetExtension($ArtifactPath).ToLowerInvariant() -notin @('.msi', '.exe')) {
    throw 'Release artifact must be an MSI or EXE.'
}

$artifactName = "AgentMissionControl-$Version-$Channel$([IO.Path]::GetExtension($ArtifactPath))"
$publishedArtifact = Join-Path $outputRoot $artifactName
Copy-Item -LiteralPath $ArtifactPath -Destination $publishedArtifact -Force
$artifactHash = Get-Sha256Hex -Path $publishedArtifact

$gitCommit = (& git -C $repositoryRoot rev-parse HEAD 2>$null).Trim()
if ($LASTEXITCODE -ne 0) { $gitCommit = 'working-tree' }
$provenance = [ordered]@{
    schema = 'mission-control-provenance-v1'
    channel = $Channel
    version = $Version
    artifactSha256 = $artifactHash
    gitCommit = $gitCommit
    builtAtUtc = [DateTime]::UtcNow.ToString('o')
    signer = 'UNSIGNED'
}
$sbomPath = Join-Path $outputRoot 'sbom.json'
if (Test-Path -LiteralPath $sbomPath -PathType Leaf) { Remove-Item -LiteralPath $sbomPath -Force }
& (Join-Path $PSScriptRoot 'generate-sbom.ps1') -ArtifactPath $publishedArtifact -OutputPath $sbomPath -Version $Version -Channel $Channel
if ($LASTEXITCODE -ne 0) { throw 'SBOM generation failed.' }
$manifest = [ordered]@{
    schema = 'mission-control-update-manifest-v1'
    channel = $Channel
    version = $Version
    artifact = $artifactName
    artifactSha256 = $artifactHash
    signerFingerprint = 'UNSIGNED'
    signatureScheme = 'tauri-updater-v1'
    signature = ''
    schemaVersion = 1
    minSchemaVersion = 1
    sbom = 'sbom.json'
    provenance = 'provenance.json'
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'manifest.json') -Encoding utf8
$provenance | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'provenance.json') -Encoding utf8
Write-Output (Join-Path $outputRoot 'manifest.json')
