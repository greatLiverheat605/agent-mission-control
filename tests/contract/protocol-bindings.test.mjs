import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import assert from "node:assert/strict";

const root = resolve(import.meta.dirname, "../..");
const generatedPath = resolve(root, "packages/protocol/src/generated.ts");

test("Rust exporter and checked-in TypeScript bindings do not drift", () => {
  const expected = readFileSync(generatedPath, "utf8").replace(/\r\n/g, "\n");
  const actual = execFileSync("cargo.exe", [
    "run",
    "-p",
    "mission-protocol",
    "--bin",
    "mission-protocol-export",
    "--locked",
    "--offline",
  ], { cwd: root, encoding: "utf8" });
  assert.equal(actual.replace(/\r\n/g, "\n"), expected);
});

test("bindings expose only explicit IPC commands and retain unknown-safe strings", () => {
  const source = readFileSync(generatedPath, "utf8");
  for (const command of [
    "Handshake",
    "CreateMission",
    "UpdateMissionContract",
    "LaunchRoute",
    "RequestSafePause",
    "ForceTerminate",
    "ResolveApproval",
    "SubscribeMission",
    "BuildRecoveryPackage",
  ]) {
    assert.match(source, new RegExp(`\\"${command}\\"`));
  }
  assert.match(source, /details\?: unknown/);
  assert.match(source, /Branded<string, "MissionId">/);
});
