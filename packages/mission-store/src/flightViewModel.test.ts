import { describe, expect, it } from "vitest";
import { emptyMission, type MissionReadModel } from "./reducer";
import { ROUTE_STATES, toFlightViewModel, type RouteState } from "./flightViewModel";

const EXPECTED_MOTION = {
  Draft: "unknown",
  ReadOnlyExploration: "unknown",
  AwaitingPlanApproval: "waiting",
  Executing: "moving",
  Verifying: "checking",
  AwaitingAcceptance: "waiting",
  Completed: "settled",
  Paused: "paused",
  Blocked: "interrupted",
  Abandoned: "interrupted",
  Unknown: "unknown",
} as const satisfies Record<RouteState, ReturnType<typeof toFlightViewModel>["agentPosition"]["motionState"]>;

describe("toFlightViewModel", () => {
  it("projects route state, contract, action, and evidence without UI dependencies", () => {
    const mission: MissionReadModel = {
      ...emptyMission("mission-7"),
      phase: "Executing",
      status: "running",
      currentAction: "Run cargo test",
      lastSequence: 3,
      events: [
        { missionId: "mission-7", sequence: 1, kind: "mission_created", payload: { goal: "Ship policy UI", contract_version: 4, driving_mode: "Autopilot", route_id: "route-main" } },
        { missionId: "mission-7", sequence: 2, kind: "route_state_changed", payload: { state: "Executing", route_id: "route-main" } },
        { missionId: "mission-7", sequence: 3, kind: "test_completed", source: "supervisor", payload: { summary: "Policy tests passed", evidence_id: "ev-3", confidence: "verified", files: ["crates/policy/src/engine.rs"] } },
      ],
    };

    const flight = toFlightViewModel(mission);
    expect(flight.primaryRoute).toMatchObject({ id: "route-main", state: "Executing" });
    expect(flight.contract).toMatchObject({ goal: "Ship policy UI", version: 4, drivingMode: "Autopilot" });
    expect(flight.currentAction.summary).toBe("Run cargo test");
    expect(flight.evidenceBatches[0]).toMatchObject({ id: "ev-3", confidence: "verified", source: "supervisor" });
    expect(flight.renderConfidence).toBe("trusted");
  });

  it("fails closed when route evidence is missing or terminal-derived", () => {
    const mission: MissionReadModel = {
      ...emptyMission("mission-unknown"),
      phase: "Working",
      status: "running",
      currentAction: "Maybe done",
      events: [{ missionId: "mission-unknown", sequence: 1, kind: "terminal_output", source: "agent", payload: { text: "finished" } }],
    };

    const flight = toFlightViewModel(mission);
    expect(flight.primaryRoute.state).toBe("Unknown");
    expect(flight.renderConfidence).toBe("incomplete");
    expect(flight.agentPosition.motionState).not.toBe("settled");
  });

  it("projects concurrent project missions without UI event parsing", () => {
    const mission: MissionReadModel = {
      ...emptyMission("mission-primary"),
      events: [
        { missionId: "mission-primary", sequence: 1, kind: "mission_created", payload: { name: "Primary", route_id: "route-primary" } },
        { missionId: "mission-primary", sequence: 2, kind: "route_state_changed", payload: { state: "Executing", route_id: "route-primary" } },
        { missionId: "mission-primary", sequence: 3, kind: "mission_peer_snapshot", payload: { missions: [
          { id: "mission-primary", label: "Primary", route_state: "Executing", action: "Build UI" },
          { id: "mission-peer", label: "Peer", route_state: "Verifying", action: "Run tests" },
        ] } },
      ],
    };

    expect(toFlightViewModel(mission).projectMissions).toEqual([
      { id: "mission-primary", label: "Primary", routeState: "Executing", action: "Build UI" },
      { id: "mission-peer", label: "Peer", routeState: "Verifying", action: "Run tests" },
    ]);
  });

  it.each(ROUTE_STATES)("maps %s to an explicit motion and confidence contract", (routeState) => {
    const mission: MissionReadModel = {
      ...emptyMission(`mission-${routeState}`),
      phase: routeState,
      status: routeState === "Completed" ? "completed" : routeState === "Paused" ? "paused" : "running",
      currentAction: `Observe ${routeState}`,
      events: [
        { missionId: `mission-${routeState}`, sequence: 1, kind: "mission_created", source: "supervisor", payload: { route_id: `route-${routeState}` } },
        { missionId: `mission-${routeState}`, sequence: 2, kind: "route_state_changed", source: "supervisor", payload: { state: routeState, route_id: `route-${routeState}` } },
      ],
    };

    const flight = toFlightViewModel(mission);
    expect(flight.primaryRoute.state).toBe(routeState);
    expect(flight.agentPosition.motionState).toBe(EXPECTED_MOTION[routeState]);
    expect(flight.renderConfidence).toBe(routeState === "Unknown" ? "incomplete" : "trusted");
    if (routeState === "Unknown") {
      expect(flight.stages.every((stage) => stage.state === "unknown")).toBe(true);
      expect(flight.agentPosition.progress).toBe(0);
    }
  });

  it("marks agent-sourced route evidence degraded without promoting it to trusted", () => {
    const mission: MissionReadModel = {
      ...emptyMission("mission-degraded"),
      events: [
        { missionId: "mission-degraded", sequence: 1, kind: "mission_created", source: "supervisor", payload: { route_id: "route-degraded" } },
        { missionId: "mission-degraded", sequence: 2, kind: "route_state_changed", source: "agent", payload: { state: "Verifying", route_id: "route-degraded" } },
      ],
    };
    const flight = toFlightViewModel(mission);
    expect(flight.primaryRoute.state).toBe("Verifying");
    expect(flight.renderConfidence).toBe("degraded");
    expect(flight.agentPosition.motionState).toBe("checking");
  });

  it("projects nested approval events and removes resolved approvals", () => {
    const mission: MissionReadModel = {
      ...emptyMission("mission-approval"),
      events: [
        { missionId: "mission-approval", sequence: 1, kind: "mission_created", payload: { route_id: "route-approval" } },
        { missionId: "mission-approval", sequence: 2, kind: "approval_requested", payload: {
          approval: {
            id: "approval-nested",
            state: "pending",
            scope: "once",
            expires_at_ms: 1_900_000_000_000,
            subject: { action_class: "write" },
          },
        } },
        { missionId: "mission-approval", sequence: 3, kind: "approval_resolved", payload: {
          approval: { id: "approval-nested", state: "approved", scope: "once", subject: { action_class: "write" } },
        } },
      ],
    };
    expect(toFlightViewModel(mission).pendingApprovals).toEqual([]);
  });
});
