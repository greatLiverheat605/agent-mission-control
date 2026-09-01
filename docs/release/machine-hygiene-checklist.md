# Machine Hygiene Checklist

This checklist removes only the TEST certificate identified by the exact subject
`CN=Agent Mission Control TEST self-signed` and the thumbprints recorded in the
before snapshot. It never performs a wildcard or bulk certificate deletion.

## Exact cleanup procedure

1. Locate and record `Subject`, `Thumbprint`, `NotAfter`, and store paths:

   ```powershell
   Get-ChildItem Cert:\CurrentUser\My,Cert:\CurrentUser\Root |
     Where-Object Subject -eq 'CN=Agent Mission Control TEST self-signed' |
     Select-Object Subject,Thumbprint,NotAfter,PSPath
   ```

2. Run the auditable cleanup script. It writes `store-before.json`,
   `cleanup-steps.json`, and `store-after.json` under the evidence directory:

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/cleanup-test-certificates.ps1
   ```

3. The script attempts, in order, exact `Remove-Item` paths, then
   `certutil.exe -user -f -delstore <Store> <Thumbprint>` with a finite timeout,
   then the exact HKCU registry key:

   ```text
   HKCU:\Software\Microsoft\SystemCertificates\Root\Certificates\<thumbprint-lowercase>
   HKCU:\Software\Microsoft\SystemCertificates\My\Certificates\<thumbprint-lowercase>
   ```

4. Confirm `matchedAfter=0` and that CurrentUser/My, CurrentUser/Root,
   LocalMachine/My, and LocalMachine/Root contain no matching TEST Subject.
   Any `.pfx` or password file under the R6 test-signing artifact paths must be
   absent; the cleanup report records `pass-none-found` when none exists.

5. Restart Windows Explorer or the machine, then rerun the same script. A clean
   result is `status=pass`, `matchedBefore=<count>`, and `matchedAfter=0`.
   The script deliberately marks this post-restart check `user-required` so it
   never restarts a user's shell or machine implicitly.

## Evidence from Round 7

- Before snapshot: `artifacts/release-gate/r7-machine-hygiene-20260828/store-before.json`
- Cleanup steps: `artifacts/release-gate/r7-machine-hygiene-20260828/cleanup-steps.json`
- After snapshot: `artifacts/release-gate/r7-machine-hygiene-20260828/store-after.json`

The TEST certificate is not production signing evidence. Production signing
continues to require the approved CI secrets and a valid Authenticode chain.
