param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Path $Path -Force | Out-Null
git -C $Path init -q
git -C $Path config user.name 'Mission Control Fixture'
git -C $Path config user.email 'fixture@example.invalid'
Set-Content -LiteralPath (Join-Path $Path 'tracked.txt') -Value "baseline`n" -Encoding utf8
git -C $Path add -- tracked.txt
git -C $Path commit -qm 'baseline'
