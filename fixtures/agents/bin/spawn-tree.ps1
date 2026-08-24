param([int]$SleepSeconds = 60)
$child = Start-Process -FilePath powershell.exe -ArgumentList "-NoProfile", "-Command", "Start-Sleep -Seconds $SleepSeconds" -PassThru
Write-Output (ConvertTo-Json @{ root = $PID; child = $child.Id })
Wait-Process -Id $child.Id
