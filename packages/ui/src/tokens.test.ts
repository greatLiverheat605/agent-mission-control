import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");

describe("mission control tokens", () => {
  it("defines structural and five semantic status tokens", () => {
    for (const token of [
      "--mc-background",
      "--mc-surface",
      "--mc-text",
      "--mc-border",
      "--mc-status-info",
      "--mc-status-verified",
      "--mc-status-waiting",
      "--mc-status-danger",
      "--mc-status-unknown",
      "--mc-status-offline",
      "--mc-status-degraded",
    ]) expect(css).toContain(token);
  });

  it("keeps focus and interaction dimensions auditable", () => {
    expect(css).toContain("--mc-target-min: 44px");
    expect(css).toContain("--mc-focus-width: 2px");
    expect(css).toContain("--mc-radius-item: 2px");
    expect(css).toContain("--mc-text-body: 14px");
    expect(css).toContain("--mc-text-telemetry: 11px");
  });
});
