param(
  [string]$Codex = "codex",
  [string]$Output = "fixtures/agents/codex/app-server/v1/schema"
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $Output | Out-Null
& $Codex app-server generate-json-schema --out $Output
if ($LASTEXITCODE -ne 0) { throw "Codex schema generation failed" }
& $Codex app-server generate-ts --out (Join-Path $Output "typescript")
if ($LASTEXITCODE -ne 0) { throw "Codex TypeScript schema generation failed" }
@{ captured_at = (Get-Date).ToUniversalTime().ToString("o"); command = $Codex; protocol = "app-server-stdio" } |
  ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Output "manifest.json")
