import { useEffect, useState } from "react";
import { emptyMission, type MissionEvent, type MissionReadModel } from "@mission-control/mission-store";
import { BasicMissionFlight } from "../features/mission/BasicMissionFlight";
import { VISUAL_ROUTE_EVENT, type VisualFixtureConfig } from "./visualFixture";

export function VisualFixtureApp({ config }: { config: VisualFixtureConfig }) {
  const [mission, setMission] = useState<MissionReadModel>(() => fixtureMission(config));
  useEffect(() => {
    const updateRouteState = (event: Event) => {
      const routeState = (event as CustomEvent<VisualFixtureConfig["routeState"]>).detail;
      setMission(fixtureMission({ ...config, routeState }));
    };
    window.addEventListener(VISUAL_ROUTE_EVENT, updateRouteState);
    return () => window.removeEventListener(VISUAL_ROUTE_EVENT, updateRouteState);
  }, [config]);
  return <BasicMissionFlight
    mission={mission}
    events={mission.events}
    initialView={config.view}
    forceSceneFallback={config.webgl === "fallback"}
    forceReducedMotion={config.motion === "reduced"}
    onPause={() => setMission((current) => ({ ...current, phase: "Paused", status: "paused", currentAction: "Safe pause requested", reason: "user requested safe pause" }))}
    onReconnect={() => undefined}
    onDiscard={() => undefined}
    onHandoff={() => undefined}
  />;
}

export function fixtureMission(config: VisualFixtureConfig): MissionReadModel {
  const { routeState, contentCase } = config;
  const missionId = "mission-visual-primary";
  const routeId = "route-primary";
  const root = "C:\\workspace\\示例项目\\packages\\mission-control-with-a-deliberately-long-name";
  const long = contentCase === "long";
  const empty = contentCase === "empty";
  const error = contentCase === "error";
  const offline = contentCase === "offline";
  const missionName = long ? "Control Coding · 超长任务名称用于验证缩放与文字溢出以及多语言舰桥布局" : "Control Coding";
  const goal = long ? "完成完整 P6 视觉发布门并保留所有安全边界、证据链和不可伪造的 Supervisor 事实" : "Complete the visual release gate without changing safety boundaries";
  const currentAction = error
    ? "Supervisor channel failed during evidence synchronization"
    : long
      ? "Run visual release gates across every target viewport without truncating the operator command"
      : "Run visual release gates";
  const events: MissionEvent[] = [
    event(missionId, 1, "mission_created", { name: missionName, goal, contract_version: 6, driving_mode: "Assisted", route_id: routeId }),
    event(missionId, 2, "route_state_changed", { state: routeState, route_id: routeId }),
  ];
  if (!empty) events.push(
    event(missionId, events.length + 1, "mission_peer_snapshot", { missions: [
      { id: missionId, label: missionName, route_state: routeState, action: currentAction },
      { id: "mission-91c4", label: "Dependency audit sweep", route_state: "Verifying", action: "Auditing lockfile drift" },
      { id: "mission-2bd8", label: "Docs rebuild", route_state: "Paused", action: "Holding at checkpoint 12" },
      { id: "mission-55e0", label: "Telemetry exporter", route_state: "Draft", action: "Awaiting plan approval" },
      { id: "mission-77aa", label: "Flaky test hunt", route_state: "Blocked", action: "File lock conflict" },
      { id: "mission-08ff", label: "Release cut 0.9.3", route_state: "AwaitingAcceptance", action: "Staged for acceptance" },
      { id: "mission-3e19", label: "Cache migration", route_state: "Completed", action: "Route closed" },
      { id: "mission-6d02", label: "Log pipeline spike", route_state: "ReadOnlyExploration", action: "Read-only scan" },
      { id: "mission-44b7", label: "Legacy shim removal", route_state: "Abandoned", action: "Abandoned at checkpoint 29" },
      { id: "mission-9a58", label: "Orphan probe", route_state: "Unknown", action: "No telemetry fix" },
      { id: "mission-c310", label: "Benchmark harness", route_state: "AwaitingPlanApproval", action: "Plan submitted" },
    ] }),
    event(missionId, events.length + 2, "loadout_snapshot", { provider: "Codex", model: "GPT-5", fingerprint: "fixture-loadout-v1", items: [{ name: "Playwright", status: "loaded", source: "plugin" }, { name: "Shrimp MCP", status: "loaded", source: "native" }] }),
    event(missionId, events.length + 3, "budget_updated", { dimensions: [{ dimension: "Tokens", used: 72000, limit: 120000, unit: "tokens", status: "normal" }, { dimension: "Context", used: 6800, limit: 10000, unit: "tokens", status: "normal" }] }),
    event(missionId, events.length + 4, "route_derived", { route_id: "route-abandoned", state: "Abandoned", derived_from: routeId }),
    event(missionId, events.length + 5, "test_completed", { summary: "Visual viewport and Canvas pixel checks passed", evidence_id: "evidence-visual", confidence: "verified", files: [`${root}\\apps\\desktop\\tests\\visual\\mission-flight.spec.ts`] }),
    event(missionId, events.length + 6, "approval_requested", { approval_id: "approval-visual", action: "Run signed release packaging", scope: "Single release command", expires_at: "2026-08-25T21:00:00+08:00" }),
    event(missionId, events.length + 7, "memory_item_changed", { item: { id: "memory-visual-1", kind: "constraint", content: "Keep the release workspace read-only", source_event_ids: ["event-visual-1"], scope: "mission", freshness: "fresh", version: 1, status: "candidate", author: "user" } }),
    event(missionId, events.length + 8, "memory_item_changed", { item: { id: "memory-visual-2", kind: "fact", content: "Visual release gate passed", source_event_ids: ["event-visual-2"], scope: "route", freshness: "fresh", version: 2, status: "confirmed", author: "user" } }),
    event(missionId, events.length + 9, "context_pack_built", { hash: "fixture-context-v1" }),
    event(missionId, events.length + 10, "checkpoint_created", { checkpoint_id: "checkpoint-visual-1" }),
  );
  if (error) events.push(event(missionId, events.length + 1, "supervisor_error", { message: currentAction, reason: "Evidence transport unavailable" }));
  if (offline) events.push(event(missionId, events.length + 1, "connection_warning", { message: "Supervisor connection offline", reason: "ui disconnected" }));
  return {
    ...emptyMission(missionId),
    phase: routeState,
    status: offline || routeState === "Paused" ? "paused" : routeState === "Completed" ? "completed" : "running",
    currentAction,
    reason: offline ? "ui disconnected: Supervisor connection offline" : routeState === "Blocked" ? "Waiting for conflict resolution" : error ? "Evidence transport unavailable" : null,
    lastSequence: events.length,
    events,
  };
}

function event(missionId: string, sequence: number, kind: string, payload: Record<string, unknown>): MissionEvent {
  return { missionId, sequence, kind, payload, source: "supervisor" };
}
