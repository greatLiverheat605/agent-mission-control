Describe 'verify-prerequisites.ps1' {
    BeforeAll {
        $verificationScript = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\verify-prerequisites.ps1'))
        $originalPath = $env:PATH
        $utf8 = New-Object System.Text.UTF8Encoding -ArgumentList $false

        function New-CommandShim {
            param(
                [Parameter(Mandatory)]
                [string] $Directory,

                [Parameter(Mandatory)]
                [string] $Name,

                [string] $Output,

                [int] $ExitCode = 0
            )

            $body = "@echo off`r`n"
            if ($Output) {
                $body += "echo $Output`r`n"
            }
            $body += "exit /b $ExitCode`r`n"

            [IO.File]::WriteAllText((Join-Path $Directory "$Name.cmd"), $body, $utf8)
        }

        function Invoke-PrerequisiteVerification {
            param(
                [Parameter(Mandatory)]
                [string] $ShimPath,

                [switch] $SkipWebView2
            )

            $powerShell = (Get-Command powershell.exe -CommandType Application).Source
            $arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$verificationScript`" -Json"
            if ($SkipWebView2) {
                $arguments += ' -SkipWebView2'
            }

            $startInfo = New-Object System.Diagnostics.ProcessStartInfo
            $startInfo.FileName = $powerShell
            $startInfo.Arguments = $arguments
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = $true
            $startInfo.RedirectStandardOutput = $true
            $startInfo.RedirectStandardError = $true
            $startInfo.EnvironmentVariables['PATH'] = "$ShimPath;$($startInfo.EnvironmentVariables['PATH'])"

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
        $shimPath = Join-Path $TestDrive ([guid]::NewGuid().ToString('N'))
        [void] (New-Item -ItemType Directory -Path $shimPath)
    }

    It 'reports a failing rustc shim as missing' {
        New-CommandShim -Directory $shimPath -Name rustc -ExitCode 1

        $result = Invoke-PrerequisiteVerification -ShimPath $shimPath -SkipWebView2

        $result.ExitCode | Should -Be 1
        $json = $result.StdOut | ConvertFrom-Json
        $json.rust.status | Should -Be 'missing'
    }

    It 'reports explicit statuses for the supported Windows toolchain' {
        New-CommandShim -Directory $shimPath -Name node -Output 'v24.18.0'
        New-CommandShim -Directory $shimPath -Name git -Output 'git version 2.55.0.windows.3'
        New-CommandShim -Directory $shimPath -Name rustc -Output 'rustc 1.98.0 (88d9e12ae 2026-08-18)'
        New-CommandShim -Directory $shimPath -Name cargo -Output 'cargo 1.98.0 (797e8a9bc 2026-08-05)'

        $result = Invoke-PrerequisiteVerification -ShimPath $shimPath

        $result.ExitCode | Should -Be 0
        $json = $result.StdOut | ConvertFrom-Json
        $json.node.major | Should -Be 24
        $json.pester.major | Should -BeGreaterOrEqual 5
        foreach ($name in 'node', 'git', 'rust', 'cargo', 'pester', 'webview2') {
            $json.$name.status | Should -Be 'ok'
        }
    }

    AfterAll {
        $env:PATH | Should -Be $originalPath
    }
}
