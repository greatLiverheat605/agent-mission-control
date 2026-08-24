import type { MissionReadModel, MissionEvent } from "../../../../../packages/mission-store/src";
import { EmergencyPause } from "./EmergencyPause";
import { EventEvidence } from "./EventEvidence";

export function BasicMissionFlight({ mission, events, onPause }: { mission: MissionReadModel; events: MissionEvent[]; onPause: () => void }) {
  return <section className="mission-flight" aria-labelledby="flight-title"><header><div><div className="eyebrow">Mission flight</div><h1 id="flight-title">{mission.phase}</h1><p className="muted">{mission.currentAction ?? "Waiting for the next supervised action"}</p></div><EmergencyPause disabled={mission.status === "paused" || mission.status === "completed"} onPause={onPause} /></header><div className="flight-grid"><div className="route-panel"><span className="metric-label">Status</span><strong>{mission.status}</strong><span className="metric-label">Sequence</span><strong>{mission.lastSequence}</strong>{mission.reason && <p className="warning">{mission.reason}</p>}</div><EventEvidence events={events} /></div></section>;
}
