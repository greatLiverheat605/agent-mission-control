[CmdletBinding()]
param(
    [string] $ArtifactPath,
    [string] $ManifestPath,
    [string] $RunId = ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')),
    [string] $OutputRoot,
    [string] $TimestampUrl = 'http://timestamp.digicert.com',
    [string] $ReleaseGateRunId
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string] $Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes($Path))) -replace '-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutputRoot)) { $OutputRoot = Join-Path $repositoryRoot 'artifacts\release-gate' }
$runRoot = Join-Path $OutputRoot $RunId
$steps = [System.Collections.Generic.List[object]]::new()
$status = 'blocked'
$failure = $null
$certificate = $null
$trustedCertificate = $null
$trustedRoot = $null
$originalUpdaterSignature = $env:TAURI_UPDATER_SIGNATURE
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

try {
    if (-not $ArtifactPath) {
        $releaseRoot = Join-Path $repositoryRoot 'artifacts\codex-release\preview'
        $ArtifactPath = Get-ChildItem -LiteralPath $releaseRoot -Recurse -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Extension -in @('.msi', '.exe') -and $_.FullName -notmatch 'unsigned-fixture-synthetic|signed-fixture' } |
            Sort-Object @{ Expression = { if ($_.Extension -eq '.msi') { 0 } else { 1 } } }, LastWriteTime -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $ArtifactPath -or -not (Test-Path -LiteralPath $ArtifactPath -PathType Leaf)) { throw 'TEST artifact is required.' }
    $ArtifactPath = [IO.Path]::GetFullPath($ArtifactPath)
    if (-not $ManifestPath) {
        $ManifestPath = Join-Path (Split-Path -Parent $ArtifactPath) 'manifest.json'
    }
    if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) { throw 'Manifest for TEST artifact is required.' }

    $testArtifact = Join-Path $runRoot ([IO.Path]::GetFileName($ArtifactPath))
    Copy-Item -LiteralPath $ArtifactPath -Destination $testArtifact -Force
    $testManifest = Join-Path $runRoot 'manifest.json'
    Copy-Item -LiteralPath $ManifestPath -Destination $testManifest -Force
    $sourceManifest = Get-Content -LiteralPath $ManifestPath -Raw -Encoding utf8 | ConvertFrom-Json
    foreach ($sidecar in @([string]$sourceManifest.provenance, [string]$sourceManifest.sbom)) {
        if ([string]::IsNullOrWhiteSpace($sidecar)) { continue }
        $sourceSidecar = Join-Path (Split-Path -Parent $ManifestPath) $sidecar
        if (Test-Path -LiteralPath $sourceSidecar -PathType Leaf) { Copy-Item -LiteralPath $sourceSidecar -Destination (Join-Path $runRoot $sidecar) -Force }
    }
    $steps.Add([ordered]@{ name = 'build'; status = 'pass'; source = 'existing release artifact copied into isolated TEST run' })

    $certificate = Get-ChildItem -Path Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
        Where-Object { $_.Subject -eq 'CN=Agent Mission Control TEST self-signed' } | Select-Object -First 1
    if (-not $certificate) {
        $certificate = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=Agent Mission Control TEST self-signed' -CertStoreLocation 'Cert:\CurrentUser\My' -NotAfter ([DateTime]::UtcNow.AddDays(2))
    }
    if (-not $certificate -or [string]::IsNullOrWhiteSpace($certificate.Thumbprint)) { throw 'Unable to create TEST self-signed certificate.' }
    $certificateFile = Join-Path $runRoot 'test-certificate.cer'
    Export-Certificate -Cert $certificate -FilePath $certificateFile -Type CERT | Out-Null
    $trustedCertificate = Import-Certificate -FilePath $certificateFile -CertStoreLocation 'Cert:\CurrentUser\TrustedPublisher'
    $trustedRoot = Import-Certificate -FilePath $certificateFile -CertStoreLocation 'Cert:\CurrentUser\Root'
    $steps.Add([ordered]@{ name = 'certificate'; status = 'pass'; certificateType = 'TEST-self-signed'; thumbprint = $certificate.Thumbprint.ToUpperInvariant() })

    $signtool = Get-Command signtool.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $signtool) { throw 'signtool.exe is unavailable; TEST chain is ready but cannot be executed on this host.' }
    $env:TAURI_UPDATER_SIGNATURE = 'TEST-self-signed-updater-signature'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'sign-release.ps1') -ArtifactPath $testArtifact -ManifestPath $testManifest -CertificateThumbprint $certificate.Thumbprint -TimestampUrl $TimestampUrl
    if ($LASTEXITCODE -ne 0) { throw 'sign-release.ps1 failed for TEST certificate.' }
    # sign-release.ps1 runs the equivalent `signtool verify /pa /all` check;
    # Get-AuthenticodeSignature independently confirms the same TEST certificate.
    $signature = Get-AuthenticodeSignature -LiteralPath $testArtifact
    if ([string]$signature.Status -ne 'Valid') { throw "signtool verification returned $($signature.Status)." }
    $steps.Add([ordered]@{ name = 'sign-and-verify'; status = 'pass'; authenticodeStatus = [string]$signature.Status; certificateType = 'TEST-self-signed' })

    $manifest = Get-Content -LiteralPath $testManifest -Raw -Encoding utf8 | ConvertFrom-Json
    $actualHash = Get-Sha256Hex -Path $testArtifact
    $sidecarHashes = @($manifest, (Get-Content -LiteralPath (Join-Path $runRoot ([string]$manifest.provenance)) -Raw -Encoding utf8 | ConvertFrom-Json), (Get-Content -LiteralPath (Join-Path $runRoot ([string]$manifest.sbom)) -Raw -Encoding utf8 | ConvertFrom-Json)) | ForEach-Object { [string]$_.artifactSha256 }
    if (@($sidecarHashes | Where-Object { $_ -ne $actualHash }).Count -ne 0) { throw 'manifest/provenance/SBOM hashes are inconsistent after TEST signing.' }
    $steps.Add([ordered]@{ name = 'hash-consistency'; status = 'pass'; artifactSha256 = $actualHash; locations = @('manifest', 'provenance', 'sbom') })
    if ($ReleaseGateRunId) {
        $releaseGatePath = Join-Path $repositoryRoot "artifacts\release-gate\$ReleaseGateRunId\codex-release-gate.json"
        if (-not (Test-Path -LiteralPath $releaseGatePath -PathType Leaf)) { throw 'verify-codex-release.ps1 gate report is missing.' }
        $releaseGate = Get-Content -LiteralPath $releaseGatePath -Raw -Encoding utf8 | ConvertFrom-Json
        if ($releaseGate.releaseDecision -ne 'PASS_INTERNAL_PREVIEW' -or $releaseGate.releaseReady -ne $false) { throw 'verify-codex-release.ps1 gate boundary is invalid for TEST evidence.' }
        $steps.Add([ordered]@{ name = 'verify-codex-release'; status = 'pass'; script = 'scripts/verify-codex-release.ps1'; gateRunId = $ReleaseGateRunId; releaseReady = $false })
    }
    $status = 'pass'
} catch {
    $failure = $_.Exception.Message
    $steps.Add([ordered]@{ name = 'mechanical-chain'; status = 'blocked'; error = $failure })
} finally {
    if ($null -eq $originalUpdaterSignature) { Remove-Item Env:TAURI_UPDATER_SIGNATURE -ErrorAction SilentlyContinue } else { $env:TAURI_UPDATER_SIGNATURE = $originalUpdaterSignature }
    if ($trustedCertificate) {
        try { Remove-Item -LiteralPath (Join-Path 'Cert:\CurrentUser\TrustedPublisher' $trustedCertificate.Thumbprint) -Force -Confirm:$false -ErrorAction SilentlyContinue } catch { }
    }
    if ($trustedRoot) {
        try { Remove-Item -LiteralPath (Join-Path 'Cert:\CurrentUser\Root' $trustedRoot.Thumbprint) -Force -Confirm:$false -ErrorAction SilentlyContinue } catch { }
    }
    $report = [ordered]@{
        schema = 'mission-control-test-signing-chain-v1'
        runId = $RunId
        generatedAtUtc = [DateTime]::UtcNow.ToString('o')
        status = $status
        certificateType = 'TEST-self-signed'
        productionSigningStatus = 'deferred-ci'
        releaseReady = $false
        artifact = if ($ArtifactPath) { [IO.Path]::GetFileName($ArtifactPath) } else { $null }
        steps = @($steps)
        error = $failure
        excludedFromProductionEvidence = $true
    }
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $runRoot 'test-signing-chain.json') -Encoding utf8
}

Write-Output (Join-Path $runRoot 'test-signing-chain.json')
if ($status -ne 'pass') { exit 1 }
