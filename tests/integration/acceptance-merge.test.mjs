import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

test("autopilot worktree checkpoint acceptance merge preserves conflicts and pauses route", () => {
  const output = execFileSync("cargo", [
    "test",
    "-p",
    "mission-workspace",
    "--test",
    "merge",
    "conflict_preserves_both_sides_and_pauses_without_aborting_worktree",
    "--",
    "--exact",
  ], { encoding: "utf8", cwd: process.cwd() });
  assert.match(output, /test result: ok/);
});
