import { FileCheck2 } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

import "../continuity.css";

export type RecallEvidence = {
  id: string;
  content: string;
  sourceEventIds: string[];
  scope: string;
  freshness: string;
  version: number;
};

export function RecallInspector({ evidence }: { evidence: RecallEvidence[] }) {
  const { t } = useLocale();
  return (
    <section className="orbit-panel continuity-panel recall-inspector" aria-labelledby="recall-inspector-title">
      <header className="panel-heading">
        <span className="panel-kicker">{t("recall.kicker")}</span>
        <h2 id="recall-inspector-title">{t("recall.title")}</h2>
      </header>
      <p className="continuity-notice"><FileCheck2 aria-hidden="true" size={16} />{t("recall.noHiddenReasoning")}</p>
      {evidence.length ? (
        <div className="continuity-list">
          {evidence.map((item) => (
            <article className="continuity-item recall-item" aria-label={item.id} key={`${item.id}-${item.version}`}>
              <header className="continuity-item-heading"><FileCheck2 aria-hidden="true" size={17} /><div><strong>{item.content}</strong><small>{t("recall.evidence")}</small></div></header>
              <dl className="continuity-metadata">
                <div><dt>{t("memory.sourceEvents")}</dt><dd>{item.sourceEventIds.join(", ")}</dd></div>
                <div><dt>{t("memory.scope")}</dt><dd>{item.scope}</dd></div>
                <div><dt>{t("memory.freshness")}</dt><dd>{item.freshness}</dd></div>
                <div><dt>{t("memory.version")}</dt><dd>{item.version}</dd></div>
              </dl>
            </article>
          ))}
        </div>
      ) : <p className="panel-empty">{t("recall.empty")}</p>}
    </section>
  );
}
