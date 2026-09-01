[CmdletBinding()]
param(
    [string] $ArtifactPath,
    [string] $OutputPath = (Join-Path (Get-Location) 'artifacts\codex-release\sbom.json'),
    [string] $Version,
    [ValidateSet('preview', 'internal')]
    [string] $Channel = 'preview'
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

if (Test-Path -LiteralPath $OutputPath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite an SBOM.' }
$repositoryRoot = Split-Path -Parent $PSScriptRoot
if (-not $Version) {
    $config = Get-Content -LiteralPath (Join-Path $repositoryRoot 'apps\desktop\src-tauri\tauri.conf.json') -Raw -Encoding utf8 | ConvertFrom-Json
    $Version = [string]$config.version
}
$artifactHash = $null
if ($ArtifactPath) {
    if (-not (Test-Path -LiteralPath $ArtifactPath -PathType Leaf)) { throw 'Artifact does not exist.' }
    $artifactHash = Get-Sha256Hex -Path ([IO.Path]::GetFullPath($ArtifactPath))
}

$cargoText = Get-Content -LiteralPath (Join-Path $repositoryRoot 'Cargo.lock') -Raw -Encoding utf8
$cargoDependencies = @([regex]::Matches($cargoText, '(?ms)^\[\[package\]\]\s*(.*?)(?=^\[\[package\]\]|\z)') | ForEach-Object {
    $block = $_.Groups[1].Value
    $nameMatch = [regex]::Match($block, '(?m)^name = "([^"]+)"')
    $versionMatch = [regex]::Match($block, '(?m)^version = "([^"]+)"')
    if (-not $nameMatch.Success -or -not $versionMatch.Success) { throw 'Cargo.lock package is missing name or version.' }
    $dependency = [ordered]@{
        name = $nameMatch.Groups[1].Value
        version = $versionMatch.Groups[1].Value
        source = 'Cargo.lock'
    }
    $checksumMatch = [regex]::Match($block, '(?m)^checksum = "([^"]+)"')
    if ($checksumMatch.Success) { $dependency.checksum = $checksumMatch.Groups[1].Value }
    $dependency
})
$packageLockPath = Join-Path $repositoryRoot 'package-lock.json'
# Windows PowerShell 5.1 cannot parse npm lockfiles containing the valid empty
# workspace key (`packages[""]`). JavaScriptSerializer handles that shape on
# both Windows PowerShell and PowerShell 7 without relying on -AsHashtable.
Add-Type -AssemblyName System.Web.Extensions
$jsonSerializer = New-Object System.Web.Script.Serialization.JavaScriptSerializer
$packageLock = $jsonSerializer.DeserializeObject([IO.File]::ReadAllText($packageLockPath, [Text.Encoding]::UTF8))
$npmDependencies = @($packageLock['packages'].GetEnumerator() | Where-Object { $_.Key -and $_.Key -ne '' } | ForEach-Object {
    $package = $_.Value
    if ($package -is [System.Collections.IDictionary] -and $package.ContainsKey('version')) {
        [ordered]@{ name = $_.Key -replace '^node_modules/', ''; version = [string]$package['version']; source = 'package-lock.json' }
    }
})
$parent = Split-Path -Parent $OutputPath
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
$report = [ordered]@{
    schema = 'mission-control-sbom-v1'
    product = 'Agent Mission Control'
    channel = $Channel
    version = $Version
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    artifactSha256 = $artifactHash
    dependencies = @($cargoDependencies + $npmDependencies)
}
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
Write-Output $OutputPath
