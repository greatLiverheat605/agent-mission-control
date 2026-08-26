export type MissionEvent = {
  missionId: string;
  sequence: number;
  kind: string;
  payload: Record<string, unknown>;
  source?: "supervisor" | "agent" | "user" | "system";
  requiresSafePause?: boolean;
};

export type MissionReadModel = {
  missionId: string;
  lastSequence: number;
  phase: string;
  status: "idle" | "running" | "paused" | "completed" | "failed";
  currentAction: string | null;
  reason: string | null;
  events: MissionEvent[];
  needsResync: boolean;
};

export type MissionStoreState = Record<string, MissionReadModel>;

export type MissionAction =
  | { type: "snapshot"; missionId: string; sequence: number; events: MissionEvent[]; phase?: string; status?: MissionReadModel["status"]; currentAction?: string | null; reason?: string | null }
  | { type: "event"; event: MissionEvent }
  | { type: "resync"; missionId: string; sequence: number; events: MissionEvent[] };

export function emptyMission(missionId: string): MissionReadModel {
  return { missionId, lastSequence: 0, phase: "Ready", status: "idle", currentAction: null, reason: null, events: [], needsResync: false };
}

export function reduceMission(state: MissionStoreState, action: MissionAction): MissionStoreState {
  if (action.type === "snapshot") {
    const prior = state[action.missionId] ?? emptyMission(action.missionId);
    const next = { ...prior, lastSequence: action.sequence, events: action.events.slice(), needsResync: false };
    if (action.phase !== undefined) next.phase = action.phase;
    if (action.status !== undefined) next.status = action.status;
    if (action.currentAction !== undefined) next.currentAction = action.currentAction;
    if (action.reason !== undefined) next.reason = action.reason;
    return { ...state, [action.missionId]: next };
  }
  if (action.type === "resync") return reduceMission(state, { type: "snapshot", missionId: action.missionId, sequence: action.sequence, events: action.events });
  const event = action.event;
  const prior = state[event.missionId] ?? emptyMission(event.missionId);
  if (event.sequence <= prior.lastSequence) return state;
  if (event.sequence !== prior.lastSequence + 1) return { ...state, [event.missionId]: { ...prior, needsResync: true } };
  const next = projectEvent({ ...prior, lastSequence: event.sequence, events: [...prior.events, event].slice(-200) }, event);
  return { ...state, [event.missionId]: next };
}

function projectEvent(model: MissionReadModel, event: MissionEvent): MissionReadModel {
  const payload = event.payload;
  if (event.kind === "exploration_started" || event.kind === "agent_run_started") return { ...model, phase: "Exploring", status: "running", currentAction: "Inspecting project", reason: null };
  if (event.kind === "pause_requested") return { ...model, phase: "Paused", status: "paused", currentAction: "Safe pause requested", reason: model.status === "paused" && model.reason ? model.reason : typeof payload.reason === "string" ? payload.reason : "Pause requested" };
  if (event.kind === "route_state_changed" && payload.state === "completed") return { ...model, phase: "Completed", status: "completed", currentAction: null };
  if (event.kind === "error" || event.kind === "adapter.protocol_error") return { ...model, status: "failed", currentAction: null, reason: typeof payload.error === "string" ? payload.error : "Adapter error" };
  return model;
}
