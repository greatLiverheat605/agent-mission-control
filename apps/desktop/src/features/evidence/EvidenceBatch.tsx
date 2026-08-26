import type { EvidenceBatchView } from "@mission-control/mission-store";
import { CircleCheck, Copy } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

export function EvidenceBatch({ batch }: { batch: EvidenceBatchView }) {
  const { number, t } = useLocale();
  return <details className="evidence-batch">
    <summary>
      <CircleCheck aria-hidden="true" size={18} />
      <span><strong>{batch.summary}</strong><small>{batch.source} · {batch.confidence} · {t("panel.eventCount", { count: number(batch.count) })}</small></span>
    </summary>
    <div className="evidence-batch__detail">
      <p>{t("panel.sequences", { sequences: batch.sequences.map((sequence) => number(sequence)).join(", ") })}</p>
      {batch.files.map((path) => <div className="path-row" key={path}>
        <span className="panel-path" title={path}>{compactPath(path)}</span>
        <button type="button" className="icon-command" aria-label={t("panel.copyPathAria", { path })} title={t("panel.copyPath")} onClick={() => { void navigator.clipboard?.writeText(path); }}><Copy aria-hidden="true" size={16} /></button>
      </div>)}
    </div>
  </details>;
}

export function compactPath(path: string, maxLength = 42): string {
  if (path.length <= maxLength) return path;
  const keep = Math.floor((maxLength - 3) / 2);
  return `${path.slice(0, keep)}...${path.slice(-keep)}`;
}
