param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]] $RemainingArgs
)

$hasStreamJson = $RemainingArgs -contains "stream-json"
$hasVerbose = $VerbosePreference -eq "Continue"
$hasBare = $RemainingArgs -contains "--bare"
$child = $null
if ($RemainingArgs -notcontains "--version") {
  $child = Start-Process -FilePath "cmd.exe" -ArgumentList @("/c", "ping 127.0.0.1 -n 30 > nul") -PassThru -WindowStyle Hidden
}
$payload = @{
  type = "system"
  subtype = "init"
  arguments = @($RemainingArgs)
  argv = @{
    output_format = if ($hasStreamJson) { "stream-json" } else { "missing" }
    verbose = $hasVerbose
    bare = $hasBare
  }
  allowed_env = $env:CLAUDE_TEST_ALLOWED
  child_pid = if ($null -eq $child) { $null } else { $child.Id }
}
[Console]::WriteLine(($payload | ConvertTo-Json -Compress))
Start-Sleep -Milliseconds 500
[Console]::WriteLine('{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"fixture"}]}}')
[Console]::WriteLine('{"type":"result","subtype":"success","is_error":false,"result":"done"}')
if ($null -ne $child) {
  Start-Sleep -Seconds 30
}
