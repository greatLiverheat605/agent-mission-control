import type { EvidenceBatchView } from "@mission-control/mission-store";
import { EvidenceBatch } from "./EvidenceBatch";
import { useLocale } from "../../i18n/LocaleProvider";

export function EvidenceBay({ batches }: { batches: EvidenceBatchView[] }) {
  const { t } = useLocale();
  return <section className="evidence-bay" aria-labelledby="evidence-bay-title" tabIndex={-1}>
    <header className="panel-heading"><span className="panel-kicker">{t("panel.evidenceBay")}</span><h2 id="evidence-bay-title">{t("panel.verifiedActivity")}</h2></header>
    <div className="evidence-batches">
      {batches.length ? batches.map((batch) => <EvidenceBatch key={batch.id} batch={batch} />) : <p className="panel-empty">{t("panel.noEvidence")}</p>}
    </div>
  </section>;
}
