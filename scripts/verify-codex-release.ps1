[CmdletBinding()]
param(
    [string] $RunId = ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')), 
    [ValidateSet('preview', 'internal')]
    [string] $Channel = 'preview',
    [ValidateSet('commercial', 'oss-personal')]
    [string] $Distribution = 'oss-personal',
    [switch] $SkipPackageBuild,
    [string] $RealInvocationReport = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'release-distribution.ps1')
$gateRoot = Join-Path $repositoryRoot "artifacts\release-gate\$RunId"
$reportPath = Join-Path $gateRoot 'codex-release-gate.json'
if (Test-Path -LiteralPath $reportPath -PathType Leaf) { throw 'OUTPUT_EXISTS: refusing to overwrite a release gate report.' }
New-Item -ItemType Directory -Force -Path $gateRoot | Out-Null
$stepOutputRoot = Join-Path $gateRoot 'steps'
New-Item -ItemType Directory -Force -Path $stepOutputRoot | Out-Null
$failures = [System.Collections.Generic.List[string]]::new()
$results = [System.Collections.Generic.List[object]]::new()

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string]$Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes($Path))) -replace '-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Invoke-Gate {
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$File, [string[]]$Arguments = @())
    $previousErrorActionPreference = $ErrorActionPreference
    $safeName = ($Name -replace '[^A-Za-z0-9._-]', '-')
    $outputPath = Join-Path $stepOutputRoot "$safeName.log"
    try {
        # Windows PowerShell 5.1 surfaces native stderr (for example cargo's
        # build progress) as an ErrorRecord when preference is Stop. Keep the
        # stream non-terminating here and rely on LASTEXITCODE for the gate.
        $ErrorActionPreference = 'Continue'
        & $File @Arguments *> $outputPath
        $exitCode = if ($null -eq $LASTEXITCODE) { 1 } else { $LASTEXITCODE }
    } catch {
        $exitCode = 1
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $results.Add([ordered]@{ name = $Name; status = if ($exitCode -eq 0) { 'pass' } else { 'fail' }; exitCode = $exitCode; outputPath = $outputPath.Substring($repositoryRoot.Length).TrimStart([char]92, [char]47) })
    if ($exitCode -ne 0) { $failures.Add("$Name exited $exitCode") }
}

Set-Location -LiteralPath $repositoryRoot
Invoke-Gate 'cargo fmt' 'cargo.exe' @('fmt', '--all', '--', '--check')
Invoke-Gate 'cargo clippy' 'cargo.exe' @('clippy', '-p', 'mission-control-desktop', '--all-targets', '--locked', '--offline', '--', '-D', 'warnings')
Invoke-Gate 'ledger and supervisor tests' 'cargo.exe' @('test', '-p', 'mission-ledger', '-p', 'mission-supervisor', '--locked', '--offline')
Invoke-Gate 'desktop typecheck' 'npm.cmd' @('run', 'typecheck', '--workspace', '@mission-control/desktop')
Invoke-Gate 'desktop Vitest' 'npm.cmd' @('run', 'test', '--workspace', '@mission-control/desktop', '--', '--run')
Invoke-Gate 'contract/integration/security/recovery/soak/preview tests' 'node.exe' @('--test', 'tests/contract/protocol-bindings.test.mjs', 'tests/integration/acceptance-merge.test.mjs', 'tests/integration/ui-disconnect.test.mjs', 'tests/security/codex-security.test.mjs', 'tests/recovery/recovery-matrix.test.mjs', 'tests/soak/three-mission-codex.test.mjs', 'tests/preview/codex-task-matrix.test.mjs', 'tests/preview/codex-preview.test.mjs', 'tests/release/open-source-go-live.test.mjs')
Invoke-Gate 'security gate' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/security-gate.ps1'))
Invoke-Gate 'immutable evidence bindings' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/verify-evidence-bindings.ps1'), '-BindingsPath', (Join-Path $repositoryRoot 'docs/release/evidence-bindings.json'), '-RepositoryRoot', $repositoryRoot)
$crashReportPath = Join-Path $gateRoot 'crash-matrix.json'
Invoke-Gate 'crash matrix' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/run-crash-matrix.ps1'), '-DurationMinutes', '1', '-OutputPath', $crashReportPath)
$soakReportPath = Join-Path $gateRoot 'soak.json'
Invoke-Gate 'short Codex soak' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/run-codex-soak.ps1'), '-DurationMinutes', '1', '-OutputPath', $soakReportPath)

if (-not $SkipPackageBuild) { Invoke-Gate 'signed package smoke' 'npm.cmd' @('run', 'package:smoke') }
$releaseChannelRoot = Join-Path (Join-Path $repositoryRoot 'artifacts\codex-release') $Channel
$manifestCandidate = Get-ChildItem -LiteralPath $releaseChannelRoot -Filter manifest.json -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Directory.Parent.Name -eq $Channel } |
    Sort-Object { [version]$_.Directory.Name } -Descending | Select-Object -First 1
$manifestPath = $null
$artifact = $null
if ($manifestCandidate) {
    $candidateManifest = Get-Content -LiteralPath $manifestCandidate.FullName -Raw -Encoding utf8 | ConvertFrom-Json
    $candidateArtifactPath = Join-Path $manifestCandidate.DirectoryName ([string]$candidateManifest.artifact)
    $candidateSbomPath = Join-Path $manifestCandidate.DirectoryName ([string]$candidateManifest.sbom)
    $candidateProvenancePath = Join-Path $manifestCandidate.DirectoryName ([string]$candidateManifest.provenance)
    $candidateSbom = if (Test-Path -LiteralPath $candidateSbomPath -PathType Leaf) { Get-Content -LiteralPath $candidateSbomPath -Raw -Encoding utf8 | ConvertFrom-Json } else { $null }
    $candidateProvenance = if (Test-Path -LiteralPath $candidateProvenancePath -PathType Leaf) { Get-Content -LiteralPath $candidateProvenancePath -Raw -Encoding utf8 | ConvertFrom-Json } else { $null }
    $candidateSbomHash = if ($candidateSbom -and ($candidateSbom.PSObject.Properties.Name -contains 'artifactSha256')) { [string]$candidateSbom.artifactSha256 } else { '' }
    $candidateProvenanceHash = if ($candidateProvenance -and ($candidateProvenance.PSObject.Properties.Name -contains 'artifactSha256')) { [string]$candidateProvenance.artifactSha256 } else { '' }
    $candidateMetadataReady = (Test-Path -LiteralPath $candidateSbomPath -PathType Leaf) -and (Test-Path -LiteralPath $candidateProvenancePath -PathType Leaf) -and
        ([string]$candidateManifest.artifactSha256 -match '^[a-f0-9]{64}$') -and
        ($candidateSbomHash -match '^[a-f0-9]{64}$') -and ($candidateProvenanceHash -match '^[a-f0-9]{64}$')
    if ((Test-Path -LiteralPath $candidateArtifactPath -PathType Leaf) -and $candidateMetadataReady) {
        $manifestPath = $manifestCandidate.FullName
        $artifact = Get-Item -LiteralPath $candidateArtifactPath
    }
}
if (-not $artifact) {
    $artifact = Get-ChildItem -LiteralPath (Join-Path $repositoryRoot 'target\release\bundle') -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Extension -eq '.msi' -and $_.Name -notmatch 'smoke|supervisor' } |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}
if (-not $artifact) { $failures.Add('release artifact not found'); $results.Add([ordered]@{ name = 'release artifact'; status = 'fail'; exitCode = 1 }) }

if (-not $manifestPath -and $artifact) {
    $buildArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/build-codex-release.ps1'), '-Channel', $Channel, '-SkipBuild', '-ArtifactPath', $artifact.FullName)
    Invoke-Gate 'Codex release manifest' 'powershell.exe' $buildArgs
    $manifestPath = Join-Path $repositoryRoot "artifacts\codex-release\$Channel\0.1.0\manifest.json"
    $publishedPath = Join-Path (Split-Path -Parent $manifestPath) ((Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json).artifact)
    if (Test-Path -LiteralPath $publishedPath -PathType Leaf) { $artifact = Get-Item -LiteralPath $publishedPath }
}

$sbomPath = Join-Path $gateRoot 'sbom.json'
$sbomArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/generate-sbom.ps1'), '-OutputPath', $sbomPath, '-Version', '0.1.0', '-Channel', $Channel)
if ($artifact) { $sbomArgs += @('-ArtifactPath', $artifact.FullName) }
Invoke-Gate 'SBOM' 'powershell.exe' $sbomArgs

$signingReady = -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable('MISSION_CONTROL_SIGNING_THUMBPRINT')) -and
    -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable('MISSION_CONTROL_SIGNING_PFX_BASE64')) -and
    -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable('MISSION_CONTROL_SIGNING_PFX_PASSWORD')) -and
    (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable('TAURI_UPDATER_SIGNATURE')) -or -not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY')))
$signingStatus = 'deferred-ci'
$manifestForSigning = $null
if ($manifestPath -and (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    $manifestForSigning = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
}
if ($signingReady -and $artifact -and $manifestPath -and ($null -eq $manifestForSigning -or [string]::IsNullOrWhiteSpace([string]$manifestForSigning.signature) -or [string]$manifestForSigning.signerFingerprint -eq 'UNSIGNED')) {
    Invoke-Gate 'Authenticode and Tauri signing' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/sign-release.ps1'), '-ArtifactPath', $artifact.FullName, '-ManifestPath', $manifestPath, '-CertificateThumbprint', [Environment]::GetEnvironmentVariable('MISSION_CONTROL_SIGNING_THUMBPRINT'))
} elseif (-not $signingReady) {
    $results.Add([ordered]@{ name = 'signing credentials'; status = 'deferred-ci'; exitCode = 0 })
}
if ($manifestPath -and (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    $manifestForSigning = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
    if (-not [string]::IsNullOrWhiteSpace([string]$manifestForSigning.signature) -and [string]$manifestForSigning.signerFingerprint -ne 'UNSIGNED') { $signingStatus = 'signed-ci' }
}
if ($manifestPath) {
    $fixtureArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/test-update-rollback.ps1'), '-ManifestPath', $manifestPath, '-FixtureMode')
    Invoke-Gate 'update rollback fixture' 'powershell.exe' $fixtureArgs
    if (-not $signingReady) {
        $previousErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $unsignedOutputPath = Join-Path $stepOutputRoot 'unsigned-rejection-selfcheck.log'
            & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repositoryRoot 'scripts/test-update-rollback.ps1') -ManifestPath $manifestPath *> $unsignedOutputPath
            $unsignedExitCode = if ($null -eq $LASTEXITCODE) { 1 } else { $LASTEXITCODE }
        } catch {
            $unsignedExitCode = 1
        } finally {
            $ErrorActionPreference = $previousErrorActionPreference
        }
        if ($unsignedExitCode -eq 0) { $failures.Add('unsigned update was accepted'); $results.Add([ordered]@{ name = 'unsigned update rejection'; status = 'fail'; exitCode = 1 }) }
        else { $results.Add([ordered]@{ name = 'unsigned update rejection'; status = 'pass'; exitCode = $unsignedExitCode; outputPath = $unsignedOutputPath.Substring($repositoryRoot.Length).TrimStart([char]92, [char]47) }) }
    }
}

$taskMatrixReportPath = Join-Path $gateRoot 'task-matrix-report.json'
Invoke-Gate 'controlled task matrix' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/summarize-codex-matrix.ps1'), '-FixtureMode', '-EvidenceRoot', (Join-Path $gateRoot 'task-matrix'), '-OutputPath', $taskMatrixReportPath)
$previewReportPath = Join-Path $gateRoot 'preview-report.json'
Invoke-Gate 'controlled preview thresholds' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repositoryRoot 'scripts/summarize-codex-preview.ps1'), '-FixtureMode', '-SamplesPath', (Join-Path $gateRoot 'preview-samples'), '-OutputPath', $previewReportPath)

function Read-EvidenceReport {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Name)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        $failures.Add("$Name report missing")
        $results.Add([ordered]@{ name = "$Name evidence"; status = 'fail'; exitCode = 1 })
        return $null
    }
    try {
        return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        $failures.Add("$Name report invalid JSON")
        $results.Add([ordered]@{ name = "$Name evidence"; status = 'fail'; exitCode = 1 })
        return $null
    }
}

$crashEvidence = Read-EvidenceReport -Path $crashReportPath -Name 'crash matrix'
$soakEvidence = Read-EvidenceReport -Path $soakReportPath -Name 'Codex soak'
$taskMatrixEvidence = Read-EvidenceReport -Path $taskMatrixReportPath -Name 'task matrix'
$previewEvidence = Read-EvidenceReport -Path $previewReportPath -Name 'preview'
$controlledEvidenceValid = $true
foreach ($evidence in @(
    [ordered]@{ name = 'crash matrix'; value = $crashEvidence },
    [ordered]@{ name = 'Codex soak'; value = $soakEvidence },
    [ordered]@{ name = 'task matrix'; value = $taskMatrixEvidence },
    [ordered]@{ name = 'preview'; value = $previewEvidence }
)) {
    if ($null -eq $evidence.value) { $controlledEvidenceValid = $false; continue }
    $modeProperty = $evidence.value.PSObject.Properties['mode']
    $mode = if ($null -eq $modeProperty) { $null } else { [string]$modeProperty.Value }
    $releaseReadyProperty = $evidence.value.PSObject.Properties['releaseReady']
    $releaseReadyValue = if ($null -eq $releaseReadyProperty) { $null } else { [bool]$releaseReadyProperty.Value }
    $realInvocationProperty = $evidence.value.PSObject.Properties['realInvocation']
    if ($null -eq $realInvocationProperty) { $realInvocationProperty = $evidence.value.PSObject.Properties['real_invocation'] }
    $realInvocation = if ($null -eq $realInvocationProperty) { $null } else { [string]$realInvocationProperty.Value }
    if ($mode -ne 'controlled' -or $realInvocation -ne 'deferred' -or $null -eq $releaseReadyProperty -or $releaseReadyValue -ne $false) {
        $controlledEvidenceValid = $false
        $failures.Add("$($evidence.name) evidence boundary invalid")
    }
    $statusProperty = $evidence.value.PSObject.Properties['status']
    if ($null -ne $statusProperty -and $evidence.value.status -eq 'fail') { $controlledEvidenceValid = $false }
}
if ($null -eq $taskMatrixEvidence -or -not ($taskMatrixEvidence.PSObject.Properties.Name -contains 'evidence') -or @($taskMatrixEvidence.evidence).Count -eq 0 -or [int]$taskMatrixEvidence.totalTasks -le 0) {
    $controlledEvidenceValid = $false
    $failures.Add('task matrix evidence is empty')
    $results.Add([ordered]@{ name = 'task matrix non-empty evidence'; status = 'fail'; exitCode = 1 })
}
if ($null -eq $previewEvidence -or -not ($previewEvidence.PSObject.Properties.Name -contains 'samples') -or @($previewEvidence.samples).Count -eq 0 -or [int]$previewEvidence.participants -le 0) {
    $controlledEvidenceValid = $false
    $failures.Add('preview evidence is empty')
    $results.Add([ordered]@{ name = 'preview non-empty evidence'; status = 'fail'; exitCode = 1 })
}
$hashConsistencyValid = $true
if ($artifact -and $manifestPath -and (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    try {
        $releaseRoot = Split-Path -Parent $manifestPath
        $releaseManifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
        $releaseProvenancePath = Join-Path $releaseRoot ([string]$releaseManifest.provenance)
        $releaseSbomPath = Join-Path $releaseRoot ([string]$releaseManifest.sbom)
        $releaseProvenance = Get-Content -LiteralPath $releaseProvenancePath -Raw -Encoding utf8 | ConvertFrom-Json
        $releaseSbom = Get-Content -LiteralPath $releaseSbomPath -Raw -Encoding utf8 | ConvertFrom-Json
        $actualArtifactHash = Get-Sha256Hex -Path $artifact.FullName
        $hashValues = @{
            'manifest.artifactSha256' = [string]$releaseManifest.artifactSha256
            'provenance.artifactSha256' = [string]$releaseProvenance.artifactSha256
            'release sbom.artifactSha256' = [string]$releaseSbom.artifactSha256
            'gate sbom.artifactSha256' = [string]$((Get-Content -LiteralPath $sbomPath -Raw -Encoding utf8 | ConvertFrom-Json).artifactSha256)
        }
        foreach ($entry in $hashValues.GetEnumerator()) {
            if ($entry.Value -ne $actualArtifactHash) { $hashConsistencyValid = $false; $failures.Add("$($entry.Key) does not match artifact SHA-256") }
        }
    } catch {
        $hashConsistencyValid = $false
        $failures.Add("artifact hash consistency check failed: $($_.Exception.Message)")
    }
} else {
    $hashConsistencyValid = $false
    $failures.Add('release manifest missing for hash consistency check')
}
$results.Add([ordered]@{ name = 'manifest/provenance/sbom hash consistency'; status = if ($hashConsistencyValid) { 'pass' } else { 'fail' }; exitCode = if ($hashConsistencyValid) { 0 } else { 1 } })
$evidenceReports = @($crashEvidence, $soakEvidence, $taskMatrixEvidence, $previewEvidence)
$realEvidenceReports = @($evidenceReports | Where-Object {
    if ($null -eq $_) { return $false }
    $modeProperty = $_.PSObject.Properties['mode']
    $mode = if ($null -eq $modeProperty) { $null } else { [string]$modeProperty.Value }
    $realInvocationProperty = $_.PSObject.Properties['realInvocation']
    if ($null -eq $realInvocationProperty) { $realInvocationProperty = $_.PSObject.Properties['real_invocation'] }
    $releaseReadyProperty = $_.PSObject.Properties['releaseReady']
    $releaseReady = if ($null -eq $releaseReadyProperty) { $false } else { [bool]$releaseReadyProperty.Value }
    $mode -ne 'controlled' -and $null -ne $realInvocationProperty -and [string]$realInvocationProperty.Value -eq 'executed' -and $releaseReady
})
$evidenceReady = ($controlledEvidenceValid -and $realEvidenceReports.Count -eq 4)
$evidenceStatus = if ($evidenceReady) { 'ready' } elseif ($controlledEvidenceValid) { 'controlled-deferred' } else { 'invalid' }
$results.Add([ordered]@{ name = 'release evidence readiness'; status = if ($evidenceReady) { 'pass' } else { 'deferred-ci' }; exitCode = 0 })
$controlledInternalPreview = ($failures.Count -eq 0 -and $controlledEvidenceValid)
$releaseDecision = if ($controlledInternalPreview) { 'PASS_INTERNAL_PREVIEW' } else { 'BLOCK_INTERNAL_PREVIEW' }
$realInvocationStatus = 'deferred'
$invocationTransport = 'app-server'
$realEvidenceValid = $false
$evidenceHygieneValid = $true
if (-not [string]::IsNullOrWhiteSpace($RealInvocationReport)) {
    if (-not (Test-Path -LiteralPath $RealInvocationReport -PathType Leaf)) {
        $failures.Add('real invocation report missing')
        $results.Add([ordered]@{ name = 'real invocation evidence'; status = 'fail'; exitCode = 1 })
    } else {
        try {
            $realReport = Get-Content -LiteralPath $RealInvocationReport -Raw -Encoding utf8 | ConvertFrom-Json
            $budgetExceededProperty = $realReport.PSObject.Properties['budgetExceeded']
            $budgetExceededValue = if ($null -eq $budgetExceededProperty) { $false } else { [bool]$budgetExceededProperty.Value }
            $transportProperty = $realReport.PSObject.Properties['invocationTransport']
            $transportValue = if ($null -eq $transportProperty) { $null } else { [string]$transportProperty.Value }
            $reportedUsageTotal = $null
            $actualUsageProperty = $realReport.PSObject.Properties['actualUsage']
            if ($null -ne $actualUsageProperty -and $null -ne $actualUsageProperty.Value) {
                $actualTotalProperty = $actualUsageProperty.Value.PSObject.Properties['totalTokens']
                if ($null -ne $actualTotalProperty) { $reportedUsageTotal = [long]$actualTotalProperty.Value }
            }
            if ($null -eq $reportedUsageTotal) {
                $tokenUsageProperty = $realReport.PSObject.Properties['tokenUsage']
                if ($null -ne $tokenUsageProperty -and $tokenUsageProperty.Value -isnot [string]) {
                    $tokenTotalProperty = $tokenUsageProperty.Value.PSObject.Properties['totalTokens']
                    if ($null -ne $tokenTotalProperty) { $reportedUsageTotal = [long]$tokenTotalProperty.Value }
                }
            }
            $tokenLimitProperty = $realReport.PSObject.Properties['tokenLimit']
            $withinTokenLimit = $true
            if ($null -ne $reportedUsageTotal -and $null -ne $tokenLimitProperty) {
                $withinTokenLimit = $reportedUsageTotal -le [long]$tokenLimitProperty.Value
            }
             $realValid = [string]$realReport.mode -eq 'authorized-real' -and
                [string]$realReport.realInvocation -eq 'authorized-real' -and
                [bool]$realReport.redacted -and
                 -not [bool]$realReport.releaseReady -and
                 ($transportValue -in @('exec', 'app-server')) -and
                 -not $budgetExceededValue -and
                 $withinTokenLimit
             if (-not $realValid) { throw 'real invocation report is not valid authorized evidence' }
             $realEvidenceValid = $true
             $invocationTransport = $transportValue
             $realInvocationStatus = 'authorized-real'
             $results.Add([ordered]@{ name = 'real invocation evidence'; status = 'pass'; exitCode = 0; outputPath = $RealInvocationReport })
        } catch {
            $failures.Add("real invocation evidence invalid: $($_.Exception.Message)")
            $results.Add([ordered]@{ name = 'real invocation evidence'; status = 'fail'; exitCode = 1; outputPath = $RealInvocationReport })
        }
    }
} else {
    $results.Add([ordered]@{ name = 'real invocation evidence'; status = 'deferred'; exitCode = 0 })
}

if (-not [string]::IsNullOrWhiteSpace($RealInvocationReport) -and (Test-Path -LiteralPath $RealInvocationReport -PathType Leaf)) {
    $realReportDirectory = Split-Path -Parent (Resolve-Path -LiteralPath $RealInvocationReport).Path
    $gateLocalReport = Join-Path $gateRoot 'real-invocation-report.json'
    if (Test-Path -LiteralPath $gateLocalReport -PathType Leaf) {
        if ((Get-Sha256Hex -Path $gateLocalReport) -ne (Get-Sha256Hex -Path $RealInvocationReport)) {
            $evidenceHygieneValid = $false
            $failures.Add('same-name evidence mismatch: gate real-invocation-report.json differs from referenced report')
        }
    }
    try {
        $referencedReport = Get-Content -LiteralPath $RealInvocationReport -Raw -Encoding UTF8 | ConvertFrom-Json
        $evidenceName = [string]$referencedReport.evidencePath
        if (-not [string]::IsNullOrWhiteSpace($evidenceName)) {
            $referencedEvidence = Join-Path $realReportDirectory $evidenceName
            $gateEvidence = Join-Path $gateRoot (Split-Path -Leaf $evidenceName)
            if ((Test-Path -LiteralPath $referencedEvidence -PathType Leaf) -and (Test-Path -LiteralPath $gateEvidence -PathType Leaf) -and
                ((Get-Sha256Hex -Path $referencedEvidence) -ne (Get-Sha256Hex -Path $gateEvidence))) {
                $evidenceHygieneValid = $false
                $failures.Add("same-name evidence mismatch: $evidenceName differs from gate copy")
            }
        }
    } catch {
        $evidenceHygieneValid = $false
        $failures.Add("evidence hygiene report parse failed: $($_.Exception.Message)")
    }
}
foreach ($nameGroup in @(Get-ChildItem -LiteralPath $gateRoot -Recurse -File -ErrorAction SilentlyContinue | Group-Object Name | Where-Object { $_.Count -gt 1 })) {
    $hashes = @($nameGroup.Group | ForEach-Object { Get-Sha256Hex -Path $_.FullName } | Select-Object -Unique)
    if ($hashes.Count -gt 1) {
        $evidenceHygieneValid = $false
        $failures.Add("same-name evidence mismatch inside gate directory: $($nameGroup.Name)")
    }
}
$results.Add([ordered]@{ name = 'same-name evidence hash consistency'; status = if ($evidenceHygieneValid) { 'pass' } else { 'fail' }; exitCode = if ($evidenceHygieneValid) { 0 } else { 1 } })
$cleanVmStatus = 'blocked'

$forbidden = '(?i)^(?:\.codex|handoff|prompt|screenshots?|evidence)(?:[\\/]|$)|(?:^|[\\/])AGENTS\.md$|\.log$'
$staged = @(git diff --cached --name-only)
$remote = @(git ls-tree -r --name-only origin/main)
$allowlistFailures = @($staged + $remote | Where-Object { $_ -match $forbidden } | Select-Object -Unique)
if ($allowlistFailures.Count -gt 0) { $failures.Add("remote allowlist rejected: $($allowlistFailures -join ', ')"); $results.Add([ordered]@{ name = 'remote allowlist'; status = 'fail'; exitCode = 1 }) }
elseif ((git rev-parse HEAD).Trim() -ne (git rev-parse origin/main).Trim()) { $failures.Add('origin/main does not match HEAD'); $results.Add([ordered]@{ name = 'remote allowlist'; status = 'fail'; exitCode = 1 }) }
else { $results.Add([ordered]@{ name = 'remote allowlist'; status = 'pass'; exitCode = 0 }) }

$distributionDecision = Resolve-ReleaseDistribution -Distribution $Distribution -FailureCount $failures.Count -SigningStatus $signingStatus -EvidenceReady $evidenceReady -RealInvocationStatus $realInvocationStatus -CleanVmStatus $cleanVmStatus
$releaseReady = [bool]$distributionDecision.releaseReady
$formalReleaseBlockedBy = @($distributionDecision.formalReleaseBlockedBy)
$optionalEnhancements = @($distributionDecision.optionalEnhancements)

function Get-ResultStatus([string]$Name) {
    $entry = @($results | Where-Object { [string]$_.name -eq $Name }) | Select-Object -First 1
    if ($null -eq $entry) { return 'missing' }
    return [string]$entry.status
}

$round6Verification = [ordered]@{
    cargoFmt = Get-ResultStatus 'cargo fmt'
    cargoClippy = Get-ResultStatus 'cargo clippy'
    cargoTest = Get-ResultStatus 'ledger and supervisor tests'
    desktopTypecheck = Get-ResultStatus 'desktop typecheck'
    desktopVitest = Get-ResultStatus 'desktop Vitest'
    nodeTests = Get-ResultStatus 'contract/integration/security/recovery/soak/preview tests'
    securityGate = Get-ResultStatus 'security gate'
}

$report = [ordered]@{
    schema = 'mission-control-codex-release-gate-v1'
    product = 'Agent Mission Control'
    channel = $Channel
    distribution = $Distribution
    version = '0.1.0'
    generatedAtUtc = [DateTime]::UtcNow.ToString('o')
    codexOnly = $true
    claude = 'deferred'
    realInvocation = $realInvocationStatus
    realValid = $realEvidenceValid
    invocationTransport = $invocationTransport
    signingStatus = $signingStatus
    evidenceStatus = $evidenceStatus
    evidenceReady = $evidenceReady
    releaseReady = $releaseReady
    controlledInternalPreview = $controlledInternalPreview
    releaseDecision = $releaseDecision
    formalReleaseBlockedBy = @($formalReleaseBlockedBy)
    optionalEnhancements = @($optionalEnhancements)
    failures = @($failures)
    gates = @($results)
    round6Verification = $round6Verification
    artifacts = [ordered]@{ releaseArtifact = if ($artifact) { $artifact.FullName } else { $null }; sbom = $sbomPath; soak = $soakReportPath; crashMatrix = $crashReportPath; taskMatrix = $taskMatrixReportPath; preview = $previewReportPath; realInvocationReport = if ([string]::IsNullOrWhiteSpace($RealInvocationReport)) { $null } else { (Resolve-Path -LiteralPath $RealInvocationReport).Path } }
}
$report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $reportPath -Encoding utf8
Write-Output $reportPath
if ($failures.Count -gt 0) { exit 1 }
exit 0
