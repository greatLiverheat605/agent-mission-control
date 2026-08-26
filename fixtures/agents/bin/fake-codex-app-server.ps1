$input | ForEach-Object {
    $raw = [string]$_
    if ([string]::IsNullOrWhiteSpace($raw)) { return }
    try { $request = $raw | ConvertFrom-Json } catch {
        Write-Output '{"type":"invalid.request"}'
        return
    }
    if ($null -ne $request.id) {
        [ordered]@{
            jsonrpc = '2.0'
            id = [int]$request.id
            result = [ordered]@{ ok = $true; method = $request.method }
        } | ConvertTo-Json -Compress
        if ($request.method -eq 'thread/start') {
            Write-Output '{"type":"thread.started","thread_id":"fake-thread"}'
            Write-Output '{"type":"item.started","item":"analysis"}'
            Write-Output '{"type":"item.completed","item":"done"}'
        }
    } elseif ($request.method -eq 'turn/interrupt') {
        Write-Output '{"type":"turn.interrupted"}'
    }
}
