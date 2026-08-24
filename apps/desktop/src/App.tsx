import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { BasicMissionFlight } from "./features/mission/BasicMissionFlight";
import { NewMission, type MissionDraft } from "./features/onboarding/NewMission";
import { emptyMission, type MissionEvent } from "../../../packages/mission-store/src";

export type SupervisorStatus = {
  connection: "connected" | "disconnected";
  version: string | null;
  errorCode?: string;
};

const supervisorApi = {
  status: () => invoke<SupervisorStatus>("supervisor_status"),
  ping: () => invoke<SupervisorStatus>("ping_supervisor"),
};

export default function App() {
  const [status, setStatus] = useState<SupervisorStatus | null>(null);
  const [draft, setDraft] = useState<MissionDraft | null>(null);
  const mission = useMemo(() => draft ? { ...emptyMission("local-mission"), phase: "Contract review", status: "idle" as const, currentAction: "Confirm the read-only contract" } : null, [draft]);
  const pendingRequest = useRef<Promise<SupervisorStatus> | null>(null);

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

  let message = "Connecting to Local Supervisor";
  if (status?.connection === "connected") {
    message = status.version
      ? `Connected to Local Supervisor - ${status.version}`
      : "Connected to Local Supervisor";
  } else if (status?.connection === "disconnected") {
    message = "Local Supervisor Disconnected";
  }
  const state = status?.connection ?? "connecting";

  return <>
    <main className={`supervisor-shell state-${state}`} role="status" aria-live="polite" tabIndex={0}>
      <div className="signal-track" aria-hidden="true"><span /></div><span>{message}</span>
    </main>
    {status?.connection === "connected" && <div className="mission-workspace">{mission ? <BasicMissionFlight mission={mission} events={[] as MissionEvent[]} onPause={() => undefined} /> : <NewMission onCreate={setDraft} />}</div>}
  </>;
}
