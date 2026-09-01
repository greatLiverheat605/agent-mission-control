function Resolve-ReleaseDistribution {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidateSet('commercial', 'oss-personal')]
        [string] $Distribution,
        [Parameter(Mandatory)]
        [ValidateRange(0, [int]::MaxValue)]
        [int] $FailureCount,
        [Parameter(Mandatory)]
        [string] $SigningStatus,
        [Parameter(Mandatory)]
        [bool] $EvidenceReady,
        [Parameter(Mandatory)]
        [string] $RealInvocationStatus,
        [Parameter(Mandatory)]
        [string] $CleanVmStatus
    )

    $blockers = [System.Collections.Generic.List[string]]::new()
    $optionalEnhancements = [System.Collections.Generic.List[string]]::new()

    if ($Distribution -eq 'oss-personal') {
        if ($RealInvocationStatus -ne 'authorized-real') {
            $blockers.Add('real-provider-invocation')
        }
        if ($SigningStatus -ne 'signed-ci') {
            $optionalEnhancements.Add('signing-credentials')
        }
        if ($CleanVmStatus -ne 'verified') {
            $optionalEnhancements.Add('clean-vm')
        }
        $releaseReady = $FailureCount -eq 0 -and $RealInvocationStatus -eq 'authorized-real'
    } else {
        if ($SigningStatus -ne 'signed-ci') {
            $blockers.Add('signing-credentials')
        }
        if ($RealInvocationStatus -ne 'authorized-real') {
            $blockers.Add('real-provider-invocation')
        }
        if ($CleanVmStatus -ne 'verified') {
            $blockers.Add('clean-vm')
        }
        $releaseReady = $FailureCount -eq 0 -and
            $SigningStatus -eq 'signed-ci' -and
            $EvidenceReady -and
            $RealInvocationStatus -eq 'authorized-real' -and
            $CleanVmStatus -eq 'verified'
    }

    [pscustomobject][ordered]@{
        distribution = $Distribution
        releaseReady = $releaseReady
        formalReleaseBlockedBy = @($blockers)
        optionalEnhancements = @($optionalEnhancements)
    }
}
