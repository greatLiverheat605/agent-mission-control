[CmdletBinding()]
param(
  [string]$Root = (Get-Location).Path,
  [string]$Corpus = (Join-Path (Get-Location).Path 'fixtures/agents/secret-corpus.json'),
  [string]$ReportPath = '',
  [switch]$IncludeTests
)

$ErrorActionPreference = 'Stop'

function Get-SecretValues([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "Secret corpus not found: $Path"
  }
  $json = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
  @($json.PSObject.Properties | ForEach-Object { [string]$_.Value } | Where-Object { $_.Length -ge 8 })
}

function Get-DigestPrefix([string]$Value) {
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
    (($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 12)
  } finally { $sha.Dispose() }
}

$resolvedRoot = (Resolve-Path -LiteralPath $Root).Path
$resolvedCorpus = (Resolve-Path -LiteralPath $Corpus).Path
$secretValues = Get-SecretValues $resolvedCorpus

function Get-ReportPath([string]$Path) {
  $fullPath = [IO.Path]::GetFullPath($Path)
  $rootPrefix = $resolvedRoot.TrimEnd([char]92, [char]47) + [IO.Path]::DirectorySeparatorChar
  if ($fullPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    return $fullPath.Substring($rootPrefix.Length)
  }
  return $fullPath
}

$skipNames = @('.git', 'node_modules', 'target', '.codex')
$findings = [System.Collections.Generic.List[object]]::new()
$patterns = @(
  [ordered]@{ name = 'private_key'; regex = '-----BEGIN [^-]+-----[\s\S]+?-----END [^-]+-----' },
  [ordered]@{ name = 'url_userinfo'; regex = '(?i)https?://[^/\s:@]+:[^/\s@]+@[^\s/]+' },
  [ordered]@{ name = 'cookie'; regex = '(?i)(?:cookie|set-cookie):\s*[^\r\n]+' },
  [ordered]@{ name = 'bearer'; regex = '(?i)(?:authorization|proxy-authorization):\s*bearer\s+[A-Za-z0-9._~+/-]{20,}=*' },
  [ordered]@{ name = 'basic'; regex = '(?i)(?:authorization|proxy-authorization):\s*basic\s+[A-Za-z0-9+/=]{20,}' },
  [ordered]@{ name = 'anthropic_key'; regex = 'sk-ant-[A-Za-z0-9_-]{16,}' },
  [ordered]@{ name = 'openai_key'; regex = '\bsk-(?=[A-Za-z0-9]{20,}\b)[A-Za-z0-9_-]+' },
  [ordered]@{ name = 'github_token'; regex = '\bghp_[A-Za-z0-9]{20,}' },
  [ordered]@{ name = 'aws_key'; regex = '\bAKIA[0-9A-Z]{16}\b' },
  [ordered]@{ name = 'env_secret'; regex = '(?i)\b(?:OPENAI|ANTHROPIC|AWS|GITHUB)[A-Z0-9_]*\s*=\s*[^\s,;]+' }
)

$files = Get-ChildItem -LiteralPath $resolvedRoot -Recurse -File -Force | Where-Object {
  $full = $_.FullName
  if ($_.Extension.ToLowerInvariant() -in @('.exe', '.dll', '.bin', '.pdb')) { return $false }
  if ($full -eq $resolvedCorpus) { return $false }
  if ($skipNames | Where-Object { $full -split '[\\/]' -contains $_ }) { return $false }
  if (-not $IncludeTests -and ($full -match '[\\/]tests?[\\/]' -or $_.Name -match '\.(test|spec)\.[^.]+$')) { return $false }
  return $true
}

foreach ($file in $files) {
  try { $content = [IO.File]::ReadAllText($file.FullName, [Text.Encoding]::UTF8) } catch { continue }
  foreach ($secret in $secretValues) {
    if ($content.IndexOf($secret, [StringComparison]::Ordinal) -ge 0) {
      $findings.Add([pscustomobject]@{
        path = Get-ReportPath $file.FullName
        detection = 'corpus'
        secret_sha256_prefix = Get-DigestPrefix $secret
      })
    }
  }
  foreach ($pattern in $patterns) {
    $match = [regex]::Match($content, [string]$pattern.regex)
    if ($match.Success) {
      if ($pattern.name -eq 'env_secret' -and $match.Value -match '(?i)(?:\$|process\.env|\{|<|>|["'']|\.\.\.|your-|placeholder|example)') { continue }
      $findings.Add([pscustomobject]@{
        path = Get-ReportPath $file.FullName
        detection = 'patterns'
        pattern = [string]$pattern.name
        secret_sha256_prefix = Get-DigestPrefix $match.Value
      })
    }
  }
}

$report = [pscustomobject]@{
  generated_at = [DateTime]::UtcNow.ToString('o')
  root = $resolvedRoot
  corpus = Get-ReportPath $resolvedCorpus
  detection = 'corpus+patterns'
  patternCount = $patterns.Count
  files_scanned = @($files).Count
  matches = @($findings)
  status = if ($findings.Count -eq 0) { 'pass' } else { 'fail' }
}
$jsonReport = $report | ConvertTo-Json -Depth 5
if ($ReportPath) {
  $parent = Split-Path -Parent $ReportPath
  if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
  Set-Content -LiteralPath $ReportPath -Value $jsonReport -Encoding UTF8
}
Write-Output $jsonReport
if ($findings.Count -gt 0) { exit 1 }
exit 0
