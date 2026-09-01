# Codex Controlled Preview Study Protocol

The Codex technical preview uses a five-person pilot before any wider invite. Each participant works in an isolated, credential-free sample project with the controlled Codex fixture. The first ten minutes are unguided: the facilitator may observe but may not tell the participant what to click or type. Claude is out of scope for this release and `real invocation deferred` remains the default.

## Measures

- First launch completion: the participant creates a Mission, reviews the Contract, and reaches the first controlled Agent event without external guidance.
- State recognition: within 10 seconds, the participant correctly identifies the current phase, the next action, and any pending decision from the cockpit state.
- Safety comprehension: the participant can find pause/recovery and does not approve an unsafe or unreviewed action.
- Privacy: the participant sees a redacted export preview before any export and explicitly confirms the action. Local telemetry remains off.

The pilot gate is at least 80% unguided first-launch completion, at least 90% state recognition within 10 seconds, and zero P0/P1 findings. Rates are calculated locally from immutable participant records and are reported by project type, hardware profile and failure reason. Samples are not deleted or rewritten when a run fails; a rerun uses a new sample directory.

## Run order

1. Record consent, project type, hardware profile and a pseudonymous participant ID; do not record source, credentials or hidden model reasoning.
2. Start the isolated controlled Codex fixture and observe the first ten minutes without coaching.
3. Let the participant review the redacted diagnostic/export preview and actively confirm any export.
4. Record launch completion, state recognition answer/time, pause/recovery discovery, issue severity and a short redacted note.
5. Run `summarize-codex-preview.ps1` and keep the report hash with the study receipt. Do not expand beyond five participants until both thresholds and the zero-P0/P1 gate pass.

Any failed gate keeps the preview internal and sends the issue back to onboarding, state presentation or response handling for correction.
