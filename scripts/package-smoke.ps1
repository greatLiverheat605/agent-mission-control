[CmdletBinding()]
param(
    [string] $BundlePath,

    [string] $PipeName = "mission-package-smoke-$([guid]::NewGuid().ToString('N'))",

    [string] $TimeoutSeconds = '15',

    [string] $InstallTimeoutSeconds = '120'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$protocolVersion = 1
$workRoot = Join-Path ([IO.Path]::GetTempPath()) "mission-package-smoke-$([guid]::NewGuid().ToString('N'))"
$installRoot = Join-Path $workRoot 'installed'
$dataRoot = Join-Path $workRoot 'data'
$supervisorProcess = $null
$installerProcess = $null
$failure = $null
$result = $null

function Throw-PackageSmokeFailure {
    param(
        [Parameter(Mandatory)]
        [string] $Code,

        [Parameter(Mandatory)]
        [string] $Message
    )

    $exception = New-Object System.InvalidOperationException -ArgumentList $Message
    $exception.Data['PackageSmokeCode'] = $Code
    throw $exception
}

function Get-FailureCode {
    param([Parameter(Mandatory)] $ErrorRecord)

    $code = $ErrorRecord.Exception.Data['PackageSmokeCode']
    if ($code) { return [string] $code }
    'PACKAGE_SMOKE_UNEXPECTED'
}

function Resolve-BoundedInteger {
    param(
        [Parameter(Mandatory)][string] $Value,
        [Parameter(Mandatory)][int] $Minimum,
        [Parameter(Mandatory)][int] $Maximum
    )

    $parsed = 0
    if (-not [int]::TryParse(
            $Value,
            [Globalization.NumberStyles]::Integer,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref] $parsed
        ) -or $parsed -lt $Minimum -or $parsed -gt $Maximum) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_ARGUMENT_INVALID' -Message 'A package smoke timeout argument is invalid.'
    }
    $parsed
}

function Stop-OwnedProcess {
    param(
        [Parameter(Mandatory)] $Process,
        [Parameter(Mandatory)][string] $Description,
        [int] $TimeoutMilliseconds = 5000
    )

    if (-not $Process.HasExited) {
        $Process.Kill()
        if (-not $Process.WaitForExit($TimeoutMilliseconds)) {
            throw "$Description did not exit during cleanup."
        }
    }
}

function Throw-PackageCleanupFailure {
    param(
        [Parameter(Mandatory)][string] $OriginalCode,
        [Parameter(Mandatory)][string[]] $FailedActions
    )

    $names = @($FailedActions | ForEach-Object { ([string] $_).Split(':')[0] })
    Throw-PackageSmokeFailure -Code 'PACKAGE_CLEANUP_FAILED' `
        -Message "Package cleanup failed for: $($names -join ', '); original=$OriginalCode."
}

function Resolve-BundlePath {
    param([string] $RequestedPath)

    if ($RequestedPath) {
        if (-not (Test-Path -LiteralPath $RequestedPath)) {
            Throw-PackageSmokeFailure -Code 'PACKAGE_ARTIFACT_NOT_FOUND' -Message 'The requested package artifact does not exist.'
        }
        return [IO.Path]::GetFullPath($RequestedPath)
    }

    $root = Split-Path -Parent $PSScriptRoot
    $msiRoot = Join-Path $root 'target\release\bundle\msi'
    $bundles = @(Get-ChildItem -LiteralPath $msiRoot -Filter '*.msi' -File -ErrorAction SilentlyContinue)
    if ($bundles.Count -eq 0) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_ARTIFACT_NOT_FOUND' -Message 'No Tauri MSI package artifact was found.'
    }
    if ($bundles.Count -ne 1) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_ARTIFACT_AMBIGUOUS' -Message 'More than one Tauri MSI package artifact was found.'
    }
    $bundles[0].FullName
}

function Copy-ExtractedArtifact {
    param(
        [Parameter(Mandatory)]
        [string] $Source,

        [Parameter(Mandatory)]
        [string] $Destination
    )

    [void] @(Get-PackageTreeEntries -Root $Source)
    foreach ($entry in Get-ChildItem -LiteralPath $Source -Force) {
        Copy-Item -LiteralPath $entry.FullName -Destination $Destination -Recurse -Force
    }
}

function Expand-MsiArtifact {
    param(
        [Parameter(Mandatory)]
        [string] $MsiPath,

        [Parameter(Mandatory)]
        [string] $Destination,

        [Parameter(Mandatory)]
        [int] $Timeout
    )

    $msiexec = (Get-Command msiexec.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (-not $msiexec) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_INSTALL_TOOL_MISSING' -Message 'msiexec.exe is not available.'
    }

    $logPath = Join-Path $workRoot 'msiexec.log'
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $msiexec
    $startInfo.Arguments = "/a `"$MsiPath`" /qn TARGETDIR=`"$Destination`" /L*v `"$logPath`""
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true

    $script:installerProcess = New-Object System.Diagnostics.Process
    $script:installerProcess.StartInfo = $startInfo
    if (-not $script:installerProcess.Start()) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_INSTALL_FAILED' -Message 'The MSI administrative install did not start.'
    }
    if (-not $script:installerProcess.WaitForExit($Timeout * 1000)) {
        try {
            Stop-OwnedProcess -Process $script:installerProcess -Description 'The MSI installer'
        } catch {}
        Throw-PackageSmokeFailure -Code 'PACKAGE_INSTALL_TIMEOUT' -Message 'The MSI administrative install timed out.'
    }
    if ($script:installerProcess.ExitCode -ne 0) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_INSTALL_FAILED' -Message "The MSI administrative install exited with code $($script:installerProcess.ExitCode)."
    }
}

function Test-FileContainsFixtureSecret {
    param([Parameter(Mandatory)][string] $Path)

    $chunkSize = 65536
    $overlapLength = 'mission-control-fixture-secret'.Length - 1
    $buffer = New-Object byte[] $chunkSize
    $carry = ''
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $window = $carry + [Text.Encoding]::ASCII.GetString($buffer, 0, $read)
            if ($window -match '(?i)mission-control-fixture-secret|fixture[-_]secret') {
                return $true
            }
            $carry = $window.Substring([Math]::Max(0, $window.Length - $overlapLength))
        }
    } finally {
        $stream.Dispose()
    }
    $false
}

function Get-PackageTreeEntries {
    param([Parameter(Mandatory)][string] $Root)

    $rootItem = Get-Item -LiteralPath $Root -Force
    if (($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_REPARSE_POINT' -Message 'The package contains a reparse point.'
    }
    $pending = New-Object 'Collections.Generic.Queue[IO.DirectoryInfo]'
    $pending.Enqueue([IO.DirectoryInfo] $rootItem)
    while ($pending.Count -gt 0) {
        $directory = $pending.Dequeue()
        foreach ($entry in $directory.EnumerateFileSystemInfos()) {
            if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                Throw-PackageSmokeFailure -Code 'PACKAGE_REPARSE_POINT' -Message 'The package contains a reparse point.'
            }
            $entry
            if (($entry.Attributes -band [IO.FileAttributes]::Directory) -ne 0) {
                $pending.Enqueue([IO.DirectoryInfo] $entry)
            }
        }
    }
}

function Assert-PackageContents {
    param([Parameter(Mandatory)][string] $Root)

    $entries = @(Get-PackageTreeEntries -Root $Root)
    $mcp = $entries | Where-Object {
        ($_.Attributes -band [IO.FileAttributes]::Directory) -ne 0 -and $_.Name -ieq 'MCP'
    } | Select-Object -First 1
    if ($mcp) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_MCP' -Message 'The package contains a forbidden MCP directory.'
    }

    $files = @($entries | Where-Object {
            ($_.Attributes -band [IO.FileAttributes]::Directory) -eq 0
        })
    $environmentFile = $files | Where-Object Name -Match '^\.env(?:\.|$)' | Select-Object -First 1
    if ($environmentFile) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_ENV' -Message 'The package contains a forbidden environment file.'
    }

    $secretPath = $entries |
        Where-Object Name -Match '(?i)fixture[-_. ]?secret|secret[-_. ]?fixture' | Select-Object -First 1
    if ($secretPath) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_SECRET' -Message 'The package contains fixture secret material.'
    }

    $smokeClient = $files | Where-Object Name -IEQ 'mission-control-package-smoke.exe' | Select-Object -First 1
    if ($smokeClient) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_HELPER' -Message 'The package contains a build-only smoke client.'
    }

    foreach ($file in $files) {
        if (Test-FileContainsFixtureSecret -Path $file.FullName) {
            Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_SECRET' -Message 'The package contains fixture secret material.'
        }
    }
}

function Get-SingleBinary {
    param(
        [Parameter(Mandatory)]
        [string] $Root,

        [Parameter(Mandatory)]
        [string] $Name,

        [Parameter(Mandatory)]
        [string] $MissingCode
    )

    $matches = @(Get-PackageTreeEntries -Root $Root | Where-Object {
            ($_.Attributes -band [IO.FileAttributes]::Directory) -eq 0 -and $_.Name -ieq $Name
        })
    if ($matches.Count -eq 0) {
        Throw-PackageSmokeFailure -Code $MissingCode -Message "The package does not contain $Name."
    }
    if ($matches.Count -ne 1) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_BINARY_AMBIGUOUS' -Message "The package contains more than one $Name."
    }
    $matches[0].FullName
}

function New-BinarySnapshot {
    param([Parameter(Mandatory)][string] $Path)

    $file = Get-Item -LiteralPath $Path -Force
    if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_BINARY_CHANGED' -Message 'A package binary changed after scanning.'
    }
    [pscustomobject]@{
        Path = [IO.Path]::GetFullPath($file.FullName)
        Length = [int64] $file.Length
        LastWriteTicks = [int64] $file.LastWriteTimeUtc.Ticks
        CreationTicks = [int64] $file.CreationTimeUtc.Ticks
    }
}

function Assert-BinarySnapshot {
    param([Parameter(Mandatory)] $Snapshot)

    try {
        $current = New-BinarySnapshot -Path $Snapshot.Path
    } catch {
        Throw-PackageSmokeFailure -Code 'PACKAGE_BINARY_CHANGED' -Message 'A package binary changed after scanning.'
    }
    if ($current.Path -cne $Snapshot.Path -or
        $current.Length -ne $Snapshot.Length -or
        $current.LastWriteTicks -ne $Snapshot.LastWriteTicks -or
        $current.CreationTicks -ne $Snapshot.CreationTicks) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_BINARY_CHANGED' -Message 'A package binary changed after scanning.'
    }
}

function Convert-PackageSmokeClientResult {
    param(
        [Parameter(Mandatory)][int] $ExitCode,
        [AllowEmptyString()][string] $StdOut,
        [AllowEmptyString()][string] $StdErr
    )

    if ($StdErr -ne '') {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The Rust smoke client emitted unexpected stderr.'
    }
    $line = [string] $StdOut
    if ($line.EndsWith("`r`n")) {
        $line = $line.Substring(0, $line.Length - 2)
    } elseif ($line.EndsWith("`n")) {
        $line = $line.Substring(0, $line.Length - 1)
    }
    if ($line -notmatch '^\{.+\}$' -or $line -match "`r|`n") {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The Rust smoke client returned an invalid output contract.'
    }
    try {
        $contract = $line | ConvertFrom-Json
    } catch {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The Rust smoke client returned invalid JSON.'
    }
    if ($ExitCode -ne 0 -or $contract.ok -ne $true -or
        $contract.protocolVersion -ne $protocolVersion -or
        [string]::IsNullOrWhiteSpace([string] $contract.supervisorVersion)) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The bundled Supervisor failed the authenticated protocol v1 Ping/Pong.'
    }
    $contract
}

function Invoke-PackageSmokeClient {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][int] $TimeoutMilliseconds
    )

    $repositoryRoot = Split-Path -Parent $PSScriptRoot
    $clientPath = Join-Path $repositoryRoot 'target\release\mission-control-package-smoke.exe'
    if (-not (Test-Path -LiteralPath $clientPath -PathType Leaf)) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_SMOKE_CLIENT_MISSING' -Message 'The Rust package smoke client has not been built.'
    }
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $clientPath
    $startInfo.Arguments = "--pipe-name `"$Name`" --timeout-milliseconds $TimeoutMilliseconds"
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    $stdoutTask = $null
    $stderrTask = $null
    try {
        if (-not $process.Start()) {
            Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The Rust package smoke client did not start.'
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutMilliseconds + 5000)) {
            Stop-OwnedProcess -Process $process -Description 'The Rust package smoke client'
            Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The Rust package smoke client exceeded its deadline.'
        }
        $drain = [Threading.Tasks.Task]::WhenAll([Threading.Tasks.Task[]] @($stdoutTask, $stderrTask))
        if (-not $drain.Wait(5000)) {
            Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The Rust package smoke client output did not drain.'
        }
        Convert-PackageSmokeClientResult -ExitCode $process.ExitCode `
            -StdOut $stdoutTask.GetAwaiter().GetResult() -StdErr $stderrTask.GetAwaiter().GetResult()
    } finally {
        if (-not $process.HasExited) {
            try { Stop-OwnedProcess -Process $process -Description 'The Rust package smoke client' } catch {}
        }
        $process.Dispose()
    }
}

function Start-BundledSupervisor {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [string] $DataDirectory
    )

    $arguments = "--data-dir `"$DataDirectory`" --pipe-name `"$PipeName`""

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Path
    $startInfo.Arguments = $arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The bundled Supervisor did not start.'
    }
    $process
}

function Invoke-BestEffortCleanup {
    param([Parameter(Mandatory)][object[]] $Actions)

    foreach ($action in $Actions) {
        try {
            [void] (& $action.Action)
        } catch {
            [string] $action.Name
        }
    }
}

try {
    $timeoutSecondsValue = Resolve-BoundedInteger -Value $TimeoutSeconds -Minimum 1 -Maximum 300
    $installTimeoutSecondsValue = Resolve-BoundedInteger -Value $InstallTimeoutSeconds -Minimum 1 -Maximum 600
    if ($PipeName -notmatch '^[A-Za-z0-9._-]+$') {
        Throw-PackageSmokeFailure -Code 'PACKAGE_ARGUMENT_INVALID' -Message 'The smoke pipe name is invalid.'
    }
    $resolvedBundle = Resolve-BundlePath -RequestedPath $BundlePath
    [void] (New-Item -ItemType Directory -Path $installRoot)
    [void] (New-Item -ItemType Directory -Path $dataRoot)

    if ((Get-Item -LiteralPath $resolvedBundle).PSIsContainer) {
        Copy-ExtractedArtifact -Source $resolvedBundle -Destination $installRoot
    } elseif ([IO.Path]::GetExtension($resolvedBundle) -ieq '.msi') {
        Expand-MsiArtifact -MsiPath $resolvedBundle -Destination $installRoot -Timeout $installTimeoutSecondsValue
    } else {
        Throw-PackageSmokeFailure -Code 'PACKAGE_ARTIFACT_UNSUPPORTED' -Message 'Only an MSI or an extracted artifact directory is supported.'
    }

    Assert-PackageContents -Root $installRoot
    $desktopPath = Get-SingleBinary -Root $installRoot -Name 'mission-control-desktop.exe' -MissingCode 'PACKAGE_DESKTOP_MISSING'
    $supervisorPath = Get-SingleBinary -Root $installRoot -Name 'mission-control-supervisor.exe' -MissingCode 'PACKAGE_SUPERVISOR_MISSING'
    $desktopSnapshot = New-BinarySnapshot -Path $desktopPath
    $supervisorSnapshot = New-BinarySnapshot -Path $supervisorPath

    Assert-BinarySnapshot -Snapshot $desktopSnapshot
    Assert-BinarySnapshot -Snapshot $supervisorSnapshot
    $supervisorProcess = Start-BundledSupervisor -Path $supervisorPath -DataDirectory $dataRoot
    Assert-BinarySnapshot -Snapshot $desktopSnapshot
    Assert-BinarySnapshot -Snapshot $supervisorSnapshot
    $deadline = [DateTime]::UtcNow.AddSeconds($timeoutSecondsValue)
    $readyPath = Join-Path $dataRoot 'supervisor.ready'
    while (-not (Test-Path -LiteralPath $readyPath) -and [DateTime]::UtcNow -lt $deadline) {
        if ($supervisorProcess.HasExited) { break }
        Start-Sleep -Milliseconds 50
    }
    if (-not (Test-Path -LiteralPath $readyPath)) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The bundled Supervisor did not become ready before the deadline.'
    }
    $ready = Get-Content -Raw -Encoding utf8 -LiteralPath $readyPath | ConvertFrom-Json
    if ($ready.pipe -ne $PipeName -or $ready.pid -ne $supervisorProcess.Id) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The bundled Supervisor published an invalid ready record.'
    }

    $remainingMilliseconds = [int] [Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
    if ($remainingMilliseconds -le 0) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The bundled Supervisor failed the authenticated protocol v1 Ping/Pong.'
    }
    $clientResult = Invoke-PackageSmokeClient -Name $PipeName -TimeoutMilliseconds $remainingMilliseconds

    $result = [ordered]@{
        ok = $true
        artifact = $resolvedBundle
        desktop = [IO.Path]::GetFileName($desktopPath)
        supervisor = [IO.Path]::GetFileName($supervisorPath)
        protocolVersion = $clientResult.protocolVersion
        supervisorVersion = $clientResult.supervisorVersion
    }
} catch {
    $failure = $_
} finally {
    $cleanupActions = @(
        [pscustomobject]@{
            Name = 'supervisor'
            Action = {
                try {
                    if ($script:supervisorProcess) {
                        Stop-OwnedProcess -Process $script:supervisorProcess -Description 'The bundled Supervisor'
                    }
                } finally {
                    if ($script:supervisorProcess) { $script:supervisorProcess.Dispose() }
                }
            }
        }
        [pscustomobject]@{
            Name = 'installer'
            Action = {
                try {
                    if ($script:installerProcess) {
                        Stop-OwnedProcess -Process $script:installerProcess -Description 'The MSI installer'
                    }
                } finally {
                    if ($script:installerProcess) { $script:installerProcess.Dispose() }
                }
            }
        }
        [pscustomobject]@{
            Name = 'temp'
            Action = {
                if (Test-Path -LiteralPath $script:workRoot) {
                    Remove-Item -LiteralPath $script:workRoot -Recurse -Force
                }
            }
        }
    )
    $cleanupErrors = @(Invoke-BestEffortCleanup -Actions $cleanupActions)
    if ($cleanupErrors.Count -gt 0) {
        $originalCode = if ($failure) { Get-FailureCode -ErrorRecord $failure } else { 'none' }
        try {
            Throw-PackageCleanupFailure -OriginalCode $originalCode -FailedActions $cleanupErrors
        } catch {
            $failure = $_
        }
    }
}

if ($failure) {
    $errorResult = [ordered]@{
        ok = $false
        errorCode = Get-FailureCode -ErrorRecord $failure
        message = $failure.Exception.Message
    }
    [Console]::Error.WriteLine(($errorResult | ConvertTo-Json -Compress))
    exit 1
}

$result | ConvertTo-Json -Compress
