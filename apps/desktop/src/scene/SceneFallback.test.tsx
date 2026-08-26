import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FlightViewModel } from "@mission-control/mission-store";
import { SceneFallback } from "./SceneFallback";

afterEach(cleanup);

const flight = {
  mission: { id: "m1", label: "Fallback mission" },
  contract: { goal: "Keep controls", version: 1, drivingMode: "Manual" },
  primaryRoute: { id: "r1", state: "Blocked", derivedFrom: null },
  derivedRoutes: [{ id: "r-old", state: "Abandoned", derivedFrom: "r1" }],
  stages: [
    { id: "stage-Draft", label: "Contract", routeState: "Draft", state: "complete" },
    { id: "stage-Executing", label: "Execute", routeState: "Executing", state: "current" },
  ],
  agentPosition: { stageIndex: 1, progress: 0.5, motionState: "interrupted" },
  currentAction: { summary: "Resolve conflict", explanation: "Merge paused", impact: [], nextDecision: "Choose a resolution" },
  evidenceBatches: [], pendingApprovals: [], loadout: { provider: "Codex", model: "Current", items: [] }, budget: [], notifications: [], renderConfidence: "trusted",
} satisfies FlightViewModel;

describe("SceneFallback", () => {
  it("keeps route, branch, stages, agent, and selection controls", () => {
    const select = vi.fn();
    render(<SceneFallback flight={flight} onStageSelect={select} />);
    expect(screen.getByRole("img", { name: /Fallback mission.*Blocked.*Resolve conflict/i })).toBeTruthy();
    expect(screen.getByText("Abandoned route r-old")).toBeTruthy();
    expect(screen.getByTestId("fallback-agent")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Focus stage Executing" }));
    expect(select).toHaveBeenCalledWith("stage-Executing");
  });
});
