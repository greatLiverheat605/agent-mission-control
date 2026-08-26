import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { BasicMissionFlight } from "./features/mission/BasicMissionFlight";
import { NewMission, type MissionDraft } from "./features/onboarding/NewMission";
import { emptyMission, reduceMission, type MissionEvent, type MissionReadModel } from "../../../packages/mission-store/src";
import { useLocale } from "./i18n/LocaleProvider";

export type SupervisorStatus = {
  connection: "connected" | "disconnected";
  version: string | null;
  errorCode?: string;
};

const supervisorApi = {
  status: () => invoke<SupervisorStatus>("supervisor_status"),
  ping: () => invoke<SupervisorStatus>("ping_supervisor"),
  create: (request: MissionCommandRequest) => invoke<MissionCommandResult>("create_mission", { request }),
  launch: (request: MissionCommandRequest) => invoke<MissionCommandResult>("launch_route", { request }),
  subscribe: (request: MissionCommandRequest) => invoke<MissionCommandResult>("subscribe_mission", { request }),
  pause: (request: MissionCommandRequest) => invoke<MissionCommandResult>("request_safe_pause", { request }),
};

type MissionCommandRequest = {
  missionId?: string;
  routeId?: string;
  expectedVersion?: number;
  projectRoot?: string;
  goal?: string;
  reason?: string;
};

type MissionCommandResult = {
  accepted: boolean;
  missionId: string | null;
  routeId: string | null;
  sequence: number | null;
  errorCode?: string | null;
  events: Array<Record<string, unknown>>;
};

type ActiveMission = {
  draft: MissionDraft;
  missionId: string;
  routeId: string;
  lastSequence: number;
  phase: string;
  status: MissionReadModel["status"];
  currentAction: string | null;
  reason: string | null;
};

const ACTIVE_MISSION_KEY = "mission-control.active-mission.v1";

export default function App() {
  const { t } = useLocale();
  const restored = useRef(loadActiveMission()).current;
  const [status, setStatus] = useState<SupervisorStatus | null>(null);
  const [draft, setDraft] = useState<MissionDraft | null>(restored?.draft ?? null);
  const [missionId, setMissionId] = useState<string | null>(restored?.missionId ?? null);
  const [routeId, setRouteId] = useState<string | null>(restored?.routeId ?? null);
  const [mission, setMission] = useState(() => ({
    ...emptyMission(restored?.missionId ?? "local-mission"),
    lastSequence: restored?.lastSequence ?? 0,
    phase: restored?.phase ?? "Ready",
    status: restored?.status ?? "idle",
    currentAction: restored?.currentAction ?? null,
    reason: restored?.reason ?? null,
  }));
  const pendingRequest = useRef<Promise<SupervisorStatus> | null>(null);
  const disconnectPauseSent = useRef(false);
  const lastSequence = useRef(restored?.lastSequence ?? 0);

  useEffect(() => {
    let active = true;
    let observed: Promise<SupervisorStatus> | null = null;
    const update = (request: () => Promise<SupervisorStatus>) => {
      const pending = pendingRequest.current ?? request();
      pendingRequest.current = pending;
      if (observed === pending) return;
      observed = pending;
      void pending
        .then((nextStatus) => {
          if (active) setStatus(nextStatus);
        })
        .catch(() => {
          if (active) {
            setStatus({ connection: "disconnected", version: null });
          }
        })
        .finally(() => {
          if (pendingRequest.current === pending) pendingRequest.current = null;
          if (observed === pending) observed = null;
        });
    };

    update(supervisorApi.status);
    const heartbeat = window.setInterval(() => {
      update(supervisorApi.ping);
    }, 1_000);

    return () => {
      active = false;
      window.clearInterval(heartbeat);
    };
  }, []);

  useEffect(() => {
    if (status?.connection !== "disconnected" || !missionId || mission.status !== "running" || disconnectPauseSent.current) return;
    disconnectPauseSent.current = true;
    void supervisorApi.pause({ missionId, routeId: routeId ?? undefined, reason: "UI disconnected; request safe pause" })
      .then((result) => {
        const events = toMissionEvents(result.events);
        const sequence = result.sequence ?? currentSequence(events);
        lastSequence.current = sequence;
        setMission((current) => ({
          ...current,
          phase: "Paused",
          status: "paused",
          currentAction: "Safe pause requested",
          reason: "UI disconnected; request safe pause",
          lastSequence: sequence,
          events: mergeMissionEvents(current.events, events),
        }));
      })
      .catch(() => undefined);
  }, [mission.status, missionId, routeId, status?.connection]);

  useEffect(() => {
    if (status?.connection !== "connected" || !missionId) return;
    let active = true;
    let pending = false;
    const syncMission = () => {
      if (!active || pending) return;
      pending = true;
      void supervisorApi.subscribe({ missionId, expectedVersion: lastSequence.current })
        .then((result) => {
          if (!active) return;
          setMission((current) => {
            const incoming = toMissionEvents(result.events);
            const projected = projectMissionEvents(current, incoming);
            const sequence = Math.max(projected.lastSequence, result.sequence ?? 0);
            lastSequence.current = sequence;
            const next = { ...projected, lastSequence: sequence };
            saveActiveMission(draft && routeId ? activeMission(draft, missionId, routeId, next) : null);
            return next;
          });
        })
        .catch(() => undefined)
        .finally(() => {
          pending = false;
        });
    };
    syncMission();
    const poll = window.setInterval(syncMission, 1_000);
    return () => {
      active = false;
      window.clearInterval(poll);
    };
  }, [draft, missionId, routeId, status?.connection]);

  const createMission = async (nextDraft: MissionDraft) => {
    setDraft(nextDraft);
    const created = await supervisorApi.create({ projectRoot: nextDraft.projectRoot, goal: nextDraft.goal });
    if (!created.accepted || !created.missionId || !created.routeId) throw new Error(created.errorCode ?? "MISSION_CREATE_FAILED");
    disconnectPauseSent.current = false;
    setMissionId(created.missionId);
    setRouteId(created.routeId);
    lastSequence.current = created.sequence ?? 0;
    const createdMission = { ...emptyMission(created.missionId), phase: "Contract review", status: "idle" as const, currentAction: "Read-only contract created", lastSequence: created.sequence ?? 0, events: toMissionEvents(created.events) };
    saveActiveMission(activeMission(nextDraft, created.missionId, created.routeId, createdMission));
    setMission(createdMission);
    const launched = await supervisorApi.launch({ missionId: created.missionId, routeId: created.routeId, projectRoot: nextDraft.projectRoot });
    const launchEvents = toMissionEvents(launched.events);
    setMission((current) => {
      const projected = projectMissionEvents(current, launchEvents);
      const launchSequence = Math.max(projected.lastSequence, launched.sequence ?? 0);
      const next = { ...projected, lastSequence: launchSequence };
      lastSequence.current = launchSequence;
      saveActiveMission(activeMission(nextDraft, created.missionId!, created.routeId!, next));
      return next;
    });
  };

  const discardRecovery = () => {
    saveActiveMission(null);
    setDraft(null);
    setMissionId(null);
    setRouteId(null);
    lastSequence.current = 0;
    setMission(emptyMission("local-mission"));
  };

  const reconnectRecovery = () => {
    setMission((current) => ({ ...current, currentAction: "Reconnected to paused mission" }));
  };

  const pauseMission = async () => {
    if (!missionId) return;
    const result = await supervisorApi.pause({ missionId, reason: "user requested safe pause" });
    const events = toMissionEvents(result.events);
    const sequence = result.sequence ?? currentSequence(events);
    lastSequence.current = sequence;
    setMission((current) => ({ ...current, phase: "Paused", status: "paused", currentAction: "Safe pause requested", reason: "user requested safe pause", lastSequence: sequence, events: mergeMissionEvents(current.events, events) }));
  };

  let message = t("app.connecting");
  if (status?.connection === "connected") {
    message = status.version
      ? t("app.connectedVersion", { version: status.version })
      : t("app.connected");
  } else if (status?.connection === "disconnected") {
    message = t("app.disconnected");
  }
  const state = status?.connection ?? "connecting";
  const hasMission = Boolean(draft && missionId);
  const displayOverride = state === "connected"
    ? hasMission ? undefined : <NewMission embedded onCreate={(nextDraft) => { void createMission(nextDraft); }} />
    : <ConnectionDisplay message={message} state={state} />;

  return <>
    <div className="supervisor-status sr-only" role="status" aria-live="polite">{message}</div>
    <div className="mission-workspace"><BasicMissionFlight
      mission={mission}
      events={mission.events}
      connectionState={state}
      displayOverride={displayOverride}
      onPause={state === "connected" && missionId ? () => { void pauseMission(); } : undefined}
      onReconnect={reconnectRecovery}
      onDiscard={discardRecovery}
    /></div>
  </>;
}

function ConnectionDisplay({ message, state }: { message: string; state: "connecting" | "disconnected" }) {
  const { t } = useLocale();
  return <section className="cockpit-standby" data-connection={state} aria-label={message}>
    <div className="cockpit-standby__scope" aria-hidden="true"><i /><i /><span /></div>
    <span className="panel-kicker">{t("beam.control")}</span>
    <h1>{message}</h1>
    <p>{state === "connecting" ? t("new.uplinkReady") : t("systems.recovery")}</p>
  </section>;
}

function toMissionEvents(events: Array<Record<string, unknown>>): MissionEvent[] {
  return events.map((event) => ({
    missionId: String(event.mission_id ?? ""),
    sequence: Number(event.sequence ?? 0),
    kind: String(event.kind ?? "unknown"),
    payload: (event.payload as Record<string, unknown> | undefined) ?? {},
    source: (event.source as MissionEvent["source"]) ?? "supervisor",
  }));
}

function mergeMissionEvents(current: MissionEvent[], incoming: MissionEvent[]): MissionEvent[] {
  const bySequence = new Map(current.map((event) => [event.sequence, event]));
  for (const event of incoming) bySequence.set(event.sequence, event);
  return [...bySequence.values()].sort((left, right) => left.sequence - right.sequence);
}

function currentSequence(events: MissionEvent[]): number {
  return events.reduce((sequence, event) => Math.max(sequence, event.sequence), 0);
}

function projectMissionEvents(current: MissionReadModel, incoming: MissionEvent[]): MissionReadModel {
  let state = { [current.missionId]: current };
  for (const event of incoming.sort((left, right) => left.sequence - right.sequence)) {
    state = reduceMission(state, { type: "event", event });
  }
  return state[current.missionId] ?? current;
}

function loadActiveMission(): ActiveMission | null {
  try {
    const raw = window.localStorage.getItem(ACTIVE_MISSION_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<ActiveMission>;
    if (!value.missionId || !value.routeId || !value.draft || typeof value.lastSequence !== "number") return null;
    const hasProjection = typeof value.phase === "string"
      && ["idle", "running", "paused", "completed", "failed"].includes(value.status ?? "")
      && (typeof value.currentAction === "string" || value.currentAction === null)
      && (typeof value.reason === "string" || value.reason === null);
    if (hasProjection) return value as ActiveMission;
    return { draft: value.draft, missionId: value.missionId, routeId: value.routeId, lastSequence: 0, phase: "Ready", status: "idle", currentAction: null, reason: null };
  } catch {
    return null;
  }
}

function saveActiveMission(active: ActiveMission | null): void {
  if (active) window.localStorage.setItem(ACTIVE_MISSION_KEY, JSON.stringify(active));
  else window.localStorage.removeItem(ACTIVE_MISSION_KEY);
}

function activeMission(draft: MissionDraft, missionId: string, routeId: string, mission: MissionReadModel): ActiveMission {
  const { lastSequence, phase, status, currentAction, reason } = mission;
  return { draft, missionId, routeId, lastSequence, phase, status, currentAction, reason };
}
