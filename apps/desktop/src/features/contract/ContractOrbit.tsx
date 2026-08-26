import type { FlightViewModel } from "@mission-control/mission-store";
import { FileCheck2 } from "@mission-control/ui";
import { ContractDiff } from "./ContractDiff";
import { compactPath } from "../evidence/EvidenceBatch";
import { useLocale } from "../../i18n/LocaleProvider";

export function ContractOrbit({ flight, onEdit }: { flight: FlightViewModel; onEdit?: () => void }) {
  const { t } = useLocale();
  return <section className="orbit-panel contract-orbit" aria-labelledby="contract-title" tabIndex={-1}>
    <header className="panel-heading panel-heading--action">
      <div><span className="panel-kicker">{t("panel.contractOrbit")}</span><h2 id="contract-title">{t("panel.missionContract")}</h2></div>
      <button type="button" className="icon-command" aria-label={t("panel.editContract")} title={t("panel.editContract")} onClick={onEdit}><FileCheck2 aria-hidden="true" size={18} /></button>
    </header>
    <dl className="contract-facts">
      <div><dt>{t("panel.goal")}</dt><dd>{flight.contract.goal}</dd></div>
      <div><dt>{t("panel.mode")}</dt><dd>{flight.contract.drivingMode}</dd></div>
      <div><dt>{t("panel.version")}</dt><dd>v{flight.contract.version}</dd></div>
    </dl>
    <div className="action-brief">
      <span className="panel-kicker">{t("panel.currentAction")}</span>
      <strong>{flight.currentAction.summary}</strong>
      <p>{flight.currentAction.explanation}</p>
      <span className="panel-kicker">{t("panel.impact")}</span>
      {flight.currentAction.impact.length ? <ul>{flight.currentAction.impact.map((path) => <li key={path} title={path}>{compactPath(path)}</li>)}</ul> : <p>{t("panel.noImpact")}</p>}
      <span className="panel-kicker">{t("panel.nextDecision")}</span>
      <strong>{flight.currentAction.nextDecision}</strong>
    </div>
    <ContractDiff version={flight.contract.version} />
  </section>;
}
