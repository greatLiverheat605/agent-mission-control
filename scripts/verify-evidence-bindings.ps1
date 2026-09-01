[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $BindingsPath,
    [string] $RepositoryRoot = '',
    [string] $ReviewBundlePath = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) { $RepositoryRoot = Split-Path -Parent $PSScriptRoot }

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string] $Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes($Path))) -replace '-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

if (-not (Test-Path -LiteralPath $BindingsPath -PathType Leaf)) { throw 'EVIDENCE_BINDINGS_MISSING' }
$registry = Get-Content -LiteralPath $BindingsPath -Raw -Encoding utf8 | ConvertFrom-Json
if ($registry.schema -ne 'mission-control-evidence-bindings-v1') { throw 'EVIDENCE_BINDINGS_SCHEMA_INVALID' }
$bindings = @($registry.bindings)
if ($bindings.Count -eq 0) { throw 'EVIDENCE_BINDINGS_EMPTY' }
$failures = [System.Collections.Generic.List[string]]::new()
$unavailable = [System.Collections.Generic.List[string]]::new()
$normalizedBindings = @{}
$bundleBindingNames = @{}
foreach ($binding in $bindings) {
    $relativePath = [string]$binding.evidencePath
    $path = if ([IO.Path]::IsPathRooted($relativePath)) { $relativePath } else { Join-Path $RepositoryRoot ($relativePath -replace '/', [IO.Path]::DirectorySeparatorChar) }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        $availabilityProperty = $binding.PSObject.Properties['availability']
        $availability = if ($null -eq $availabilityProperty) { 'source-required' } else { [string]$availabilityProperty.Value }
        if ($availability -eq 'local-artifact') {
            $unavailable.Add($relativePath)
        } else {
            $failures.Add("bound evidence missing: $relativePath")
        }
        continue
    }
    $expected = ([string]$binding.sha256).ToLowerInvariant()
    if ($expected -notmatch '^[a-f0-9]{64}$') { $failures.Add("bound evidence hash invalid: $relativePath"); continue }
    $actual = Get-Sha256Hex -Path $path
    if ($actual -ne $expected) { $failures.Add("bound evidence hash mismatch: $relativePath expected $expected actual $actual") }
    $leaf = Split-Path -Leaf $relativePath
    if (-not $normalizedBindings.ContainsKey($leaf)) { $normalizedBindings[$leaf] = [System.Collections.Generic.List[string]]::new() }
    $normalizedBindings[$leaf].Add($expected)
    $firstBundle = [string]$binding.firstBoundBundle
    if (-not [string]::IsNullOrWhiteSpace($firstBundle)) {
        if (-not $bundleBindingNames.ContainsKey($firstBundle)) { $bundleBindingNames[$firstBundle] = [System.Collections.Generic.List[string]]::new() }
        $bundleBindingNames[$firstBundle].Add($leaf)
    }
}
if (-not [string]::IsNullOrWhiteSpace($ReviewBundlePath)) {
    if (-not (Test-Path -LiteralPath $ReviewBundlePath -PathType Leaf)) {
        $failures.Add("review bundle missing: $ReviewBundlePath")
    } else {
        try {
            $bundle = Get-Content -LiteralPath $ReviewBundlePath -Raw -Encoding utf8 | ConvertFrom-Json
            $bundleArtifacts = @($bundle.artifacts)
            $bundleFullPath = (Resolve-Path -LiteralPath $ReviewBundlePath).Path
            foreach ($artifact in $bundleArtifacts) {
                $name = [string]$artifact.name
                $hashProperty = $artifact.PSObject.Properties['sha256']
                if ([string]::IsNullOrWhiteSpace($name) -or $null -eq $hashProperty) { continue }
                $bundleHash = ([string]$hashProperty.Value).ToLowerInvariant()
                if ($bundleHash -notmatch '^[a-f0-9]{64}$') { continue }
                $leafName = Split-Path -Leaf $name
                $matchingBindings = [System.Collections.Generic.List[object]]::new()
                foreach ($candidate in $bindings) {
                    if ((Split-Path -Leaf ([string]$candidate.evidencePath)) -ne $leafName) { continue }
                    $candidateBundle = [string]$candidate.firstBoundBundle
                    if ([string]::IsNullOrWhiteSpace($candidateBundle)) { continue }
                    try {
                        $candidateBundlePath = (Resolve-Path -LiteralPath (Join-Path $RepositoryRoot $candidateBundle)).Path
                        if ($candidateBundlePath -eq $bundleFullPath) { $matchingBindings.Add($candidate) }
                    } catch { continue }
                }
                if ($matchingBindings.Count -eq 0) { continue }
                $registeredHashes = @($matchingBindings | ForEach-Object { ([string]$_.sha256).ToLowerInvariant() } | Select-Object -Unique)
                if ($registeredHashes.Count -ne 1 -or $registeredHashes[0] -ne $bundleHash) {
                    $failures.Add("review bundle hash mismatch: $name expected $($registeredHashes -join ',') actual $bundleHash")
                }
            }
        } catch {
            $failures.Add("review bundle parse failed: $($_.Exception.Message)")
        }
    }
}
if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Error $_ }
    exit 1
}
([ordered]@{ schema = 'mission-control-evidence-bindings-v1'; status = 'pass'; registered = $bindings.Count; checked = $bindings.Count - $unavailable.Count; unavailableLocalArtifacts = @($unavailable); bindingsPath = $BindingsPath; reviewBundlePath = if ([string]::IsNullOrWhiteSpace($ReviewBundlePath)) { $null } else { $ReviewBundlePath } } | ConvertTo-Json -Compress)
exit 0
