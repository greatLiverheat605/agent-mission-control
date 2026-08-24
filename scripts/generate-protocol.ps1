[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$destination = Join-Path $repositoryRoot 'packages\protocol\src\generated.ts'
$temporary = "$destination.$([guid]::NewGuid().ToString('N')).tmp"
$cargo = (Get-Command cargo.exe -CommandType Application -ErrorAction Stop).Source

try {
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $content = & $cargo run -p mission-protocol --bin mission-protocol-export --locked --offline 2>$null | Out-String
    $ErrorActionPreference = $previousErrorActionPreference
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrEmpty($content)) {
        throw 'protocol exporter failed'
    }
    [IO.File]::WriteAllText($temporary, $content, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $destination -Force
} finally {
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Force
    }
}
