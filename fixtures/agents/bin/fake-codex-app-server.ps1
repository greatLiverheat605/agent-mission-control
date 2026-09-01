$threadId = if ([string]::IsNullOrWhiteSpace($env:FAKE_CODEX_MISSION_ID)) { 'fake-thread' } else { "fake-thread-$($env:FAKE_CODEX_MISSION_ID)" }
$turnId = if ([string]::IsNullOrWhiteSpace($env:FAKE_CODEX_MISSION_ID)) { 'fake-turn' } else { "fake-turn-$($env:FAKE_CODEX_MISSION_ID)" }
$approvalSent = $false

function Send-Json($value) {
    $value | ConvertTo-Json -Compress -Depth 20 | Write-Output
}

function Send-Result($id, $result) {
    Send-Json ([ordered]@{ jsonrpc = '2.0'; id = $id; result = $result })
}

function Send-Error($id, $message) {
    Send-Json ([ordered]@{ jsonrpc = '2.0'; id = $id; error = [ordered]@{ code = -32602; message = $message } })
}

function Send-Notification($method, $params) {
    Send-Json ([ordered]@{ jsonrpc = '2.0'; method = $method; params = $params })
}

$input | ForEach-Object {
    $raw = [string]$_
    if ([string]::IsNullOrWhiteSpace($raw)) { return }
    try { $request = $raw | ConvertFrom-Json } catch {
        Send-Error 0 'invalid JSON'
        Send-Notification 'error' ([ordered]@{ message = 'invalid JSON' })
        return
    }

    # Approval responses are JSON-RPC responses, not new requests.
    if ($null -eq $request.method -and $null -ne $request.id) {
        if ([int64]$request.id -eq 900) {
            Send-Notification 'turn/completed' ([ordered]@{ threadId = $threadId; turn = [ordered]@{ id = $turnId; items = @(); status = 'completed' } })
        }
        return
    }

    if ($null -eq $request.id -or $null -eq $request.method) { return }
    switch ([string]$request.method) {
        'initialize' {
            if ($null -eq $request.params.clientInfo -or [string]::IsNullOrWhiteSpace([string]$request.params.clientInfo.name)) {
                Send-Error $request.id 'clientInfo is required'
            } else {
                Send-Result $request.id ([ordered]@{})
                Send-Notification 'warning' ([ordered]@{ message = 'fixture server' })
            }
        }
        'thread/start' {
            $sandbox = $request.params.sandbox
            $sandboxValues = @('read-only', 'workspace-write', 'danger-full-access')
            if ($sandbox -isnot [string] -or [string]$sandbox -notin $sandboxValues) {
                Send-Error $request.id 'sandbox must be a SandboxMode string enum'
                continue
            }
            $thread = [ordered]@{ id = $threadId; status = [ordered]@{ type = 'idle' } }
            Send-Result $request.id ([ordered]@{ thread = $thread; cwd = [string]$request.params.cwd; model = [string]$request.params.model; modelProvider = 'fixture'; sandbox = $sandbox; approvalPolicy = 'on-request'; approvalsReviewer = 'user' })
            Send-Notification 'thread/started' ([ordered]@{ thread = $thread })
        }
        'thread/resume' {
            $threadId = [string]$request.params.threadId
            $thread = [ordered]@{ id = $threadId; status = [ordered]@{ type = 'idle' } }
            Send-Result $request.id ([ordered]@{ thread = $thread; cwd = [string]$request.params.cwd; model = [string]$request.params.model; modelProvider = 'fixture'; sandbox = 'read-only'; approvalPolicy = 'on-request'; approvalsReviewer = 'user' })
            Send-Notification 'thread/started' ([ordered]@{ thread = $thread })
        }
        'turn/start' {
            $turnInput = $request.params.input
            $inputIsArray = $null -ne $turnInput -and $turnInput -is [System.Array]
            $inputIsValid = $inputIsArray -and $turnInput.Count -gt 0
            if ($inputIsValid) {
                foreach ($entry in $turnInput) {
                    if ($null -eq $entry -or [string]$entry.type -ne 'text' -or $null -eq $entry.text -or $entry.text -isnot [string]) {
                        $inputIsValid = $false
                        break
                    }
                }
            }
            if (-not $inputIsValid -or [string]::IsNullOrWhiteSpace([string]$request.params.threadId)) {
                Send-Error $request.id 'turn/start requires threadId and input text array'
                continue
            }
            $turn = [ordered]@{ id = $turnId; items = @(); status = 'inProgress' }
            Send-Result $request.id ([ordered]@{ turn = $turn })
            Send-Notification 'turn/started' ([ordered]@{ threadId = $threadId; turn = $turn })
            Send-Notification 'item/started' ([ordered]@{ threadId = $threadId; turnId = $turnId; item = [ordered]@{ id = 'item-1'; type = 'agentMessage' }; startedAtMs = 1 })
            Send-Notification 'item/agentMessage/delta' ([ordered]@{ threadId = $threadId; turnId = $turnId; itemId = 'item-1'; delta = 'fixture' })
            Send-Notification 'item/completed' ([ordered]@{ threadId = $threadId; turnId = $turnId; item = [ordered]@{ id = 'item-1'; type = 'agentMessage'; status = 'completed' } })
            Send-Notification 'turn/diff/updated' ([ordered]@{ threadId = $threadId; turnId = $turnId; diff = '' })
            Send-Notification 'thread/tokenUsage/updated' ([ordered]@{ threadId = $threadId; turnId = $turnId; tokenUsage = [ordered]@{ inputTokens = 1; outputTokens = 1 } })
            if (-not $approvalSent) {
                $approvalSent = $true
                Send-Json ([ordered]@{ jsonrpc = '2.0'; id = 900; method = 'item/commandExecution/requestApproval'; params = [ordered]@{ itemId = 'item-1'; startedAtMs = 1; threadId = $threadId; turnId = $turnId; command = 'Write-Output fixture'; reason = 'fixture approval' } })
            }
        }
        'turn/interrupt' {
            Send-Result $request.id ([ordered]@{})
            Send-Notification 'turn/completed' ([ordered]@{ threadId = $threadId; turn = [ordered]@{ id = $turnId; items = @(); status = 'interrupted' } })
        }
        default { Send-Result $request.id ([ordered]@{ ok = $true; method = [string]$request.method }) }
    }
}
