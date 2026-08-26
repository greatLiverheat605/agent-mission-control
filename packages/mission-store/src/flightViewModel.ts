import type { MissionEvent, MissionReadModel } from "./reducer";

export const ROUTE_STATES = [
  "Draft",
  "ReadOnlyExploration",
  "AwaitingPlanApproval",
  "Executing",
  "Verifying",
  "AwaitingAcceptance",
  "Completed",
  "Paused",
  "Blocked",
  "Abandoned",
  "Unknown",
] as const;

export type RouteState = typeof ROUTE_STATES[number];
export type RenderConfidence = "trusted" | "degraded" | "incomplete";

export type FlightStage = {
  id: string;
  label: string;
  routeState: Exclude<RouteState, "Paused" | "Blocked" | "Abandoned" | "Unknown">;
  state: "complete" | "current" | "upcoming" | "unknown";
};

export type EvidenceBatchView = {
  id: string;
  summary: string;
  count: number;
  source: MissionEvent["source"] | "unknown";
  confidence: "verified" | "reported" | "unknown";
  sequences: number[];
  files: string[];
};

export type FlightViewModel = {
  mission: { id: string; label: string };
  projectMissions?: Array<{ id: string; label: string; routeState: RouteState; action: string }>;
  contract: { goal: string; version: number; drivingMode: string };
  primaryRoute: { id: string; state: RouteState; derivedFrom: string | null };
  derivedRoutes: Array<{ id: string; state: RouteState; derivedFrom: string }>;
  stages: FlightStage[];
  agentPosition: { stageIndex: number; progress: number; motionState: "moving" | "checking" | "waiting" | "paused" | "interrupted" | "settled" | "unknown" };
  currentAction: { summary: string; explanation: string; impact: string[]; nextDecision: string };
  evidenceBatches: EvidenceBatchView[];
  pendingApprovals: Array<{ id: string; action: string; scope: string; expiresAt: string | null }>;
  loadout: { provider: string; model: string; items: Array<{ name: string; status: string; source: string }> };
  budget: Array<{ dimension: string; used: number | null; limit: number | null; unit: string; status: string }>;
  notifications: Array<{ id: string; level: string; message: string }>;
  renderConfidence: RenderConfidence;
};

const STAGE_STATES = ROUTE_STATES.filter((state): state is FlightStage["routeState"] =>
  !["Paused", "Blocked", "Abandoned", "Unknown"].includes(state),
);

const STAGE_LABELS: Record<FlightStage["routeState"], string> = {
  Draft: "Contract",
  ReadOnlyExploration: "Explore",
  AwaitingPlanApproval: "Plan",
  Executing: "Execute",
  Verifying: "Verify",
  AwaitingAcceptance: "Accept",
  Completed: "Complete",
};

export function toFlightViewModel(model: MissionReadModel): FlightViewModel {
  const events = [...model.events].sort((left, right) => left.sequence - right.sequence);
  const contractEvent = lastEvent(events, "mission_created", "contract_updated");
  const routeEvent = lastEvent(events, "route_state_changed", "route_created");
  const routeState = routeEvent ? normalizeRouteState(routeEvent.payload.state) : "Unknown";
  const routeId = stringValue(routeEvent?.payload.route_id) ?? stringValue(contractEvent?.payload.route_id) ?? `${model.missionId}-route`;
  const trustedRouteEvidence = routeState !== "Unknown" && routeEvent?.source !== "agent";
  const renderConfidence: RenderConfidence = trustedRouteEvidence ? "trusted" : routeState === "Unknown" ? "incomplete" : "degraded";
  const activeStage = stageIndex(routeState);
  const evidenceBatches = evidenceViews(events);
  const missionLabel = stringValue(contractEvent?.payload.name) ?? shortId(model.missionId);

  return {
    mission: { id: model.missionId, label: missionLabel },
    projectMissions: projectMissionsView(events, { id: model.missionId, label: missionLabel, routeState, action: model.currentAction ?? "Waiting" }),
    contract: {
      goal: stringValue(contractEvent?.payload.goal) ?? "Goal not recorded",
      version: numberValue(contractEvent?.payload.contract_version) ?? 1,
      drivingMode: stringValue(contractEvent?.payload.driving_mode) ?? "Manual",
    },
    primaryRoute: { id: routeId, state: routeState, derivedFrom: stringValue(routeEvent?.payload.derived_from) },
    derivedRoutes: events.filter((event) => event.kind === "route_derived").map((event) => ({
      id: stringValue(event.payload.route_id) ?? `route-${event.sequence}`,
      state: normalizeRouteState(event.payload.state),
      derivedFrom: stringValue(event.payload.derived_from) ?? routeId,
    })),
    stages: STAGE_STATES.map((state, index) => ({
      id: `stage-${state}`,
      label: STAGE_LABELS[state],
      routeState: state,
      state: routeState === "Unknown" ? "unknown" : index < activeStage ? "complete" : index === activeStage ? "current" : "upcoming",
    })),
    agentPosition: {
      stageIndex: Math.max(0, activeStage),
      progress: routeState === "Completed" ? 1 : routeState === "Unknown" ? 0 : 0.5,
      motionState: motionFor(routeState),
    },
    currentAction: actionView(model, events, routeState),
    evidenceBatches,
    pendingApprovals: events.filter((event) => event.kind === "approval_requested").map((event) => ({
      id: stringValue(event.payload.approval_id) ?? `approval-${event.sequence}`,
      action: stringValue(event.payload.action) ?? "Unknown action",
      scope: stringValue(event.payload.scope) ?? "Single action",
      expiresAt: stringValue(event.payload.expires_at),
    })),
    loadout: loadoutView(events),
    budget: budgetView(events),
    notifications: events.filter((event) => event.kind.includes("warning") || event.kind.includes("error")).map((event) => ({
      id: `notification-${event.sequence}`,
      level: event.kind.includes("error") ? "danger" : "warning",
      message: stringValue(event.payload.message) ?? stringValue(event.payload.reason) ?? event.kind,
    })),
    renderConfidence,
  };
}

function projectMissionsView(
  events: MissionEvent[],
  active: NonNullable<FlightViewModel["projectMissions"]>[number],
): NonNullable<FlightViewModel["projectMissions"]> {
  const event = lastEvent(events, "mission_peer_snapshot");
  const raw = Array.isArray(event?.payload.missions) ? event.payload.missions : [];
  const peers = raw.map((value) => objectValue(value)).map((mission, index) => ({
    id: stringValue(mission.id) ?? `peer-${index + 1}`,
    label: stringValue(mission.label) ?? `Mission ${index + 1}`,
    routeState: normalizeRouteState(mission.route_state),
    action: stringValue(mission.action) ?? "Waiting",
  }));
  return peers.some((mission) => mission.id === active.id) ? peers : [active, ...peers];
}

export function normalizeRouteState(value: unknown): RouteState {
  if (typeof value !== "string") return "Unknown";
  const normalized = value.replace(/[_\s-]/g, "").toLowerCase();
  return ROUTE_STATES.find((state) => state.toLowerCase() === normalized) ?? "Unknown";
}

function stageIndex(state: RouteState): number {
  if (state === "Paused" || state === "Blocked") return STAGE_STATES.indexOf("Executing");
  if (state === "Abandoned") return STAGE_STATES.indexOf("Executing");
  return STAGE_STATES.indexOf(state as FlightStage["routeState"]);
}

function motionFor(state: RouteState): FlightViewModel["agentPosition"]["motionState"] {
  if (state === "Executing") return "moving";
  if (state === "Verifying") return "checking";
  if (state === "AwaitingPlanApproval" || state === "AwaitingAcceptance") return "waiting";
  if (state === "Paused") return "paused";
  if (state === "Blocked" || state === "Abandoned") return "interrupted";
  if (state === "Completed") return "settled";
  return "unknown";
}

function actionView(model: MissionReadModel, events: MissionEvent[], state: RouteState): FlightViewModel["currentAction"] {
  const event = [...events].reverse().find((candidate) => candidate.kind !== "route_state_changed");
  return {
    summary: model.currentAction ?? stringValue(event?.payload.summary) ?? "Waiting for the next supervised action",
    explanation: stringValue(event?.payload.explanation) ?? model.reason ?? "No additional explanation recorded",
    impact: stringArray(event?.payload.files),
    nextDecision: state === "AwaitingPlanApproval" ? "Approve or revise the plan" : state === "AwaitingAcceptance" ? "Review evidence and accept or revise" : state === "Blocked" ? "Resolve the blocking condition" : state === "Unknown" ? "Refresh route evidence" : "Continue within the mission contract",
  };
}

function evidenceViews(events: MissionEvent[]): EvidenceBatchView[] {
  return events.filter((event) => event.payload.evidence_id || /test|evidence|command|file/.test(event.kind)).slice(-12).map((event) => ({
    id: stringValue(event.payload.evidence_id) ?? `event-${event.sequence}`,
    summary: stringValue(event.payload.summary) ?? event.kind.replaceAll("_", " "),
    count: numberValue(event.payload.count) ?? 1,
    source: event.source ?? "unknown",
    confidence: confidenceValue(event.payload.confidence),
    sequences: [event.sequence],
    files: stringArray(event.payload.files),
  }));
}

function loadoutView(events: MissionEvent[]): FlightViewModel["loadout"] {
  const event = lastEvent(events, "loadout_snapshot");
  const rawItems = Array.isArray(event?.payload.items) ? event.payload.items : [];
  return {
    provider: stringValue(event?.payload.provider) ?? "Codex",
    model: stringValue(event?.payload.model) ?? "Current model",
    items: rawItems.map((value, index) => {
      const item = objectValue(value);
      return { name: stringValue(item.name) ?? `Item ${index + 1}`, status: stringValue(item.status) ?? "loaded", source: stringValue(item.source) ?? "native" };
    }),
  };
}

function budgetView(events: MissionEvent[]): FlightViewModel["budget"] {
  const event = lastEvent(events, "budget_updated", "budget_warning");
  const raw = Array.isArray(event?.payload.dimensions) ? event.payload.dimensions : [];
  return raw.map((value) => {
    const item = objectValue(value);
    return { dimension: stringValue(item.dimension) ?? "unknown", used: numberValue(item.used), limit: numberValue(item.limit), unit: stringValue(item.unit) ?? "units", status: stringValue(item.status) ?? "normal" };
  });
}

function lastEvent(events: MissionEvent[], ...kinds: string[]): MissionEvent | undefined {
  return [...events].reverse().find((event) => kinds.includes(event.kind));
}

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function confidenceValue(value: unknown): EvidenceBatchView["confidence"] {
  return value === "verified" || value === "reported" ? value : "unknown";
}

function shortId(value: string): string {
  return value.length > 18 ? `${value.slice(0, 8)}...${value.slice(-6)}` : value;
}
