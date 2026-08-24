[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

Push-Location $root
try {
    & (Join-Path $PSScriptRoot 'verify-prerequisites.ps1')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & npm.cmd install --ignore-scripts
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & node --test scripts/tests/workspace-boundaries.test.mjs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}
