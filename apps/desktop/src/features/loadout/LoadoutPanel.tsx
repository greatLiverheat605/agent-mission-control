import type { FlightViewModel } from "@mission-control/mission-store";
import { Box } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

export function LoadoutPanel({ loadout }: { loadout: FlightViewModel["loadout"] }) {
  const { t } = useLocale();
  return <section className="orbit-panel loadout-panel" aria-labelledby="loadout-title">
    <header className="panel-heading"><span className="panel-kicker">{t("panel.loadout")}</span><h2 id="loadout-title">{loadout.provider} · {loadout.model}</h2></header>
    {loadout.change && <div className="loadout-change" role="status"><strong>{t("panel.loadoutChanged")}</strong><small>{loadout.change.previous} -&gt; {loadout.change.next}</small><span>{t("panel.loadoutPaused")}</span></div>}
    {loadout.items.length ? <ul>{loadout.items.map((item) => <li key={`${item.name}-${item.source}`}><Box aria-hidden="true" size={15} /><span><strong>{item.name}</strong><small>{item.status} · {item.source}</small></span></li>)}</ul> : <p className="panel-empty">{t("panel.noLoadout")}</p>}
  </section>;
}
