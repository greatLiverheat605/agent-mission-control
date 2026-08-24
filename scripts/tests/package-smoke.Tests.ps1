Describe 'package-smoke.ps1' {
    BeforeAll {
        $smokeScript = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\package-smoke.ps1'))
        $powerShell = (Get-Command powershell.exe -CommandType Application).Source

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

                [int] $TimeoutSeconds = 1
            )

            $pipeName = "mission-package-smoke-$([guid]::NewGuid().ToString('N'))"
            $credentialTarget = "Agent Mission Control/Package Smoke/$([guid]::NewGuid().ToString('N'))"
            $script:credentialTargets.Add($credentialTarget)
            $arguments = @(
                '-NoProfile',
                '-NonInteractive',
                '-ExecutionPolicy', 'Bypass',
                '-File', ('"{0}"' -f $smokeScript),
                '-BundlePath', ('"{0}"' -f $BundlePath),
                '-PipeName', $pipeName,
                '-CredentialTarget', ('"{0}"' -f $credentialTarget),
                '-TimeoutSeconds', $TimeoutSeconds
            ) -join ' '

            $startInfo = New-Object System.Diagnostics.ProcessStartInfo
            $startInfo.FileName = $powerShell
            $startInfo.Arguments = $arguments
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = $true
            $startInfo.RedirectStandardOutput = $true
            $startInfo.RedirectStandardError = $true

            $process = New-Object System.Diagnostics.Process
            $process.StartInfo = $startInfo
            [void] $process.Start()
            $stdout = $process.StandardOutput.ReadToEnd()
            $stderr = $process.StandardError.ReadToEnd()
            $process.WaitForExit()

            [pscustomobject]@{
                ExitCode = $process.ExitCode
                StdOut = $stdout.Trim()
                StdErr = $stderr.Trim()
            }
        }
    }

    BeforeEach {
        $script:credentialTargets = [Collections.Generic.List[string]]::new()
    }

    It 'computes protocol v1 proofs with HMAC-SHA256 on Windows PowerShell' {
        $tokens = $null
        $parseErrors = $null
        $ast = [Management.Automation.Language.Parser]::ParseFile(
            $smokeScript,
            [ref] $tokens,
            [ref] $parseErrors
        )
        $parseErrors | Should -BeNullOrEmpty
        $definitions = $ast.FindAll({
                param($node)
                $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -in @('Write-UInt32LittleEndian', 'New-HandshakeProof')
            }, $true) |
            Sort-Object { $_.Extent.StartOffset } |
            ForEach-Object { $_.Extent.Text }
        $proofScript = [scriptblock]::Create(($definitions -join "`n") + @'
$productInstallId = 'mission-control-desktop-v1'
$protocolVersion = 1
$secret = [byte[]] (0..31)
$nonce = [byte[]] (32..63)
$proof = New-HandshakeProof -Secret $secret -Nonce $nonce
(($proof | ForEach-Object { $_.ToString('x2') }) -join '')
'@)

        (& $proofScript) | Should -Be 'bd5b23b3a234ccac320572b35275e24c80fff88b7c776d5d6d420c316bf77573'
    }

    It 'fails closed with a stable error when the desktop binary is missing' {
        $artifact = New-PackageFixture -Supervisor

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        $result.ExitCode | Should -Not -Be 0
        $result.StdErr | Should -Match 'PACKAGE_DESKTOP_MISSING'
    }

    It 'fails closed with a stable error when the supervisor binary is missing' {
        $artifact = New-PackageFixture -Desktop

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        $result.ExitCode | Should -Not -Be 0
        $result.StdErr | Should -Match 'PACKAGE_SUPERVISOR_MISSING'
    }

    It 'fails within the deadline when the installed sidecar cannot authenticate protocol v1' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        $stopwatch.Stop()
        $result.ExitCode | Should -Not -Be 0
        $result.StdErr | Should -Match 'PACKAGE_HANDSHAKE_FAILED'
        $stopwatch.Elapsed | Should -BeLessThan ([TimeSpan]::FromSeconds(15))
    }

    It 'rejects an artifact containing an MCP directory' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        [void] (New-Item -ItemType Directory -Path (Join-Path $artifact 'MCP'))

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        $result.ExitCode | Should -Not -Be 0
        $result.StdErr | Should -Match 'PACKAGE_FORBIDDEN_MCP'
    }

    It 'rejects an artifact containing an environment file' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        [IO.File]::WriteAllText((Join-Path $artifact '.env.production'), 'TOKEN=not-a-real-secret')

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        $result.ExitCode | Should -Not -Be 0
        $result.StdErr | Should -Match 'PACKAGE_FORBIDDEN_ENV'
    }

    It 'rejects an artifact containing fixture secret material' {
        $artifact = New-PackageFixture -Desktop -Supervisor
        [IO.File]::WriteAllText((Join-Path $artifact 'fixture.txt'), 'mission-control-fixture-secret')

        $result = Invoke-PackageSmokeFixture -BundlePath $artifact

        $result.ExitCode | Should -Not -Be 0
        $result.StdErr | Should -Match 'PACKAGE_FORBIDDEN_SECRET'
    }

    AfterEach {
        foreach ($target in $script:credentialTargets) {
            & cmdkey.exe "/delete:$target" *> $null
        }
    }
}

Describe 'Windows package CI contract' {
    BeforeAll {
        $repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
        $workflowPath = Join-Path $repositoryRoot '.github\workflows\windows-ci.yml'
        $verifyPath = Join-Path $repositoryRoot 'scripts\verify-workspace.ps1'
        $packagePath = Join-Path $repositoryRoot 'package.json'
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
        $desktopPackage = Get-Content -Raw -Encoding utf8 -LiteralPath (Join-Path $repositoryRoot 'apps\desktop\package.json') | ConvertFrom-Json

        $sidecarConfig.bundle.targets | Should -Be 'msi'
        @($sidecarConfig.bundle.icon) | Should -Be @('generated/mission-control.ico')
        $desktopPackage.scripts.'prepare:sidecar' | Should -Match 'scripts/prepare-sidecar\.mjs'
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
