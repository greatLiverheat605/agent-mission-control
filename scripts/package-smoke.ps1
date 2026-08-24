[CmdletBinding()]
param(
    [string] $BundlePath,

    [string] $PipeName = "mission-package-smoke-$([guid]::NewGuid().ToString('N'))",

    [string] $CredentialTarget = 'Agent Mission Control/Install Secret/v1',

    [ValidateRange(1, 300)]
    [int] $TimeoutSeconds = 15,

    [ValidateRange(1, 600)]
    [int] $InstallTimeoutSeconds = 120
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$productionCredentialTarget = 'Agent Mission Control/Install Secret/v1'
$productInstallId = 'mission-control-desktop-v1'
$protocolVersion = 1
$workRoot = Join-Path ([IO.Path]::GetTempPath()) "mission-package-smoke-$([guid]::NewGuid().ToString('N'))"
$installRoot = Join-Path $workRoot 'installed'
$dataRoot = Join-Path $workRoot 'data'
$supervisorProcess = $null
$installerProcess = $null
$credentialStateKnown = $false
$credentialExisted = $false
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
        Stop-Process -Id $script:installerProcess.Id -Force -ErrorAction SilentlyContinue
        [void] $script:installerProcess.WaitForExit(5000)
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

function Assert-PackageContents {
    param([Parameter(Mandatory)][string] $Root)

    $mcp = Get-ChildItem -LiteralPath $Root -Directory -Recurse -Force | Where-Object Name -IEQ 'MCP' | Select-Object -First 1
    if ($mcp) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_MCP' -Message 'The package contains a forbidden MCP directory.'
    }

    $environmentFile = Get-ChildItem -LiteralPath $Root -File -Recurse -Force |
        Where-Object Name -Match '^\.env(?:\.|$)' | Select-Object -First 1
    if ($environmentFile) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_ENV' -Message 'The package contains a forbidden environment file.'
    }

    $secretPath = Get-ChildItem -LiteralPath $Root -Recurse -Force |
        Where-Object Name -Match '(?i)fixture[-_. ]?secret|secret[-_. ]?fixture' | Select-Object -First 1
    if ($secretPath) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_FORBIDDEN_SECRET' -Message 'The package contains fixture secret material.'
    }

    foreach ($file in Get-ChildItem -LiteralPath $Root -File -Recurse -Force) {
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

    $matches = @(Get-ChildItem -LiteralPath $Root -File -Recurse -Force -Filter $Name)
    if ($matches.Count -eq 0) {
        Throw-PackageSmokeFailure -Code $MissingCode -Message "The package does not contain $Name."
    }
    if ($matches.Count -ne 1) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_BINARY_AMBIGUOUS' -Message "The package contains more than one $Name."
    }
    $matches[0].FullName
}

function Add-CredentialInterop {
    if ('PackageSmokeCredential' -as [type]) { return }

    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
internal struct PackageSmokeNativeCredential
{
    public UInt32 Flags;
    public UInt32 Type;
    public IntPtr TargetName;
    public IntPtr Comment;
    public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
    public UInt32 CredentialBlobSize;
    public IntPtr CredentialBlob;
    public UInt32 Persist;
    public UInt32 AttributeCount;
    public IntPtr Attributes;
    public IntPtr TargetAlias;
    public IntPtr UserName;
}

public static class PackageSmokeCredential
{
    private const UInt32 Generic = 1;
    private const Int32 NotFound = 1168;

    [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredRead(string target, UInt32 type, UInt32 flags, out IntPtr credential);

    [DllImport("advapi32.dll", EntryPoint = "CredDeleteW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredDelete(string target, UInt32 type, UInt32 flags);

    [DllImport("advapi32.dll", EntryPoint = "CredFree")]
    private static extern void CredFree(IntPtr credential);

    public static byte[] Read(string target)
    {
        IntPtr raw;
        if (!CredRead(target, Generic, 0, out raw))
        {
            int error = Marshal.GetLastWin32Error();
            if (error == NotFound) return null;
            throw new Win32Exception(error, "CredReadW failed");
        }

        try
        {
            var credential = (PackageSmokeNativeCredential)Marshal.PtrToStructure(
                raw, typeof(PackageSmokeNativeCredential));
            var secret = new byte[credential.CredentialBlobSize];
            if (secret.Length != 0) Marshal.Copy(credential.CredentialBlob, secret, 0, secret.Length);
            return secret;
        }
        finally
        {
            CredFree(raw);
        }
    }

    public static void DeleteIfPresent(string target)
    {
        if (CredDelete(target, Generic, 0)) return;
        int error = Marshal.GetLastWin32Error();
        if (error != NotFound) throw new Win32Exception(error, "CredDeleteW failed");
    }
}
'@
}

function Write-UInt32LittleEndian {
    param(
        [Parameter(Mandatory)]
        [IO.Stream] $Stream,

        [Parameter(Mandatory)]
        [uint32] $Value
    )

    $bytes = [BitConverter]::GetBytes($Value)
    if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($bytes) }
    $Stream.Write($bytes, 0, $bytes.Length)
}

function Wait-StreamTaskWithDeadline {
    param(
        [Parameter(Mandatory)]
        [Threading.Tasks.Task] $Task,

        [Parameter(Mandatory)]
        [IO.Stream] $Stream,

        [Parameter(Mandatory)]
        [Threading.CancellationTokenSource] $CancellationSource,

        [Parameter(Mandatory)]
        [Diagnostics.Stopwatch] $Stopwatch,

        [Parameter(Mandatory)]
        [int] $TimeoutMilliseconds,

        [Parameter(Mandatory)]
        [string] $Operation
    )

    $remaining = $TimeoutMilliseconds - [int] $Stopwatch.ElapsedMilliseconds
    $completed = $false
    if ($remaining -gt 0) {
        try {
            $completed = $Task.Wait($remaining)
        } catch [AggregateException] {
            $completed = $true
        }
    }
    if ($completed) {
        [void] $Task.GetAwaiter().GetResult()
        return
    }

    $CancellationSource.Cancel()
    $Stream.Dispose()
    $settled = $false
    try {
        $settled = $Task.Wait(5000)
    } catch [AggregateException] {
        $settled = $true
    }
    if (-not $settled -or -not $Task.IsCompleted) {
        throw [TimeoutException]::new("Pipe $Operation did not stop after timing out.")
    }
    throw [TimeoutException]::new("Pipe $Operation timed out.")
}

function New-HandshakeProof {
    param(
        [Parameter(Mandatory)]
        [byte[]] $Secret,

        [Parameter(Mandatory)]
        [byte[]] $Nonce
    )

    $utf8 = New-Object Text.UTF8Encoding -ArgumentList $false
    $installId = $utf8.GetBytes($productInstallId)
    $input = New-Object IO.MemoryStream
    $hmac = $null
    try {
        $tag = $utf8.GetBytes("mission-control-handshake-v1`0")
        $input.Write($tag, 0, $tag.Length)
        Write-UInt32LittleEndian -Stream $input -Value $installId.Length
        $input.Write($installId, 0, $installId.Length)
        Write-UInt32LittleEndian -Stream $input -Value $Nonce.Length
        $input.Write($Nonce, 0, $Nonce.Length)
        Write-UInt32LittleEndian -Stream $input -Value 1
        Write-UInt32LittleEndian -Stream $input -Value $protocolVersion

        $hmac = New-Object Security.Cryptography.HMACSHA256
        $hmac.Key = $Secret
        $hmac.ComputeHash($input.ToArray())
    } finally {
        if ($hmac) { $hmac.Dispose() }
        $input.Dispose()
    }
}

function Write-Frame {
    param(
        [Parameter(Mandatory)]
        [IO.Stream] $Stream,

        [Parameter(Mandatory)]
        $Message,

        [Parameter(Mandatory)]
        [int] $TimeoutMilliseconds
    )

    $utf8 = New-Object Text.UTF8Encoding -ArgumentList $false
    $payload = $utf8.GetBytes(($Message | ConvertTo-Json -Depth 8 -Compress))
    $length = [BitConverter]::GetBytes([uint32] $payload.Length)
    if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($length) }
    $frame = New-Object byte[] ($length.Length + $payload.Length)
    [Array]::Copy($length, 0, $frame, 0, $length.Length)
    [Array]::Copy($payload, 0, $frame, $length.Length, $payload.Length)

    $cancellation = New-Object Threading.CancellationTokenSource
    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $write = $Stream.WriteAsync($frame, 0, $frame.Length, $cancellation.Token)
        Wait-StreamTaskWithDeadline -Task $write -Stream $Stream -CancellationSource $cancellation `
            -Stopwatch $stopwatch -TimeoutMilliseconds $TimeoutMilliseconds -Operation 'write'
        $flush = $Stream.FlushAsync($cancellation.Token)
        Wait-StreamTaskWithDeadline -Task $flush -Stream $Stream -CancellationSource $cancellation `
            -Stopwatch $stopwatch -TimeoutMilliseconds $TimeoutMilliseconds -Operation 'flush'
    } finally {
        $stopwatch.Stop()
        $cancellation.Dispose()
    }
}

function Read-Exact {
    param(
        [Parameter(Mandatory)]
        [IO.Stream] $Stream,

        [Parameter(Mandatory)]
        [int] $Length,

        [Parameter(Mandatory)]
        [int] $TimeoutMilliseconds
    )

    $buffer = New-Object byte[] $Length
    $offset = 0
    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    while ($offset -lt $Length) {
        $remaining = $TimeoutMilliseconds - [int] $stopwatch.ElapsedMilliseconds
        if ($remaining -le 0) { throw [TimeoutException]::new('Pipe read timed out.') }
        $pending = $Stream.BeginRead($buffer, $offset, $Length - $offset, $null, $null)
        try {
            if (-not $pending.AsyncWaitHandle.WaitOne($remaining)) {
                $Stream.Dispose()
                throw [TimeoutException]::new('Pipe read timed out.')
            }
            $read = $Stream.EndRead($pending)
        } finally {
            $pending.AsyncWaitHandle.Dispose()
        }
        if ($read -eq 0) { throw [IO.EndOfStreamException]::new('Pipe closed before the frame completed.') }
        $offset += $read
    }
    $buffer
}

function Read-Frame {
    param(
        [Parameter(Mandatory)]
        [IO.Stream] $Stream,

        [Parameter(Mandatory)]
        [int] $TimeoutMilliseconds
    )

    $lengthBytes = Read-Exact -Stream $Stream -Length 4 -TimeoutMilliseconds $TimeoutMilliseconds
    if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($lengthBytes) }
    $length = [BitConverter]::ToUInt32($lengthBytes, 0)
    if ($length -gt 1048576) { throw [IO.InvalidDataException]::new('Frame exceeds the protocol limit.') }
    $payload = Read-Exact -Stream $Stream -Length $length -TimeoutMilliseconds $TimeoutMilliseconds
    ([Text.Encoding]::UTF8.GetString($payload) | ConvertFrom-Json)
}

function Wait-CredentialSecret {
    param(
        [Parameter(Mandatory)]
        [string] $Target,

        [Parameter(Mandatory)]
        [DateTime] $Deadline
    )

    do {
        $secret = [PackageSmokeCredential]::Read($Target)
        if ($null -ne $secret) { return $secret }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $Deadline)
    throw [TimeoutException]::new('The install credential was not created before the deadline.')
}

function Invoke-ProtocolHandshake {
    param(
        [Parameter(Mandatory)]
        [string] $Name,

        [Parameter(Mandatory)]
        [byte[]] $Secret,

        [Parameter(Mandatory)]
        [int] $TimeoutMilliseconds
    )

    $nonce = New-Object byte[] 32
    $random = [Security.Cryptography.RandomNumberGenerator]::Create()
    $pipe = $null
    try {
        $random.GetBytes($nonce)
        $proof = New-HandshakeProof -Secret $Secret -Nonce $nonce
        $pipe = New-Object IO.Pipes.NamedPipeClientStream -ArgumentList '.', $Name,
            ([IO.Pipes.PipeDirection]::InOut), ([IO.Pipes.PipeOptions]::Asynchronous)
        $pipe.Connect($TimeoutMilliseconds)

        Write-Frame -Stream $pipe -TimeoutMilliseconds $TimeoutMilliseconds -Message ([ordered]@{
                type = 'Handshake'
                payload = [ordered]@{
                    install_id = $productInstallId
                    protocol_versions = @($protocolVersion)
                    nonce = @($nonce)
                    proof = @($proof)
                }
            })
        $accepted = Read-Frame -Stream $pipe -TimeoutMilliseconds $TimeoutMilliseconds
        if ($accepted.type -ne 'HandshakeAccepted' -or $accepted.payload.protocol_version -ne $protocolVersion) {
            throw [IO.InvalidDataException]::new('The sidecar rejected protocol v1 authentication.')
        }

        Write-Frame -Stream $pipe -TimeoutMilliseconds $TimeoutMilliseconds -Message ([ordered]@{ type = 'Ping' })
        $pong = Read-Frame -Stream $pipe -TimeoutMilliseconds $TimeoutMilliseconds
        if ($pong.type -ne 'Pong' -or $pong.payload.protocol_version -ne $protocolVersion) {
            throw [IO.InvalidDataException]::new('The sidecar did not return a protocol v1 Pong.')
        }
        [string] $pong.payload.supervisor_version
    } finally {
        if ($pipe) { $pipe.Dispose() }
        $random.Dispose()
        [Array]::Clear($nonce, 0, $nonce.Length)
        if ($proof) { [Array]::Clear($proof, 0, $proof.Length) }
    }
}

function Start-BundledSupervisor {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [string] $DataDirectory
    )

    if ($PipeName -notmatch '^[A-Za-z0-9._-]+$' -or $CredentialTarget.IndexOfAny([char[]] @('"', "`r", "`n", [char] 0)) -ge 0) {
        Throw-PackageSmokeFailure -Code 'PACKAGE_ARGUMENT_INVALID' -Message 'The smoke pipe or credential target is invalid.'
    }

    $arguments = "--data-dir `"$DataDirectory`" --pipe-name `"$PipeName`""
    if ($CredentialTarget -ne $productionCredentialTarget) {
        $arguments += " --credential-target `"$CredentialTarget`""
    }

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
            "$($action.Name): $($_.Exception.Message)"
        }
    }
}

try {
    $resolvedBundle = Resolve-BundlePath -RequestedPath $BundlePath
    [void] (New-Item -ItemType Directory -Path $installRoot)
    [void] (New-Item -ItemType Directory -Path $dataRoot)

    if ((Get-Item -LiteralPath $resolvedBundle).PSIsContainer) {
        Copy-ExtractedArtifact -Source $resolvedBundle -Destination $installRoot
    } elseif ([IO.Path]::GetExtension($resolvedBundle) -ieq '.msi') {
        Expand-MsiArtifact -MsiPath $resolvedBundle -Destination $installRoot -Timeout $InstallTimeoutSeconds
    } else {
        Throw-PackageSmokeFailure -Code 'PACKAGE_ARTIFACT_UNSUPPORTED' -Message 'Only an MSI or an extracted artifact directory is supported.'
    }

    Assert-PackageContents -Root $installRoot
    $desktopPath = Get-SingleBinary -Root $installRoot -Name 'mission-control-desktop.exe' -MissingCode 'PACKAGE_DESKTOP_MISSING'
    $supervisorPath = Get-SingleBinary -Root $installRoot -Name 'mission-control-supervisor.exe' -MissingCode 'PACKAGE_SUPERVISOR_MISSING'

    Add-CredentialInterop
    $existingSecret = [PackageSmokeCredential]::Read($CredentialTarget)
    $credentialStateKnown = $true
    $credentialExisted = $null -ne $existingSecret
    if ($existingSecret) { [Array]::Clear($existingSecret, 0, $existingSecret.Length) }

    $supervisorProcess = Start-BundledSupervisor -Path $supervisorPath -DataDirectory $dataRoot
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
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

    try {
        $secret = Wait-CredentialSecret -Target $CredentialTarget -Deadline $deadline
        $version = Invoke-ProtocolHandshake -Name $PipeName -Secret $secret -TimeoutMilliseconds ($TimeoutSeconds * 1000)
    } catch {
        Throw-PackageSmokeFailure -Code 'PACKAGE_HANDSHAKE_FAILED' -Message 'The bundled Supervisor failed the authenticated protocol v1 Ping/Pong.'
    } finally {
        if ($secret) { [Array]::Clear($secret, 0, $secret.Length) }
    }

    $result = [ordered]@{
        ok = $true
        artifact = $resolvedBundle
        desktop = [IO.Path]::GetFileName($desktopPath)
        supervisor = [IO.Path]::GetFileName($supervisorPath)
        protocolVersion = $protocolVersion
        supervisorVersion = $version
    }
} catch {
    $failure = $_
} finally {
    $cleanupActions = @(
        [pscustomobject]@{
            Name = 'supervisor'
            Action = {
                try {
                    if ($script:supervisorProcess -and -not $script:supervisorProcess.HasExited) {
                        Stop-Process -Id $script:supervisorProcess.Id -Force -ErrorAction Stop
                        if (-not $script:supervisorProcess.WaitForExit(5000)) {
                            throw 'The bundled Supervisor did not exit during cleanup.'
                        }
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
                    if ($script:installerProcess -and -not $script:installerProcess.HasExited) {
                        Stop-Process -Id $script:installerProcess.Id -Force -ErrorAction Stop
                        if (-not $script:installerProcess.WaitForExit(5000)) {
                            throw 'The MSI installer did not exit during cleanup.'
                        }
                    }
                } finally {
                    if ($script:installerProcess) { $script:installerProcess.Dispose() }
                }
            }
        }
        [pscustomobject]@{
            Name = 'credential'
            Action = {
                if ($script:credentialStateKnown -and -not $script:credentialExisted) {
                    [PackageSmokeCredential]::DeleteIfPresent($script:CredentialTarget)
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
            Throw-PackageSmokeFailure -Code 'PACKAGE_CLEANUP_FAILED' `
                -Message "Package cleanup failed: $($cleanupErrors -join '; '); original=$originalCode."
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
