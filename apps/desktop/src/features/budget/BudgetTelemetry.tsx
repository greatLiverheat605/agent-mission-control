import type { FlightViewModel } from "@mission-control/mission-store";
import { Gauge } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

export function BudgetTelemetry({ budget }: { budget: FlightViewModel["budget"] }) {
  const { number, t } = useLocale();
  return <section className="orbit-panel budget-panel" aria-labelledby="budget-title">
    <header className="panel-heading"><span className="panel-kicker">{t("panel.flightEnvelope")}</span><h2 id="budget-title">{t("panel.budget")}</h2></header>
    {budget.length ? <ul>{budget.map((item) => {
      const ratio = item.used !== null && item.limit ? Math.min(1, item.used / item.limit) : 0;
      return <li key={item.dimension}>
        <div><Gauge aria-hidden="true" size={15} /><strong>{item.dimension}</strong><span>{item.used === null ? "?" : number(item.used)} / {item.limit === null ? "?" : number(item.limit)} {item.unit}</span></div>
        <meter min={0} max={1} value={ratio} aria-label={t("panel.budgetUsage", { dimension: item.dimension })} />
        <small>{number(ratio, { style: "percent", maximumFractionDigits: 0 })} · {item.status}</small>
      </li>;
    })}</ul> : <p className="panel-empty">{t("panel.budgetUnavailable")}</p>}
  </section>;
}
