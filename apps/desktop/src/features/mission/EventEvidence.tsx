import type { MissionEvent } from "../../../../../packages/mission-store/src";
import { useLocale } from "../../i18n/LocaleProvider";

export function EventEvidence({ events }: { events: MissionEvent[] }) {
  const { number, t } = useLocale();
  return <section className="evidence-panel" aria-labelledby="evidence-title"><div className="eyebrow">{t("events.kicker")}</div><h2 id="evidence-title">{t("events.title")}</h2><ol tabIndex={0}>{events.length === 0 ? <li className="muted">{t("events.empty")}</li> : events.map((event) => <li key={`${event.missionId}-${event.sequence}`}><span className="event-sequence">{number(event.sequence)}</span><span><strong>{event.kind}</strong><small>{event.source ?? "supervisor"}</small></span></li>)}</ol></section>;
}
