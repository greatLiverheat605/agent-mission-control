[CmdletBinding()]
param(
    [switch] $Json,
    [switch] $SkipWebView2
)

$messages = New-Object 'System.Collections.Generic.List[string]'

function New-CheckResult {
    param(
        [Parameter(Mandatory)]
        [string] $Status,

        [string] $Version,

        [Nullable[int]] $Major,

        [string] $Path
    )

    [ordered]@{
        status = $Status
        version = $Version
        major = $Major
        path = $Path
    }
}

function Test-CommandVersion {
    param(
        [Parameter(Mandatory)]
        [string] $Name,

        [Parameter(Mandatory)]
        [string] $DisplayName
    )

    $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $command) {
        $messages.Add("$DisplayName is missing from PATH.")
        return New-CheckResult -Status missing
    }

    $output = @(& $command.Source --version 2>&1)
    if ($LASTEXITCODE -ne 0 -or $output.Count -eq 0) {
        $messages.Add("$DisplayName did not return a version.")
        return New-CheckResult -Status missing -Path $command.Source
    }

    $version = ($output -join ' ').Trim()
    $major = $null
    if ($version -match '(?<!\d)(\d+)\.') {
        $major = [int] $Matches[1]
    }

    New-CheckResult -Status ok -Version $version -Major $major -Path $command.Source
}

function Test-PesterVersion {
    $module = Get-Module -ListAvailable Pester | Sort-Object Version -Descending | Select-Object -First 1
    if (-not $module) {
        $messages.Add('Pester 5 or newer is not installed.')
        return New-CheckResult -Status missing
    }

    $status = if ($module.Version.Major -ge 5) { 'ok' } else { 'unsupported' }
    if ($status -ne 'ok') {
        $messages.Add("Pester $($module.Version) is installed; version 5 or newer is required.")
    }

    New-CheckResult -Status $status -Version $module.Version.ToString() -Major $module.Version.Major -Path $module.Path
}

function Test-WebView2Runtime {
    if ($SkipWebView2) {
        return New-CheckResult -Status skipped
    }

    $roots = @(
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients',
        'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients',
        'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients'
    )

    foreach ($root in $roots) {
        if (-not (Test-Path $root)) {
            continue
        }

        foreach ($key in Get-ChildItem $root -ErrorAction SilentlyContinue) {
            $registration = Get-ItemProperty $key.PSPath -ErrorAction SilentlyContinue
            if ($registration.name -like '*WebView2*' -and $registration.pv) {
                $major = $null
                if ($registration.pv -match '^(\d+)\.') {
                    $major = [int] $Matches[1]
                }

                return New-CheckResult -Status ok -Version $registration.pv -Major $major -Path $key.Name
            }
        }
    }

    $messages.Add('Microsoft Edge WebView2 Runtime is not registered.')
    New-CheckResult -Status missing
}

$node = Test-CommandVersion -Name node -DisplayName Node.js
if ($node.status -eq 'ok' -and $node.major -ne 24) {
    $node.status = 'unsupported'
    $messages.Add("Node.js 24 is required; found $($node.version).")
}

$git = Test-CommandVersion -Name git -DisplayName Git
$rust = Test-CommandVersion -Name rustc -DisplayName Rust
$cargo = Test-CommandVersion -Name cargo -DisplayName Cargo
$pester = Test-PesterVersion
$webview2 = Test-WebView2Runtime

$required = @($node, $git, $rust, $cargo, $pester)
if (-not $SkipWebView2) {
    $required += $webview2
}
$ok = @($required | Where-Object status -ne ok).Count -eq 0

$result = [ordered]@{
    ok = $ok
    node = $node
    git = $git
    rust = $rust
    cargo = $cargo
    pester = $pester
    webview2 = $webview2
    messages = $messages.ToArray()
}

if ($Json) {
    $result | ConvertTo-Json -Depth 4
} else {
    foreach ($name in 'node', 'git', 'rust', 'cargo', 'pester', 'webview2') {
        $check = $result[$name]
        $detail = if ($check.version) { " ($($check.version))" } else { '' }
        '{0}: {1}{2}' -f $name, $check.status, $detail
    }
    $result.messages | ForEach-Object { "- $_" }
}

if (-not $ok) {
    exit 1
}
