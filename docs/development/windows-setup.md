# Windows development setup

The prerequisite verifier is read-only. It reports the current machine state; it does not install software, download files, or change the registry.

## Required software

Install these prerequisites from their official sources:

| Prerequisite | Requirement | Official source |
| --- | --- | --- |
| Node.js | Major version 24 | [Node.js downloads](https://nodejs.org/en/download) |
| Git for Windows | Current supported release | [Git for Windows](https://git-scm.com/download/win) |
| Rust | Stable `x86_64-pc-windows-msvc` toolchain | [rustup](https://rustup.rs/) |
| Visual Studio 2022 Build Tools | **Desktop development with C++** workload | [Build Tools for Visual Studio 2022](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022) |
| Windows SDK | Windows 10 or Windows 11 SDK, selected in Visual Studio Installer | [Windows SDK](https://developer.microsoft.com/windows/downloads/windows-sdk/) |
| WebView2 Runtime | Evergreen Runtime | [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) |
| Pester | Exact version `5.7.1` | [PowerShell Gallery](https://www.powershellgallery.com/packages/Pester/5.7.1) |

Use the Visual Studio Installer to add or repair the C++ workload and Windows SDK. Install Pester separately from an elevated PowerShell prompt when a machine-wide installation is required:

```powershell
Install-Module Pester -RequiredVersion 5.7.1 -Scope AllUsers -Force
```

Open a new terminal after installing Rust so the Cargo bin directory is present in `PATH`.

## Verify the environment

Run the native version commands first:

```powershell
rustc --version
cargo --version
node --version
npm.cmd --version
git --version
Get-Module -ListAvailable Pester |
    Where-Object Version -EQ ([Version] '5.7.1') |
    Select-Object -First 1 Name, Version, Path

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
& $vswhere -version '[17.0,18.0)' -products Microsoft.VisualStudio.Product.BuildTools `
    -requires Microsoft.VisualStudio.Workload.VCTools `
    -property installationPath

Get-ChildItem (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Lib') -Directory |
    Where-Object Name -Match '^\d+\.\d+\.\d+\.\d+$' |
    Sort-Object { [version] $_.Name } -Descending |
    Select-Object -ExpandProperty Name
```

Windows 10 and Windows 11 SDK releases both use the `Windows Kits\10` directory layout.

Use `npm.cmd`, including in scripts and CI commands. Windows PowerShell can resolve bare `npm` to `npm.ps1`, which fails on machines with a restrictive execution policy.

Run the repository verifier for a concise report or JSON output:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-prerequisites.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-prerequisites.ps1 -Json
```

The command exits with code `1` when a required item is missing or unsupported. Use `-SkipWebView2` only for workflows that do not build or run the WebView2 desktop application.

Run the prerequisite tests with Pester `5.7.1`:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "`$ErrorActionPreference = 'Stop'; Import-Module Pester -RequiredVersion 5.7.1 -Force; `$result = Invoke-Pester .\scripts\tests\verify-prerequisites.Tests.ps1 -PassThru; if (`$null -eq `$result -or `$result.FailedCount -gt 0) { exit 1 }"
```

## Troubleshooting

### MSVC linker is missing

Errors such as `link.exe not found` usually mean the C++ workload is absent or the terminal has stale environment variables.

1. Open Visual Studio Installer and modify **Build Tools 2022**.
2. Select **Desktop development with C++** and a Windows 10 or Windows 11 SDK.
3. Confirm the Rust host and default toolchain:

   ```powershell
   rustup default stable-x86_64-pc-windows-msvc
   rustup show active-toolchain
   ```

4. Open a new terminal and rebuild. Use an **x64 Native Tools Command Prompt for VS 2022** when diagnosing `link.exe` discovery directly.

`LNK1104`, `LNK1181`, or missing Windows system libraries commonly indicate a missing or damaged SDK component. Repair the selected SDK in Visual Studio Installer before changing project linker flags.

### WebView2 is not detected

The verifier reads the Microsoft Edge Update `Clients` registry keys under `HKLM` and `HKCU`. If WebView2 is installed but reported as missing:

1. Install or repair the Evergreen WebView2 Runtime from Microsoft.
2. Close applications that host WebView2, then reopen the terminal.
3. Check that **Microsoft Edge WebView2 Runtime** appears in installed applications.
4. Re-run the verifier without `-SkipWebView2`.

Do not substitute the WebView2 SDK NuGet package for the Runtime. The SDK supplies build-time APIs; the Runtime supplies the installed browser engine that the verifier checks.
