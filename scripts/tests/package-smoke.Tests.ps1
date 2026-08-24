Describe 'package-smoke.ps1' {
    BeforeAll {
        $smokeScript = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\package-smoke.ps1'))
        $powerShell = (Get-Command powershell.exe -CommandType Application).Source

        function Get-PackageSmokeFunctions {
            param([Parameter(Mandatory)][string[]] $Names)

            $tokens = $null
            $parseErrors = $null
            $ast = [Management.Automation.Language.Parser]::ParseFile(
                $smokeScript,
                [ref] $tokens,
                [ref] $parseErrors
            )
            if ($parseErrors) { throw 'package-smoke.ps1 did not parse.' }
            $definitions = @($ast.FindAll({
                        param($node)
                        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
                        $node.Name -in $Names
                    }, $true) | Sort-Object { $_.Extent.StartOffset })
            if ($definitions.Count -ne $Names.Count) {
                throw "Expected package smoke functions were missing: $($Names -join ', ')."
            }
            ($definitions | ForEach-Object { $_.Extent.Text }) -join "`n"
        }

        function Invoke-BoundedProcess {
            param(
                [Parameter(Mandatory)][string] $FileName,
                [Parameter(Mandatory)][string] $Arguments,
                [int] $TimeoutMilliseconds = 15000
            )

            $startInfo = New-Object System.Diagnostics.ProcessStartInfo
            $startInfo.FileName = $FileName
            $startInfo.Arguments = $Arguments
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = $true
            $startInfo.RedirectStandardOutput = $true
            $startInfo.RedirectStandardError = $true

            $process = New-Object System.Diagnostics.Process
            $process.StartInfo = $startInfo
            $stdoutTask = $null
            $stderrTask = $null
            try {
                [void] $process.Start()
                $childId = $process.Id
                $stdoutTask = $process.StandardOutput.ReadToEndAsync()
                $stderrTask = $process.StandardError.ReadToEndAsync()
                $timedOut = -not $process.WaitForExit($TimeoutMilliseconds)
                if ($timedOut) {
                    $process.Kill()
                    if (-not $process.WaitForExit(5000)) {
                        throw "Timed-out child PID $childId did not exit during reap."
                    }
                }

                $drain = [Threading.Tasks.Task]::WhenAll(
                    [Threading.Tasks.Task[]] @($stdoutTask, $stderrTask)
                )
                if (-not $drain.Wait(5000)) {
                    throw "Child PID $childId output did not drain after exit."
                }
                $stdout = $stdoutTask.GetAwaiter().GetResult()
                $stderr = $stderrTask.GetAwaiter().GetResult()
                [pscustomobject]@{
                    ExitCode = $process.ExitCode
                    StdOut = $stdout
                    StdErr = $stderr
                    TimedOut = $timedOut
                    Pid = $childId
                    Reaped = -not [bool] (Get-Process -Id $childId -ErrorAction SilentlyContinue)
                }
            } finally {
                if (-not $process.HasExited) {
                    try { $process.Kill() } catch {}
                    [void] $process.WaitForExit(5000)
                }
                $process.Dispose()
            }
        }

        function New-PackageFixture {
            param(
                [switch] $Desktop,
                [switch] $Supervisor
            )

            $path = Join-Path $TestDrive ([guid]::NewGuid().ToString('N'))
            [void] (New-Item -ItemType Directory -Path $path)
            if ($Desktop) {
                [IO.File]::WriteAllBytes((Join-Path $path 'mission-control-desktop.exe'), [byte[]] @(0))
            }
            if ($Supervisor) {
                Copy-Item -LiteralPath $env:ComSpec -Destination (Join-Path $path 'mission-control-supervisor.exe')
            }
            $path
        }

        function Invoke-PackageSmokeFixture {
            param(
                [Parameter(Mandatory)]
                [string] $BundlePath,

                $TimeoutSeconds = 1,

                $InstallTimeoutSeconds = 120
            )

            $pipeName = "mission-package-smoke-$([guid]::NewGuid().ToString('N'))"
            $arguments = @(
                '-NoProfile',
                '-NonInteractive',
                '-ExecutionPolicy', 'Bypass',
                '-File', ('"{0}"' -f $smokeScript),
                '-BundlePath', ('"{0}"' -f $BundlePath),
                '-PipeName', $pipeName,
                '-TimeoutSeconds', $TimeoutSeconds,
                '-InstallTimeoutSeconds', $InstallTimeoutSeconds
            ) -join ' '

            $result = Invoke-BoundedProcess -FileName $powerShell -Arguments $arguments `
                -TimeoutMilliseconds 15000
            if ($result.TimedOut -or -not $result.Reaped) {
                throw "Package smoke child PID $($result.Pid) exceeded its harness deadline."
            }
            $result
        }

        function Assert-PackageSmokeFailure {
            param(
                [Parameter(Mandatory)] $Result,
                [Parameter(Mandatory)][string] $ExpectedCode
            )

            $Result.TimedOut | Should -BeFalse
            $Result.Reaped | Should -BeTrue
            $Result.ExitCode | Should -Be 1
            $Result.StdOut | Should -BeExactly ''
            $contractLine = [string] $Result.StdErr
            if ($contractLine.EndsWith("`r`n")) {
                $contractLine = $contractLine.Substring(0, $contractLine.Length - 2)
            } elseif ($contractLine.EndsWith("`n")) {
                $contractLine = $contractLine.Substring(0, $contractLine.Length - 1)
            }
            $contractLine | Should -MatchExactly '^\{.+\}$'
            $contractLine | Should -Not -Match "`r|`n"
            $contract = $contractLine | ConvertFrom-Json
            $contract.ok | Should -BeFalse
            $contract.errorCode | Should -BeExactly $ExpectedCode
            $contract.message | Should -Not -BeNullOrEmpty
        }
    }

    It 'keeps protocol and Credential Manager ownership in the Rust smoke client' {
        $scriptText = Get-Content -Raw -Encoding utf8 -LiteralPath $smokeScript

        $scriptText | Should -Match 'target\\release\\mission-control-package-smoke\.exe'
        $scriptText | Should -Not -Match 'Add-CredentialInterop|PackageSmokeCredential|CredDelete|CredRead'
        $scriptText | Should -Not -Match 'Write-UInt32LittleEndian|New-HandshakeProof|Write-Frame|Read-Frame|Read-Exact|Wait-CredentialSecret|Invoke-ProtocolHandshake'
        $scriptText | Should -Not -Match '(?m)^\s*\[string\]\s*\$CredentialTarget'
        $scriptText | Should -Not -Match '--credential-target'
    }

    It 'bounds and reaps a child that fills both output pipes and waits forever' {
        $probe = Join-Path $TestDrive 'forever-child.ps1'
        [IO.File]::WriteAllText($probe, @'
[Console]::Out.Write(('o' * 131072))
[Console]::Error.Write(('e' * 131072))
while ($true) { Start-Sleep -Seconds 1 }
'@, (New-Object Text.UTF8Encoding -ArgumentList $false))
        $arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$probe`""

        $result = Invoke-BoundedProcess -FileName $powerShell -Arguments $arguments -TimeoutMilliseconds 250

        $result.TimedOut | Should -BeTrue
        $result.Reaped | Should -BeTrue
        $result.StdOut.Length | Should -Be 131072
        $result.StdErr.Length | Should -Be 131072
    }

    It 'terminates an owned process through the retained process object' {
        $definitions = Get-PackageSmokeFunctions -Names @('Stop-OwnedProcess')
        $stopScript = [scriptblock]::Create($definitions + @'
$events = [Collections.Generic.List[string]]::new()
$process = [pscustomobject]@{ HasExited = $false; Id = 4242 }
$process | Add-Member -MemberType ScriptMethod -Name Kill -Value { [void] $events.Add('kill') }
$process | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value { param($timeout) [void] $events.Add("wait:$timeout"); $true }
Stop-OwnedProcess -Process $process -Description 'fixture' -TimeoutMilliseconds 321
@($events)
'@)

        (& $stopScript) | Should -Be @('kill', 'wait:321')
        (Get-Content -Raw -Encoding utf8 -LiteralPath $smokeScript) | Should -Not -Match 'Stop-Process\s+-Id'
    }

    It 'runs credential and temp cleanup after an earlier owned-resource cleanup fails' {
        $definitions = Get-PackageSmokeFunctions -Names @('Invoke-BestEffortCleanup')
        $cleanupScript = [scriptblock]::Create($definitions + @'
$events = [Collections.Generic.List[string]]::new()
$actions = @(
    [pscustomobject]@{ Name = 'supervisor'; Action = { [void] $events.Add('supervisor'); throw 'stop failed' } }
    [pscustomobject]@{ Name = 'installer'; Action = { [void] $events.Add('installer') } }
    [pscustomobject]@{ Name = 'credential'; Action = { [void] $events.Add('credential') } }
    [pscustomobject]@{ Name = 'temp'; Action = { [void] $events.Add('temp') } }
)
$errors = @(Invoke-BestEffortCleanup -Actions $actions)
[pscustomobject]@{ Events = @($events); Errors = @($errors) }
'@)

        $result = & $cleanupScript

        $result.Events | Should -Be @('supervisor', 'installer', 'credential', 'temp')
        $result.Errors.Count | Should -Be 1
        $result.Errors[0] | Should -BeExactly 'supervisor'
    }

    It 'reports cleanup failure with the stable original code and no exception detail' {
        $definitions = Get-PackageSmokeFunctions -Names @(
            'Throw-PackageSmokeFailure',
            'Throw-PackageCleanupFailure'
        )
        $cleanupScript = [scriptblock]::Create($definitions + @'
try {
    Throw-PackageCleanupFailure -OriginalCode 'PACKAGE_HANDSHAKE_FAILED' -FailedActions @('supervisor: leaked-secret-value')
} catch {
    [pscustomobject]@{
        Code = $_.Exception.Data['PackageSmokeCode']
        Message = $_.Exception.Message
    }
}
'@)

        $result = & $cleanupScript

        $result.Code | Should -BeExactly 'PACKAGE_CLEANUP_FAILED'
        $result.Message | Should -Match 'original=PACKAGE_HANDSHAKE_FAILED'
        $result.Message | Should -Not -Match 'leaked-secret-value'
    }

    It 'does not normalize extra output around failure contracts' {
        $probe = Join-Path $TestDrive 'noisy-failure.ps1'
        [IO.File]::WriteAllText($probe, @'
[Console]::Out.Write(' ')
[Console]::Error.WriteLine('')
[Console]::Error.WriteLine('{"ok":false,"errorCode":"PACKAGE_PROBE_FAILED","message":"probe"}')
exit 1
'@, (New-Object Text.UTF8Encoding -ArgumentList $false))
        $arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$probe`""

        $result = Invoke-BoundedProcess -FileName $powerShell -Arguments $arguments

        { Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_PROBE_FAILED' } |
            Should -Throw
    }

    It 'fails closed with a stable error when the desktop binary is missing' {
        $artifact = New-PackageFixture -Supervisor

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_DESKTOP_MISSING'
    }

    It 'fails closed with a stable error when the supervisor binary is missing' {
        $artifact = New-PackageFixture -Desktop

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_SUPERVISOR_MISSING'
    }

    It 'returns one stable JSON error for a non-numeric timeout' {
        $artifact = New-PackageFixture -Desktop -Supervisor

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact -TimeoutSeconds 'not-a-number'

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_ARGUMENT_INVALID'
    }

    It 'returns one stable JSON error for an out-of-range install timeout' {
        $artifact = New-PackageFixture -Desktop -Supervisor

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact -InstallTimeoutSeconds 601

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_ARGUMENT_INVALID'
    }

    It 'fails within the deadline when the installed sidecar cannot authenticate protocol v1' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        $stopwatch.Stop()
        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_HANDSHAKE_FAILED'
        $stopwatch.Elapsed | Should -BeLessThan ([TimeSpan]::FromSeconds(15))
    }

    It 'rejects an artifact containing an MCP directory' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        [void] (New-Item -ItemType Directory -Path (Join-Path $artifact 'MCP'))

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_FORBIDDEN_MCP'
    }

    It 'rejects an artifact containing an environment file' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        [IO.File]::WriteAllText((Join-Path $artifact '.env.production'), 'TOKEN=not-a-real-secret')

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_FORBIDDEN_ENV'
    }

    It 'rejects an artifact containing fixture secret material' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        [IO.File]::WriteAllText((Join-Path $artifact 'fixture.txt'), 'mission-control-fixture-secret')

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_FORBIDDEN_SECRET'
    }

    It 'rejects fixture secret bytes in a binary across a scan chunk boundary' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        $marker = [Text.Encoding]::ASCII.GetBytes('mission-control-fixture-secret')
        $bytes = New-Object byte[] (65536 + $marker.Length)
        [Array]::Copy($marker, 0, $bytes, 65530, $marker.Length)
        [IO.File]::WriteAllBytes((Join-Path $artifact 'payload.bin'), $bytes)

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_FORBIDDEN_SECRET'
    }

    It 'rejects a reparse point before recursively scanning package contents' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        $outside = Join-Path $TestDrive ([guid]::NewGuid().ToString('N'))
        [void] (New-Item -ItemType Directory -Path $outside)
        [void] (New-Item -ItemType Junction -Path (Join-Path $artifact 'linked') -Target $outside)

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_REPARSE_POINT'
    }

    It 'rejects the build-only Rust smoke client inside an extracted package' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        [IO.File]::WriteAllBytes((Join-Path $artifact 'mission-control-package-smoke.exe'), [byte[]] @(0))

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        Assert-PackageSmokeFailure -Result $result -ExpectedCode 'PACKAGE_FORBIDDEN_HELPER'
    }

    It 'detects binary replacement between package scan and launch' {
        $definitions = Get-PackageSmokeFunctions -Names @(
            'Throw-PackageSmokeFailure',
            'New-BinarySnapshot',
            'Assert-BinarySnapshot'
        )
        $snapshotScript = [scriptblock]::Create($definitions + @'
$path = Join-Path $env:TEMP "mission-snapshot-$([guid]::NewGuid().ToString('N')).exe"
try {
    [IO.File]::WriteAllBytes($path, [byte[]] @(1, 2, 3))
    $snapshot = New-BinarySnapshot -Path $path
    [IO.File]::WriteAllBytes($path, [byte[]] @(1, 2, 3, 4))
    try { Assert-BinarySnapshot -Snapshot $snapshot } catch { $_.Exception.Data['PackageSmokeCode'] }
} finally {
    Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
}
'@)

        (& $snapshotScript) | Should -BeExactly 'PACKAGE_BINARY_CHANGED'
    }
}

Describe 'Windows package CI contract' {
    BeforeAll {
        $repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
        $workflowPath = Join-Path $repositoryRoot '.github\workflows\windows-ci.yml'
        $verifyPath = Join-Path $repositoryRoot 'scripts\verify-workspace.ps1'
        $prerequisitePath = Join-Path $repositoryRoot 'scripts\verify-prerequisites.ps1'
        $packagePath = Join-Path $repositoryRoot 'package.json'
        $desktopPackagePath = Join-Path $repositoryRoot 'apps\desktop\package.json'
        $tauri = Join-Path $repositoryRoot 'node_modules\.bin\tauri.cmd'
    }

    It 'runs the reproducible Windows build and package smoke gates' {
        Test-Path -LiteralPath $workflowPath | Should -BeTrue
        $workflow = Get-Content -Raw -Encoding utf8 -LiteralPath $workflowPath

        $workflow | Should -Match 'windows-latest'
        $workflow | Should -Match 'npm\.cmd ci'
        $workflow | Should -Match 'cargo fmt --all -- --check'
        $workflow | Should -Match 'cargo clippy --workspace --all-targets --locked --offline -- -D warnings'
        $workflow | Should -Match 'cargo test --workspace --all-targets --locked --offline'
        $workflow | Should -Match 'Invoke-Pester.*package-smoke\.Tests\.ps1.*-CI'
        $workflow | Should -Match 'npm\.cmd run tauri:build'
        $workflow | Should -Match 'scripts/package-smoke\.ps1'
    }

    It 'pins every third-party action and Pester to reviewed immutable versions' {
        $workflow = Get-Content -Raw -Encoding utf8 -LiteralPath $workflowPath
        $prerequisites = Get-Content -Raw -Encoding utf8 -LiteralPath $prerequisitePath

        $workflow | Should -Match 'actions/checkout@11d5960a326750d5838078e36cf38b85af677262'
        $workflow | Should -Match 'actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020'
        $workflow | Should -Match 'actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830'
        $workflow | Should -Match 'Install-Module Pester -RequiredVersion 5\.7\.1 -Scope CurrentUser -Force'
        $workflow | Should -Match 'Import-Module Pester -RequiredVersion 5\.7\.1 -Force'
        $workflow | Should -Not -Match 'MinimumVersion|SkipPublisherCheck'
        $prerequisites | Should -Match "\[Version\]\s*'5\.7\.1'"
        $prerequisites | Should -Match 'Where-Object\s+Version\s+-EQ\s+\$requiredVersion'
    }

    It 'keys the Windows cache from both lockfiles and the Rust toolchain' {
        $workflow = Get-Content -Raw -Encoding utf8 -LiteralPath $workflowPath

        $workflow | Should -Match "hashFiles\('Cargo\.lock', 'package-lock\.json', 'rust-toolchain\.toml'\)"
    }

    It 'does not sign PR builds or upload package artifacts' {
        $workflow = Get-Content -Raw -Encoding utf8 -LiteralPath $workflowPath

        $workflow | Should -Not -Match 'TAURI_SIGNING_PRIVATE_KEY|upload-artifact|artifact@'
    }

    It 'ignores the local Pester CI result file' {
        $ignore = Get-Content -Raw -Encoding utf8 -LiteralPath (Join-Path $repositoryRoot '.gitignore')

        $ignore | Should -Match '(?m)^testResults\.xml$'
    }

    It 'exposes repeatable local package verification commands' {
        $package = Get-Content -Raw -Encoding utf8 -LiteralPath $packagePath | ConvertFrom-Json

        $package.scripts.'verify:workspace' | Should -Match 'scripts/verify-workspace\.ps1'
        $package.scripts.'test:package' | Should -Match 'package-smoke\.Tests\.ps1'
        $package.scripts.'package:smoke' | Should -Match 'scripts/package-smoke\.ps1'
    }

    It 'builds one deterministic MSI artifact for the package smoke gate' {
        $sidecarConfig = Get-Content -Raw -Encoding utf8 -LiteralPath (Join-Path $repositoryRoot 'apps\desktop\src-tauri\tauri.sidecar.conf.json') | ConvertFrom-Json
        $desktopPackage = Get-Content -Raw -Encoding utf8 -LiteralPath $desktopPackagePath | ConvertFrom-Json

        $sidecarConfig.bundle.targets | Should -Be 'msi'
        @($sidecarConfig.bundle.icon) | Should -Be @('generated/mission-control.ico')
        @($sidecarConfig.bundle.externalBin) | Should -Be @('binaries/mission-control-supervisor')
        $desktopPackage.scripts.'prepare:sidecar' | Should -MatchExactly '^cargo build -p mission-supervisor --release --locked --offline && node ../../scripts/prepare-sidecar\.mjs$'
        $desktopPackage.scripts.'tauri:build' | Should -Match 'tauri build --config src-tauri/tauri\.sidecar\.conf\.json -- --locked --offline$'
        $help = & $tauri build --help | Out-String
        $LASTEXITCODE | Should -Be 0
        $help | Should -Match 'Command line arguments passed to the runner'
        $help | Should -Match 'Use `--` to explicitly mark the start of the arguments'
    }

    It 'uses the root Tauri build path with Cargo networking disabled in CI' {
        $workflow = Get-Content -Raw -Encoding utf8 -LiteralPath $workflowPath
        $package = Get-Content -Raw -Encoding utf8 -LiteralPath $packagePath | ConvertFrom-Json

        $package.scripts.'tauri:build' | Should -Be 'npm run tauri:build --workspaces --if-present'
        $workflow | Should -Match '(?s)name: Build unsigned Tauri package.*?CARGO_NET_OFFLINE: ["'']?true["'']?.*?run: npm\.cmd run tauri:build'
    }

    It 'keeps the local workspace gate locked and offline after dependency restore' {
        $verify = Get-Content -Raw -Encoding utf8 -LiteralPath $verifyPath

        $verify | Should -Match 'npm\.cmd ci'
        $verify | Should -Match 'cargo metadata --locked --offline'
        $verify | Should -Match 'cargo clippy --workspace --all-targets --locked --offline -- -D warnings'
        $verify | Should -Match 'cargo test --workspace --all-targets --locked --offline'
        $verify | Should -Match "npm\.cmd run test --workspace '?@mission-control/desktop'?"
    }
}
