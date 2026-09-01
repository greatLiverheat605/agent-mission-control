[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $ArtifactPath,
    [Parameter(Mandatory)][string] $ManifestPath,
    [Parameter(Mandatory)][string] $CertificateThumbprint,
    [string] $TimestampUrl = 'http://timestamp.digicert.com'
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

if ($CertificateThumbprint -notmatch '^[0-9A-Fa-f]{16,64}$') { throw 'CertificateThumbprint must be a certificate-store thumbprint.' }
if (-not (Test-Path -LiteralPath $ArtifactPath -PathType Leaf)) { throw 'Artifact does not exist.' }
if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) { throw 'Manifest does not exist.' }
$signtool = Get-Command signtool.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe is required for signing; no private key was accessed.' }

$updaterSignature = [Environment]::GetEnvironmentVariable('TAURI_UPDATER_SIGNATURE')
if ([string]::IsNullOrWhiteSpace($updaterSignature)) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY'))) {
        throw 'Provide TAURI_UPDATER_SIGNATURE or TAURI_SIGNING_PRIVATE_KEY through the CI secret store; this script will not forge a signature.'
    }
    $tauri = Get-Command tauri.cmd -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $tauri) { throw 'tauri.cmd is required for updater signing.' }
    $signaturePath = "$ArtifactPath.sig"
    Push-Location (Split-Path -Parent $PSScriptRoot)
    try {
        & npm.cmd exec --workspace '@mission-control/desktop' -- tauri signer sign $ArtifactPath *> $null
        if ($LASTEXITCODE -ne 0) { throw 'Tauri updater signing failed.' }
    } finally {
        Pop-Location
    }
    if (-not (Test-Path -LiteralPath $signaturePath -PathType Leaf)) { throw 'Tauri signer did not produce an artifact signature.' }
    $updaterSignature = Get-Content -LiteralPath $signaturePath -Raw -Encoding utf8
}

& $signtool.Source sign /sha1 $CertificateThumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 $ArtifactPath
if ($LASTEXITCODE -ne 0) { throw 'Authenticode signing failed.' }
& $signtool.Source verify /pa /all $ArtifactPath
if ($LASTEXITCODE -ne 0) { throw 'Authenticode verification failed.' }

$manifest = Get-Content -LiteralPath $ManifestPath -Raw -Encoding utf8 | ConvertFrom-Json
$actualHash = Get-Sha256Hex -Path $ArtifactPath
$manifest.signerFingerprint = $CertificateThumbprint.ToUpperInvariant()
$manifest.signatureScheme = 'tauri-updater-v1'
$manifest.artifactSha256 = $actualHash
$manifest.signature = $updaterSignature.Trim()
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ManifestPath -Encoding utf8
$provenancePath = Join-Path (Split-Path -Parent $ManifestPath) 'provenance.json'
if (Test-Path -LiteralPath $provenancePath -PathType Leaf) {
    $provenance = Get-Content -LiteralPath $provenancePath -Raw -Encoding utf8 | ConvertFrom-Json
    $provenance.artifactSha256 = $actualHash
    $provenance.signer = $manifest.signerFingerprint
    $provenance | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $provenancePath -Encoding utf8
}
$sbomPath = Join-Path (Split-Path -Parent $ManifestPath) 'sbom.json'
if (Test-Path -LiteralPath $sbomPath -PathType Leaf) {
    $sbom = Get-Content -LiteralPath $sbomPath -Raw -Encoding utf8 | ConvertFrom-Json
    $sbom.artifactSha256 = $actualHash
    $sbom | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $sbomPath -Encoding utf8
}
Write-Output $ManifestPath
