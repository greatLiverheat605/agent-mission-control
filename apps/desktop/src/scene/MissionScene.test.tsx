import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { FlightViewModel } from "@mission-control/mission-store";

vi.mock("@react-three/fiber", () => ({
  Canvas: ({ children: _children, ...props }: React.ComponentProps<"canvas"> & { children?: React.ReactNode }) => <canvas {...props} data-scene-ready="true" />,
  useFrame: () => undefined,
  useThree: () => ({ setFrameloop: () => undefined }),
}));

import { agentPositionFor } from "./AgentMarker";
import { MissionScene } from "./MissionScene";
import { routeBranchVisual } from "./RouteBranch";
import { SceneRuntime } from "./SceneRuntime";
import { stageObjectId } from "./MissionSpine";
import { toNdc } from "./picking";

const flight = {
  mission: { id: "mission-7", label: "Policy UI" },
  contract: { goal: "Ship UI", version: 1, drivingMode: "Assisted" },
  primaryRoute: { id: "route-main", state: "Executing", derivedFrom: null },
  derivedRoutes: [{ id: "route-old", state: "Abandoned", derivedFrom: "route-main" }],
  stages: [
    { id: "stage-Draft", label: "Contract", routeState: "Draft", state: "complete" },
    { id: "stage-Executing", label: "Execute", routeState: "Executing", state: "current" },
  ],
  agentPosition: { stageIndex: 1, progress: 0.5, motionState: "moving" },
  currentAction: { summary: "Run tests", explanation: "Verify policy", impact: [], nextDecision: "Continue" },
  evidenceBatches: [], pendingApprovals: [], loadout: { provider: "Codex", model: "Current", items: [] }, budget: [], notifications: [], renderConfidence: "trusted",
} satisfies FlightViewModel;

describe("MissionScene", () => {
  it("renders an accessible full-bleed canvas description", () => {
    Object.defineProperty(window, "ResizeObserver", { configurable: true, value: class ResizeObserver { observe() {} unobserve() {} disconnect() {} } });
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => ({}) });
    render(<MissionScene flight={flight} />);
    expect(screen.getByRole("img", { name: /Policy UI.*Executing.*Run tests/i }).getAttribute("data-scene-ready")).toBe("true");
  });

  it("uses the 2D fallback immediately when WebGL2 is unavailable", () => {
    Object.defineProperty(window, "ResizeObserver", { configurable: true, value: class ResizeObserver { observe() {} unobserve() {} disconnect() {} } });
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => null });
    render(<MissionScene flight={flight} />);
    const fallbacks = screen.getAllByRole("img", { name: /Policy UI.*Executing.*Run tests/i });
    expect(fallbacks.find((node) => node.getAttribute("data-scene-ready") === "false")).toBeTruthy();
  });

  it("uses stable stage IDs and view-model-only agent positions", () => {
    expect(stageObjectId("route-main", "stage-Executing")).toBe("route:route-main:stage:stage-Executing");
    expect(agentPositionFor(flight)).toEqual([0.7, 0.18, -0.15]);
  });

  it("keeps abandoned routes disconnected", () => {
    expect(routeBranchVisual("Abandoned")).toMatchObject({ connected: false, lineStyle: "broken" });
  });

  it("inverts Y when converting pointer coordinates to NDC", () => {
    expect(toNdc(75, 25, { left: 25, top: 5, width: 100, height: 40 })).toEqual({ x: 0, y: 0 });
    expect(toNdc(25, 5, { left: 25, top: 5, width: 100, height: 40 })).toEqual({ x: -1, y: 1 });
  });

  it("releases owned resources and listeners once", () => {
    const runtime = new SceneRuntime();
    const dispose = vi.fn();
    const remove = vi.fn();
    runtime.own({ dispose });
    runtime.listen(remove);
    runtime.dispose();
    runtime.dispose();
    expect(dispose).toHaveBeenCalledOnce();
    expect(remove).toHaveBeenCalledOnce();
  });
});
