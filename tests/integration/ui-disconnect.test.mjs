import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

test("ledger-backed UI disconnect recovery pauses and never resumes implicitly", () => {
  const output = execFileSync("cargo", [
    "test",
    "-p",
    "mission-supervisor",
    "--test",
    "ui_disconnect_recovery",
    "--",
    "--exact",
    "ui_disconnect_is_persisted_and_restart_does_not_resume_agent",
  ], { encoding: "utf8", cwd: process.cwd() });
  assert.match(output, /test result: ok/);
});
