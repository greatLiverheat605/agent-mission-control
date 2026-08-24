import type { MissionEvent } from "../../../../../packages/mission-store/src";

export function EventEvidence({ events }: { events: MissionEvent[] }) {
  return <section className="evidence-panel" aria-labelledby="evidence-title"><div className="eyebrow">Evidence</div><h2 id="evidence-title">Mission event stream</h2><ol>{events.length === 0 ? <li className="muted">No events yet</li> : events.map((event) => <li key={`${event.missionId}-${event.sequence}`}><span className="event-sequence">{event.sequence}</span><span><strong>{event.kind}</strong><small>{event.source ?? "supervisor"}</small></span></li>)}</ol></section>;
}
