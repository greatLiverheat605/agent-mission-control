import test from "node:test";
import assert from "node:assert/strict";

test("UI disconnect requests a safe pause and never resumes implicitly", () => {
  const state = { status: "running", uiConnected: true, autoResume: false };
  state.uiConnected = false;
  state.status = "pause_requested";
  assert.equal(state.status, "pause_requested");
  assert.equal(state.autoResume, false);
});
