import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { emptyMission, type FlightViewModel, type MissionEvent } from "@mission-control/mission-store";
import { ApprovalDock } from "./approval/ApprovalDock";
import { BudgetTelemetry } from "./budget/BudgetTelemetry";
import { ContractOrbit } from "./contract/ContractOrbit";
import { EvidenceBay } from "./evidence/EvidenceBay";
import { LoadoutPanel } from "./loadout/LoadoutPanel";
import { BasicMissionFlight } from "./mission/BasicMissionFlight";

const flight = {
  mission: { id: "mission-long-identifier", label: "Policy mission" },
  contract: { goal: "Ship bounded autopilot", version: 4, drivingMode: "Autopilot" },
  primaryRoute: { id: "route-main", state: "Executing", derivedFrom: null },
  derivedRoutes: [],
  stages: [],
  agentPosition: { stageIndex: 3, progress: 0.5, motionState: "moving" },
  currentAction: { summary: "Run policy tests", explanation: "Verify approval boundaries", impact: ["C:/very/long/project/path/crates/policy/src/engine.rs"], nextDecision: "Review test evidence" },
  evidenceBatches: [{ id: "ev-8", summary: "Policy tests passed", count: 3, source: "supervisor", confidence: "verified", sequences: [7, 8, 9], files: ["C:/very/long/project/path/crates/policy/src/engine.rs"] }],
  pendingApprovals: [{ id: "approval-1", action: "Install dependency", scope: "Single action", expiresAt: "2026-08-25T20:00:00Z" }],
  loadout: { provider: "Codex", model: "gpt-5", items: [{ name: "workspace MCP", status: "loaded", source: "native" }, { name: "remote plugin", status: "disabled", source: "project" }] },
  budget: [{ dimension: "tokens", used: 800, limit: 1000, unit: "tokens", status: "warning" }],
  notifications: [], renderConfidence: "trusted",
} satisfies FlightViewModel;

afterEach(cleanup);

describe("mission edge surfaces", () => {
  it("keeps contract and action context visible in Autopilot", () => {
    render(<ContractOrbit flight={flight} onEdit={vi.fn()} />);
    expect(screen.getByText("Ship bounded autopilot")).toBeTruthy();
    expect(screen.getByText("Autopilot")).toBeTruthy();
    expect(screen.getByText("Run policy tests")).toBeTruthy();
    expect(screen.getByText("Verify approval boundaries")).toBeTruthy();
    expect(screen.getByText("Review test evidence")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Edit contract" })).toBeTruthy();
  });

  it("shows evidence result, source, confidence, and full path access", () => {
    render(<EvidenceBay batches={flight.evidenceBatches} />);
    expect(screen.getByText("Policy tests passed")).toBeTruthy();
    expect(screen.getByText(/supervisor.*verified/i)).toBeTruthy();
    expect(screen.getByTitle("C:/very/long/project/path/crates/policy/src/engine.rs")).toBeTruthy();
  });

  it("shows loadout source/status and numeric budget threshold", () => {
    const { rerender } = render(<LoadoutPanel loadout={flight.loadout} />);
    expect(screen.getByText(/workspace MCP/)).toBeTruthy();
    expect(screen.getByText(/disabled.*project/i)).toBeTruthy();
    rerender(<BudgetTelemetry budget={flight.budget} />);
    expect(screen.getByText("800 / 1,000 tokens")).toBeTruthy();
    expect(screen.getByText(/80%.*warning/i)).toBeTruthy();
  });

  it("keeps approval scope, expiry, and distinct grant choices visible", () => {
    render(<ApprovalDock approvals={flight.pendingApprovals} onResolve={vi.fn()} />);
    expect(screen.getByText("Install dependency")).toBeTruthy();
    expect(screen.getByText("Single action")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Approve once" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Approve similar actions in this route" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Deny approval" })).toBeTruthy();
  });

  it("maps every operational feature into a reachable cockpit view", () => {
    const events: MissionEvent[] = [
      { missionId: "mission-7", sequence: 1, kind: "mission_created", payload: { goal: "Pilot the workspace", contract_version: 2 }, source: "supervisor" },
      { missionId: "mission-7", sequence: 2, kind: "route_state_changed", payload: { state: "Executing", route_id: "route-main" }, source: "supervisor" },
      { missionId: "mission-7", sequence: 3, kind: "approval_requested", payload: { approval_id: "approval-3", action: "Install dependency", scope: "Single action" }, source: "supervisor" },
      { missionId: "mission-7", sequence: 4, kind: "loadout_snapshot", payload: { provider: "Codex", model: "gpt-5", items: [{ name: "workspace MCP", status: "loaded", source: "native" }] }, source: "supervisor" },
      { missionId: "mission-7", sequence: 5, kind: "budget_updated", payload: { dimensions: [{ dimension: "tokens", used: 40, limit: 100, unit: "tokens", status: "normal" }] }, source: "supervisor" },
      { missionId: "mission-7", sequence: 6, kind: "test_completed", payload: { evidence_id: "ev-3", summary: "Checks passed", confidence: "verified" }, source: "supervisor" },
    ];
    const mission = { ...emptyMission("mission-7"), phase: "Execute", status: "running" as const, currentAction: "Run cockpit checks", events };
    render(<BasicMissionFlight mission={mission} events={events} onPause={vi.fn()} onReconnect={vi.fn()} onDiscard={vi.fn()} />);

    fireEvent.click(screen.getByRole("tab", { name: /Sector/ }));
    expect(within(screen.getByRole("region", { name: "Sector display" })).getByRole("heading", { name: "Missions" })).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /^Mission$/ }));
    expect(within(screen.getByRole("region", { name: "Mission display" })).getByRole("heading", { name: "Mission contract" })).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /Records/ }));
    expect(within(screen.getByRole("region", { name: "Records display" })).getByText("Checks passed")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /Systems/ }));
    expect(within(screen.getByRole("region", { name: "Systems display" })).getByRole("group", { name: "Render quality" })).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /Authority/ }));
    const authority = within(screen.getByRole("region", { name: "Authority display" }));
    expect(authority.getByText("Install dependency")).toBeTruthy();
    expect((authority.getByRole("button", { name: "Approve once" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
