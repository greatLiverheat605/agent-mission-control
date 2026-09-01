[CmdletBinding()]
param(
    [switch] $IAuthorizeRealInvocation,
    [switch] $SimulationMode,
    [ValidateSet('app-server', 'exec')]
    [string] $InvocationTransport = 'app-server',
    [string] $OutputRoot,
    [string] $RunId = ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')),
    [int] $TokenLimit = 200,
    [int] $DurationLimitSeconds = 120,
    [string] $BudgetBasis = '',
    [string[]] $TaskCategories = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Redact-Text {
    param([AllowNull()][string] $Text)
    if ($null -eq $Text) { return $null }
    $redacted = $Text
    foreach ($pattern in @(
        '(?i)bearer\s+[A-Za-z0-9._~+/=-]+',
        '(?i)(?:password|secret|token|private[_-]?key)\s*[:=]\s*[^\s,;}]+' ,
        '(?i)(?:https?://)(?:[^/@\s]+)@',
        '(?i)(?:cookie|authorization)\s*[:=]\s*[^\r\n]+'
    )) { $redacted = [regex]::Replace($redacted, $pattern, '[REDACTED]') }
    return $redacted
}

function Write-JsonUtf8 {
    param([Parameter(Mandatory)][string] $Path, [Parameter(Mandatory)] $Value)
    $Value | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $Path -Encoding utf8
}

function Get-CodexVersion {
    try {
        $version = (& codex --version 2>$null | Select-Object -First 1)
        if ($null -ne $version -and -not [string]::IsNullOrWhiteSpace([string]$version)) {
            return ([string]$version).Trim()
        }
    } catch { }
    return 'unavailable'
}

function Get-InvocationModel {
    $explicitModel = [Environment]::GetEnvironmentVariable('CODEX_REAL_INVOCATION_MODEL')
    if (-not [string]::IsNullOrWhiteSpace($explicitModel)) { return $explicitModel.Trim() }

    $command = [Environment]::GetEnvironmentVariable('CODEX_REAL_INVOCATION_COMMAND')
    if (-not [string]::IsNullOrWhiteSpace($command)) {
        $modelMatch = [regex]::Match($command, '(?i)(?:^|\s)(?:--model|-m)\s+(?:"([^"]+)"|''([^'']+)''|([^\s]+))')
        if ($modelMatch.Success) {
            foreach ($group in @($modelMatch.Groups[1], $modelMatch.Groups[2], $modelMatch.Groups[3])) {
                if ($group.Success -and -not [string]::IsNullOrWhiteSpace($group.Value)) { return $group.Value }
            }
        }
    }

    $profileRoot = [Environment]::GetEnvironmentVariable('CODEX_HOME')
    if ([string]::IsNullOrWhiteSpace($profileRoot)) { $profileRoot = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex' }
    $configPath = Join-Path $profileRoot 'config.toml'
    if (Test-Path -LiteralPath $configPath -PathType Leaf) {
        $configMatch = Select-String -LiteralPath $configPath -Pattern '^\s*model\s*=\s*["'']([^"'']+)["'']' | Select-Object -First 1
        if ($configMatch) { return $configMatch.Matches[0].Groups[1].Value }
    }
    return 'unavailable'
}

function Get-TokenUsage {
    param(
        [Parameter(Mandatory)][System.Collections.Generic.List[string]] $EventLines,
        [switch] $Latest,
        [switch] $CompletedOnly
    )
    [long]$inputTokens = 0
    [long]$cachedInputTokens = 0
    [long]$cacheWriteInputTokens = 0
    [long]$outputTokens = 0
    [long]$reasoningOutputTokens = 0
    $found = $false
    $latestSample = $null
    foreach ($line in $EventLines) {
        try { $message = $line | ConvertFrom-Json } catch { continue }
        if ($CompletedOnly -and [string]$message.type -ne 'turn.completed' -and [string]$message.method -ne 'turn/completed') { continue }
        $usage = $null
        foreach ($candidate in @(
            $message.PSObject.Properties['usage'],
            $message.PSObject.Properties['params']
        )) {
            if ($null -eq $candidate -or $null -eq $candidate.Value) { continue }
            if ($candidate.Name -eq 'usage') { $usage = $candidate.Value; break }
            foreach ($nestedName in @('usage', 'tokenUsage')) {
                $nested = $candidate.Value.PSObject.Properties[$nestedName]
                if ($null -ne $nested -and $null -ne $nested.Value) {
                    $usage = $nested.Value
                    if ($nestedName -eq 'tokenUsage') {
                        $total = $nested.Value.PSObject.Properties['total']
                        if ($null -ne $total -and $null -ne $total.Value) { $usage = $total.Value }
                    }
                    break
                }
            }
            if ($null -ne $usage) { break }
        }
        if ($null -eq $usage) { continue }
        $sample = [ordered]@{ inputTokens = 0; cachedInputTokens = 0; cacheWriteInputTokens = 0; outputTokens = 0; reasoningOutputTokens = 0; totalTokens = $null }
        foreach ($mapping in @(
            @{ Names = @('input_tokens', 'inputTokens'); Target = 'inputTokens' },
            @{ Names = @('cached_input_tokens', 'cachedInputTokens'); Target = 'cachedInputTokens' },
            @{ Names = @('cache_write_input_tokens', 'cacheWriteInputTokens'); Target = 'cacheWriteInputTokens' },
            @{ Names = @('output_tokens', 'outputTokens'); Target = 'outputTokens' },
            @{ Names = @('reasoning_output_tokens', 'reasoningOutputTokens'); Target = 'reasoningOutputTokens' },
            @{ Names = @('total_tokens', 'totalTokens'); Target = 'totalTokens' }
        )) {
            foreach ($name in $mapping.Names) {
                $property = $usage.PSObject.Properties[$name]
                if ($null -eq $property) { continue }
                $value = [long]0
                if ([long]::TryParse([string]$property.Value, [ref]$value)) { $sample[$mapping.Target] = $value }
                break
            }
        }
        if ($null -eq $sample.totalTokens) { $sample.totalTokens = [long]$sample.inputTokens + [long]$sample.outputTokens }
        $found = $true
        if ($Latest) { $latestSample = $sample; continue }
        $inputTokens += $sample.inputTokens
        $cachedInputTokens += $sample.cachedInputTokens
        $cacheWriteInputTokens += $sample.cacheWriteInputTokens
        $outputTokens += $sample.outputTokens
        $reasoningOutputTokens += $sample.reasoningOutputTokens
    }
    if ($Latest) {
        if ($null -eq $latestSample) { return [ordered]@{ status = 'unavailable' } }
        return [ordered]@{
            status = 'reported'
            inputTokens = [long]$latestSample.inputTokens
            cachedInputTokens = [long]$latestSample.cachedInputTokens
            cacheWriteInputTokens = [long]$latestSample.cacheWriteInputTokens
            outputTokens = [long]$latestSample.outputTokens
            reasoningOutputTokens = [long]$latestSample.reasoningOutputTokens
            totalTokens = [long]$latestSample.totalTokens
        }
    }
    if (-not $found) { return [ordered]@{ status = 'unavailable' } }
    return [ordered]@{
        status = 'reported'
        inputTokens = $inputTokens
        cachedInputTokens = $cachedInputTokens
        cacheWriteInputTokens = $cacheWriteInputTokens
        outputTokens = $outputTokens
        reasoningOutputTokens = $reasoningOutputTokens
        totalTokens = $inputTokens + $outputTokens
    }
}

function New-ActualUsage {
    param([Parameter(Mandatory)]$Usage)
    if ($null -eq $Usage -or $Usage.status -ne 'reported') {
        return [ordered]@{ status = 'unavailable'; inputTokens = 0; cachedInputTokens = 0; outputTokens = 0; totalTokens = 0 }
    }
    return [ordered]@{
        status = 'reported'
        inputTokens = [long]$Usage.inputTokens
        cachedInputTokens = [long]$Usage.cachedInputTokens
        outputTokens = [long]$Usage.outputTokens
        totalTokens = [long]$Usage.totalTokens
    }
}

if ($null -eq ('MissionControlJobObject' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MissionControlJobObject {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
}
'@
}

function Attach-JobObject {
    param([Parameter(Mandatory)][System.Diagnostics.Process] $Process)
    $job = [MissionControlJobObject]::CreateJobObject([IntPtr]::Zero, $null)
    if ($job -eq [IntPtr]::Zero -or -not [MissionControlJobObject]::AssignProcessToJobObject($job, $Process.Handle)) {
        throw 'Unable to attach invocation process to a Windows Job Object.'
    }
    return $job
}

function Stop-InvocationProcessTree {
    param([Parameter(Mandatory)][System.Diagnostics.Process]$Process)
    try { & taskkill.exe /PID $Process.Id /T /F 2>$null | Out-Null } catch { }
    try { if (-not $Process.HasExited) { $Process.Kill() } } catch { }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($OutputRoot)) { $OutputRoot = Join-Path $repositoryRoot 'artifacts\real-invocation' }
$runRoot = Join-Path $OutputRoot $RunId
if (Test-Path -LiteralPath $runRoot) { throw 'OUTPUT_EXISTS: refusing to overwrite a real-invocation run.' }
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null
$allCategories = @('readonly_explanation', 'single_file_bug_fix', 'multi_file_feature', 'test_failure_repair', 'approval_required_action', 'dirty_workspace', 'long_context_compression', 'safe_pause', 'restart_recovery', 'evidence_export_redaction')
$invalidCategories = @($TaskCategories | Where-Object { $_ -notin $allCategories })
if ($invalidCategories.Count -gt 0) {
    throw "TASK_CATEGORY_INVALID: $($invalidCategories -join ', ')"
}
$categories = @(
    if ($TaskCategories.Count -eq 0) { $allCategories } else { $TaskCategories | Select-Object -Unique }
)
$restartResumeRequired = $categories -contains 'restart_recovery'
$started = [DateTime]::UtcNow
$mode = if ($SimulationMode) { 'simulation' } elseif ($IAuthorizeRealInvocation) { 'authorized-real' } else { 'fixture' }
# Keep the production default literal and auditable: realInvocation='deferred'.
$deferredRealInvocation = 'deferred'
$realInvocation = $deferredRealInvocation
$events = [System.Collections.Generic.List[string]]::new()
$fixturePath = Join-Path $repositoryRoot 'fixtures\agents\bin\fake-codex-app-server.ps1'
$taskMatrixSource = 'docs/preview/codex-task-matrix.md'
$authorizedCodexVersion = 'unavailable'
$authorizedModel = 'unavailable'
$authorizedTokenUsage = [ordered]@{ status = 'unavailable' }
$actualUsage = New-ActualUsage $authorizedTokenUsage
$budgetExceeded = $false
$violationReason = $null
$invocationExitCode = 0
$turnCompleted = $false

if ($IAuthorizeRealInvocation -and [string]::IsNullOrWhiteSpace($env:CODEX_REAL_INVOCATION_COMMAND)) {
    throw 'IAuthorizeRealInvocation requires CODEX_REAL_INVOCATION_COMMAND; no real Provider command was executed.'
}
if (-not $IAuthorizeRealInvocation) {
    Write-Output "real invocation deferred; transport=$InvocationTransport; scope=$($categories -join ','); tokenLimit=$TokenLimit; durationLimitSeconds=$DurationLimitSeconds; budgetBasis=$BudgetBasis; pass -IAuthorizeRealInvocation only after explicit user approval"
}

# The fixture path exercises the same request/response and approval bridge used by production.
if (-not $IAuthorizeRealInvocation) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'powershell.exe'
    $psi.Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$fixturePath`""
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $psi
    [void]$process.Start()
    try {
        $jobHandle = Attach-JobObject -Process $process
        function Send-Request($Value) { $process.StandardInput.WriteLine(($Value | ConvertTo-Json -Compress -Depth 20)); $process.StandardInput.Flush() }
        function Read-Events([int]$Count) {
            for ($index = 0; $index -lt $Count; $index++) {
                $task = $process.StandardOutput.ReadLineAsync()
                if (-not $task.Wait(3000)) { break }
                if ($null -eq $task.Result) { break }
                $line = Redact-Text $task.Result
                $events.Add($line)
                try {
                    $message = $line | ConvertFrom-Json
                    if ($message.method -eq 'item/commandExecution/requestApproval' -and $null -ne $message.id) {
                        Send-Request ([ordered]@{ jsonrpc = '2.0'; id = $message.id; result = [ordered]@{ decision = 'decline' } })
                    }
                } catch { }
            }
        }
        Send-Request ([ordered]@{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = [ordered]@{ clientInfo = [ordered]@{ name = 'agent-mission-control'; title = 'Agent Mission Control'; version = '0.1.0' }; capabilities = [ordered]@{} } })
        Read-Events 2
        Send-Request ([ordered]@{ jsonrpc = '2.0'; id = 2; method = 'thread/start'; params = [ordered]@{ cwd = $repositoryRoot; model = 'fixture'; sandbox = 'read-only' } })
        Read-Events 2
        Send-Request ([ordered]@{ jsonrpc = '2.0'; id = 3; method = 'turn/start'; params = [ordered]@{ threadId = 'fake-thread'; input = @([ordered]@{ type = 'text'; text = 'fixture controlled task' }) } })
        # turn/start emits eight messages; the approval response produces turn/completed.
        Read-Events 9
        Send-Request ([ordered]@{ jsonrpc = '2.0'; id = 4; method = 'thread/resume'; params = [ordered]@{ threadId = 'fake-thread'; cwd = $repositoryRoot; model = 'fixture' } })
        Read-Events 2
    } finally {
        $process.StandardInput.Close()
        if (-not $process.HasExited) { $process.Kill() }
        $process.Dispose()
    }
} else {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'cmd.exe'
    $psi.Arguments = "/d /s /c `"$($env:CODEX_REAL_INVOCATION_COMMAND)`""
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardInput = ($InvocationTransport -eq 'app-server')
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $authorizedProcess = New-Object System.Diagnostics.Process
    $authorizedProcess.StartInfo = $psi
    [void]$authorizedProcess.Start()
    try {
        $jobHandle = Attach-JobObject -Process $authorizedProcess
        $stderr = if ($InvocationTransport -eq 'exec') { $authorizedProcess.StandardError.ReadToEndAsync() } else { $null }
        if ($InvocationTransport -eq 'app-server') {
            function Send-AppServerRequest($Value) {
                $requestLine = $Value | ConvertTo-Json -Compress -Depth 20
                $authorizedProcess.StandardInput.WriteLine($requestLine)
                $authorizedProcess.StandardInput.Flush()
                $events.Add((Redact-Text $requestLine))
            }
            $nextRequestId = 1
            $handshakeStage = 'initialize'
            $inputText = 'Read docs/preview/codex-task-matrix.md and explain the readonly_explanation requirements without modifying files.'
            Send-AppServerRequest ([ordered]@{ jsonrpc = '2.0'; id = $nextRequestId; method = 'initialize'; params = [ordered]@{ clientInfo = [ordered]@{ name = 'agent-mission-control'; title = 'Agent Mission Control'; version = '0.1.0' }; capabilities = [ordered]@{} } })
            $nextRequestId++
            $stdoutLine = $authorizedProcess.StandardOutput.ReadLineAsync()
            $streamUsage = [ordered]@{ status = 'unavailable' }
            $streamThreadId = 'unknown-thread'
            $streamTurnId = 'unknown-turn'
            $streamStarted = [DateTime]::UtcNow
            while ($true) {
                if ($stdoutLine.Wait(100)) {
                    $line = $stdoutLine.Result
                    if ($null -eq $line) { break }
                    $line = Redact-Text $line
                    if (-not [string]::IsNullOrWhiteSpace($line)) {
                        $events.Add($line)
                        try {
                            $message = $line | ConvertFrom-Json
                            $messageMethodProperty = $message.PSObject.Properties['method']
                            $messageMethod = if ($null -eq $messageMethodProperty) { $null } else { [string]$messageMethodProperty.Value }
                            if ($messageMethod -eq 'thread/started') {
                                $candidate = $message.params.thread.id
                                if (-not [string]::IsNullOrWhiteSpace([string]$candidate)) { $streamThreadId = [string]$candidate }
                            }
                            if ($messageMethod -eq 'turn/started') {
                                $threadCandidate = $message.params.threadId
                                if (-not [string]::IsNullOrWhiteSpace([string]$threadCandidate)) { $streamThreadId = [string]$threadCandidate }
                                $candidate = $message.params.turn.id
                                if (-not [string]::IsNullOrWhiteSpace([string]$candidate)) { $streamTurnId = [string]$candidate }
                            }
                            $messageIdProperty = $message.PSObject.Properties['id']
                            $messageResultProperty = $message.PSObject.Properties['result']
                            $messageErrorProperty = $message.PSObject.Properties['error']
                            if ($null -ne $messageErrorProperty) {
                                $errorMessageProperty = $messageErrorProperty.Value.PSObject.Properties['message']
                                $errorMessage = if ($null -eq $errorMessageProperty) { 'unknown protocol error' } else { [string]$errorMessageProperty.Value }
                                $invocationExitCode = 1
                                $violationReason = "protocol error: $errorMessage"
                                Stop-InvocationProcessTree -Process $authorizedProcess
                                break
                            }
                            if ($null -ne $messageIdProperty -and $null -ne $messageResultProperty) {
                                $messageId = [int64]$messageIdProperty.Value
                                if ($handshakeStage -eq 'initialize' -and $messageId -eq 1) {
                                    Send-AppServerRequest ([ordered]@{ jsonrpc = '2.0'; id = $nextRequestId; method = 'thread/start'; params = [ordered]@{ cwd = $repositoryRoot; model = (Get-InvocationModel); sandbox = 'read-only' } })
                                    $nextRequestId++
                                    $handshakeStage = 'thread/start'
                                } elseif ($handshakeStage -eq 'thread/start' -and $messageId -eq 2) {
                                    $threadResult = $messageResultProperty.Value.PSObject.Properties['thread']
                                    $threadCandidate = if ($null -ne $threadResult -and $null -ne $threadResult.Value) { $threadResult.Value.PSObject.Properties['id'] } else { $null }
                                    if ($null -ne $threadCandidate -and -not [string]::IsNullOrWhiteSpace([string]$threadCandidate.Value)) { $streamThreadId = [string]$threadCandidate.Value }
                                    if ($streamThreadId -eq 'unknown-thread') {
                                        $threadIdProperty = $messageResultProperty.Value.PSObject.Properties['threadId']
                                        if ($null -ne $threadIdProperty) { $streamThreadId = [string]$threadIdProperty.Value }
                                    }
                                    Send-AppServerRequest ([ordered]@{ jsonrpc = '2.0'; id = $nextRequestId; method = 'turn/start'; params = [ordered]@{ threadId = $streamThreadId; input = @([ordered]@{ type = 'text'; text = $inputText }) } })
                                    $nextRequestId++
                                    $handshakeStage = 'turn/start'
                                }
                            }
                            if ($messageMethod -eq 'thread/tokenUsage/updated') {
                                $streamUsage = Get-TokenUsage -EventLines $events -Latest
                                $actualUsage = New-ActualUsage $streamUsage
                                if ($streamUsage.status -eq 'reported' -and [long]$streamUsage.totalTokens -gt $TokenLimit) {
                                    $budgetExceeded = $true
                                    $violationReason = 'token limit exceeded; app-server transport interrupted mid-run'
                                    $interrupt = [ordered]@{ jsonrpc = '2.0'; id = 9001; method = 'turn/interrupt'; params = [ordered]@{ threadId = $streamThreadId; turnId = $streamTurnId } }
                                    $interruptLine = $interrupt | ConvertTo-Json -Compress -Depth 20
                                    try { $authorizedProcess.StandardInput.WriteLine($interruptLine); $authorizedProcess.StandardInput.Flush() } catch { }
                                    $events.Add($interruptLine)
                                    $invocationExitCode = 1
                                    Stop-InvocationProcessTree -Process $authorizedProcess
                                    break
                                }
                            }
                            if ($messageMethod -eq 'turn/completed') {
                                $turnCompleted = $true
                                break
                            }
                        } catch { }
                    }
                    $stdoutLine = $authorizedProcess.StandardOutput.ReadLineAsync()
                } elseif (([DateTime]::UtcNow - $streamStarted).TotalSeconds -ge $DurationLimitSeconds) {
                    $violationReason = 'real Provider invocation exceeded duration limit and was aborted.'
                    $invocationExitCode = 1
                    Stop-InvocationProcessTree -Process $authorizedProcess
                    break
                }
            }
            if ($turnCompleted) {
                try { $authorizedProcess.StandardInput.Close() } catch { }
                if (-not $authorizedProcess.WaitForExit(3000)) { Stop-InvocationProcessTree -Process $authorizedProcess }
            }
            if ($budgetExceeded) {
                $authorizedTokenUsage = $streamUsage
                $actualUsage = New-ActualUsage $streamUsage
            }
        } else {
            $stdout = $authorizedProcess.StandardOutput.ReadToEndAsync()
            if (-not $authorizedProcess.WaitForExit([Math]::Max(1000, $DurationLimitSeconds * 1000))) {
                Stop-InvocationProcessTree -Process $authorizedProcess
                $violationReason = 'real Provider invocation exceeded duration limit and was aborted.'
                $invocationExitCode = 1
            }
            $stderrOutput = if ($null -ne $stderr) { $stderr.Result } else { $null }
            foreach ($streamOutput in @($stdout.Result, $stderrOutput)) {
                if ([string]::IsNullOrWhiteSpace([string]$streamOutput)) { continue }
                $redactedOutput = Redact-Text ([string]$streamOutput)
                foreach ($line in ($redactedOutput -split "`r?`n")) {
                    if (-not [string]::IsNullOrWhiteSpace($line)) { $events.Add($line) }
                }
            }
            $authorizedTokenUsage = Get-TokenUsage -EventLines $events -CompletedOnly
            $actualUsage = New-ActualUsage $authorizedTokenUsage
            if ($authorizedTokenUsage.status -eq 'reported' -and [long]$authorizedTokenUsage.totalTokens -gt $TokenLimit) {
                $budgetExceeded = $true
                $violationReason = 'post-hoc violation (exec transport cannot abort mid-run)'
                $invocationExitCode = 1
            }
        }
        $authorizedCodexVersion = Get-CodexVersion
        $authorizedModel = Get-InvocationModel
        if ($InvocationTransport -eq 'app-server' -and -not $budgetExceeded) {
            $authorizedTokenUsage = Get-TokenUsage -EventLines $events -Latest
            $actualUsage = New-ActualUsage $authorizedTokenUsage
            if ($authorizedTokenUsage.status -eq 'reported' -and [long]$authorizedTokenUsage.totalTokens -gt $TokenLimit) {
                $budgetExceeded = $true
                $violationReason = 'token limit exceeded; app-server transport interrupted mid-run'
                $invocationExitCode = 1
            }
        }
        $processExitCode = $null
        if ($authorizedProcess.HasExited) { $processExitCode = $authorizedProcess.ExitCode }
        if ($null -ne $processExitCode -and $processExitCode -ne 0 -and -not $budgetExceeded -and $invocationExitCode -eq 0) {
            $invocationExitCode = 1
            $violationReason = "authorized real invocation exited $processExitCode."
        }
        if (-not $SimulationMode -and $invocationExitCode -eq 0 -and -not $budgetExceeded) { $realInvocation = 'authorized-real' }
    } finally { $authorizedProcess.Dispose() }
}

$actualUsage = if ($IAuthorizeRealInvocation) { $actualUsage } elseif ($events.Count -eq 0) { New-ActualUsage ([ordered]@{ status = 'unavailable' }) } else { New-ActualUsage (Get-TokenUsage -EventLines $events -Latest) }
$events | Set-Content -LiteralPath (Join-Path $runRoot 'events.jsonl') -Encoding utf8
$ended = [DateTime]::UtcNow
$report = [ordered]@{
    schema = 'mission-control-real-invocation-v1'
    runId = $RunId
    generatedAtUtc = $ended.ToString('o')
    mode = $mode
    realInvocation = $realInvocation
    releaseReady = $false
    approvalDefault = 'decline'
    sandbox = 'read-only'
    tokenLimit = $TokenLimit
    budgetBasis = $BudgetBasis
    invocationTransport = $InvocationTransport
    actualUsage = $actualUsage
    budgetExceeded = $budgetExceeded
    violationReason = $violationReason
    durationLimitSeconds = $DurationLimitSeconds
    taskCategories = $categories
    taskMatrixSource = $taskMatrixSource
    restartResumeSample = $restartResumeRequired
    approvalSampleCaptured = @($events | Where-Object { $_ -match 'requestApproval' }).Count -gt 0
    jobObject = 'attached'
    redacted = $true
    redactor = 'ledger-redactor-compatible pattern set'
    costEstimate = 'requires user-provided Provider/model pricing; bounded by tokenLimit and durationLimitSeconds'
    source = if ($SimulationMode) { 'TEST/DryRun simulated event stream' } elseif ($IAuthorizeRealInvocation) { 'user-authorized command' } else { 'fake-codex-app-server fixture' }
    codexVersion = if ($IAuthorizeRealInvocation) { $authorizedCodexVersion } else { 'fixture' }
    model = if ($IAuthorizeRealInvocation) { $authorizedModel } else { 'fixture' }
    durationSeconds = [Math]::Round(($ended - $started).TotalSeconds, 3)
    tokenUsage = if ($IAuthorizeRealInvocation) { $authorizedTokenUsage } else { [ordered]@{ input = $actualUsage.inputTokens; output = $actualUsage.outputTokens } }
    eventCount = $events.Count
    evidencePath = 'events.jsonl'
}
Write-JsonUtf8 -Path (Join-Path $runRoot 'real-invocation-report.json') -Value $report
Write-Output "realInvocation=$realInvocation"
Write-Output (Join-Path $runRoot 'real-invocation-report.json')

if ($invocationExitCode -ne 0) { exit $invocationExitCode }
