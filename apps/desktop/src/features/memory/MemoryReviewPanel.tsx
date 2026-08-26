import { Check, CircleAlert, X } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

import "../continuity.css";

export type MemoryReviewStatus = "pending" | "confirmed" | "rejected" | "deferred";
export type MemoryDecision = "confirm" | "reject" | "defer";

export type MemoryReviewItem = {
  id: string;
  kind: string;
  content: string;
  sourceEventIds: string[];
  scope: string;
  freshness: string;
  version: number;
  status: MemoryReviewStatus;
  author: string;
};

type MemoryReviewPanelProps = {
  items: MemoryReviewItem[];
  onDecision?: (id: string, decision: MemoryDecision) => void;
};

export function MemoryReviewPanel({ items, onDecision }: MemoryReviewPanelProps) {
  const { t } = useLocale();
  return (
    <section className="orbit-panel continuity-panel memory-review-panel" aria-labelledby="memory-review-title">
      <header className="panel-heading">
        <span className="panel-kicker">{t("memory.kicker")}</span>
        <h2 id="memory-review-title">{t("memory.title")}</h2>
      </header>
      {items.length ? (
        <div className="continuity-list">
          {items.map((item) => (
            <article className="continuity-item memory-review-item" aria-label={item.id} key={item.id}>
              <header className="continuity-item-heading">
                {item.status === "pending" ? <CircleAlert aria-hidden="true" size={17} /> : <Check aria-hidden="true" size={17} />}
                <div>
                  <strong>{item.content}</strong>
                  <small>{item.kind} · {t("memory.status")}: {item.status}</small>
                </div>
              </header>
              <dl className="continuity-metadata">
                <div><dt>{t("memory.sourceEvents")}</dt><dd>{item.sourceEventIds.join(", ")}</dd></div>
                <div><dt>{t("memory.scope")}</dt><dd>{item.scope}</dd></div>
                <div><dt>{t("memory.freshness")}</dt><dd>{item.freshness}</dd></div>
                <div><dt>{t("memory.version")}</dt><dd>{item.version}</dd></div>
                <div><dt>{t("memory.author")}</dt><dd>{item.author}</dd></div>
              </dl>
              <div className="continuity-actions" aria-label={`${item.id} decisions`}>
                <button type="button" disabled={!onDecision || item.status !== "pending"} onClick={() => onDecision?.(item.id, "confirm")}>
                  <Check aria-hidden="true" size={15} />{t("memory.confirm", { id: item.id })}
                </button>
                <button type="button" disabled={!onDecision || item.status !== "pending"} onClick={() => onDecision?.(item.id, "defer")}>
                  {t("memory.defer", { id: item.id })}
                </button>
                <button type="button" className="danger-command" disabled={!onDecision || item.status !== "pending"} onClick={() => onDecision?.(item.id, "reject")}>
                  <X aria-hidden="true" size={15} />{t("memory.reject", { id: item.id })}
                </button>
              </div>
            </article>
          ))}
        </div>
      ) : <p className="panel-empty">{t("memory.empty")}</p>}
    </section>
  );
}
