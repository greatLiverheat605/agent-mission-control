# Clean VM Verification

`clean-vm` is an external prerequisite. A local developer workstation, a VM with
the Rust/Node toolchain installed, or a simulated result is not acceptable evidence.
The required host is a newly provisioned Windows 11 or Windows Server 2022+ VM with
WebView2 installed and no development toolchain.

## One-command execution

Copy the production-signed MSI to the clean VM and run from an elevated PowerShell:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\verify-clean-vm.ps1 `
  -MsiPath C:\staging\AgentMissionControl-0.1.0-preview.msi `
  -VmName AMC-clean-2022 -Execute `
  -OutputPath C:\staging\clean-vm-report.json
```

The runner must archive the resulting `clean-vm-report.json` under
`artifacts/release-gate/<run-id>/` together with the exact MSI SHA-256. A report is
accepted only when every check is `pass` and the VM identity, OS build, WebView2
version, and signer thumbprint are recorded.

## Required checks

1. Silent MSI installation completes and the expected install directory is recorded.
2. `supervisor.ready` appears, the authenticated Mission Control named pipe exists,
   and the supervisor process is connected.
3. Windows Credential Manager contains the `MissionControl/DatabaseKey/*` entry.
4. The ledger database is created and SQLCipher `quick_check` returns `ok`.
5. The first-launch UI reports `connected` through the normal desktop bridge.
6. Uninstall removes binaries, services, pipes, credentials, and temporary data that
   are owned by the product, while preserving explicitly retained user exports.
7. Update rollback rehearsal installs a signed replacement, interrupts the update,
   restores the rollback point, and verifies the ledger hash is unchanged.

Without a user-provided clean VM and explicit execution, the script writes
`status=blocked` and `releaseReady=false`. That status must remain in the gate until
an independently reviewable PASS report exists.
