param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]] $RemainingArgs
)

$hasStreamJson = $RemainingArgs -contains "stream-json"
$hasVerbose = $VerbosePreference -eq "Continue"
$hasBare = $RemainingArgs -contains "--bare"
$child = $null
if ($RemainingArgs -notcontains "--version") {
  # Raw CreateProcess (UseShellExecute=false) keeps the child inside the caller's
  # Windows Job Object. Start-Process goes through ShellExecute, whose resolution
  # varies with the environment (PATHEXT presence, headless sessions) and can
  # break job membership or hang on CI runners.
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $env:ComSpec
  $psi.Arguments = "/c ping 127.0.0.1 -n 30 > nul"
  $psi.UseShellExecute = $false
  $psi.CreateNoWindow = $true
  $child = [System.Diagnostics.Process]::Start($psi)
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
