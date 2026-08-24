[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

Push-Location $root
try {
    & (Join-Path $PSScriptRoot 'verify-prerequisites.ps1')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & npm.cmd ci
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & cargo metadata --locked --offline --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & cargo clippy --workspace --all-targets --locked --offline -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & cargo test --workspace --all-targets --locked --offline
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & node --test scripts/tests/workspace-boundaries.test.mjs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & npm.cmd run test --workspace '@mission-control/desktop'
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & npm.cmd run typecheck --workspace '@mission-control/desktop'
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & npm.cmd run build --workspace '@mission-control/desktop'
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}
