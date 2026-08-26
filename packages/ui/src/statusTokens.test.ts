import { describe, expect, it } from "vitest";
import { ROUTE_STATES, STATUS_VISUALS } from "./statusTokens";

describe("route status visuals", () => {
  it("maps every route state to text, icon, line, motion, and aria", () => {
    expect(Object.keys(STATUS_VISUALS)).toEqual(ROUTE_STATES);
    for (const state of ROUTE_STATES) {
      expect(STATUS_VISUALS[state]).toMatchObject({
        label: expect.any(String),
        icon: expect.any(String),
        colorToken: expect.stringMatching(/^--mc-status-/),
        lineStyle: expect.any(String),
        motionState: expect.any(String),
        ariaDescription: expect.any(String),
      });
    }
  });

  it("distinguishes paused, blocked, abandoned, and unknown semantics", () => {
    expect(STATUS_VISUALS.Paused.lineStyle).not.toBe(STATUS_VISUALS.Blocked.lineStyle);
    expect(STATUS_VISUALS.Blocked.icon).not.toBe(STATUS_VISUALS.Abandoned.icon);
    expect(STATUS_VISUALS.Abandoned.motionState).toBe("disconnected");
    expect(STATUS_VISUALS.Unknown.ariaDescription).toContain("状态证据不完整");
  });

  it("keeps terminal, waiting, and fail-closed states semantically distinct", () => {
    expect(ROUTE_STATES).toHaveLength(11);
    expect(STATUS_VISUALS.Completed.motionState).toBe("settled");
    expect(STATUS_VISUALS.Completed.colorToken).toBe("--mc-status-verified");
    expect(STATUS_VISUALS.AwaitingPlanApproval.motionState).toBe("waiting");
    expect(STATUS_VISUALS.AwaitingAcceptance.motionState).toBe("waiting");
    expect(STATUS_VISUALS.Unknown.motionState).toBe("unknown");
    expect(STATUS_VISUALS.Unknown.colorToken).not.toBe(STATUS_VISUALS.Completed.colorToken);
    expect(new Set(ROUTE_STATES.map((state) => STATUS_VISUALS[state].ariaDescription)).size).toBe(ROUTE_STATES.length);
  });

  it("keeps the complete visual and aria mapping stable", () => {
    expect(STATUS_VISUALS).toMatchSnapshot();
  });
});
