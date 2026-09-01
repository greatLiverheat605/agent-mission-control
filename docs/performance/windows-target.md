# Windows Codex Preview Resource Target

The controlled preview is supported on Windows 11 x64 with 4 logical CPU cores, 8 GB RAM, and 10 GB free disk space. The recommended profile is 8 logical cores, 16 GB RAM, and 20 GB free disk space.

The Supervisor owns the resource decision. At 80% of a configured memory, CPU, or disk budget it emits a throttle event; at 100% it emits a safe-boundary pause. Pressure never silently terminates a Mission. Soak artifacts contain only aggregate counters and opaque Mission IDs, not source, prompts, credentials, or provider payloads.
