import type { FlightViewModel } from "@mission-control/mission-store";
import { ShieldAlert } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

export type ApprovalResolution = "approve-once" | "approve-route" | "deny";

export function ApprovalDock({ approvals, onResolve }: { approvals: FlightViewModel["pendingApprovals"]; onResolve?: (id: string, decision: ApprovalResolution) => void }) {
  const { dateTime, t } = useLocale();
  if (!approvals.length) return null;
  return <section className="approval-dock" aria-labelledby="approval-title" tabIndex={-1}>
    <header className="panel-heading"><span className="panel-kicker">{t("panel.decisionRequired")}</span><h2 id="approval-title">{t("panel.approvals")}</h2></header>
    <div className="approval-list">{approvals.map((approval) => <article className="approval-item" key={approval.id}>
      <header><ShieldAlert aria-hidden="true" size={18} /><div><strong>{approval.action}</strong><small>{approval.scope}</small></div></header>
      <p>{approval.expiresAt ? t("panel.expires", { date: formatApprovalDate(approval.expiresAt, dateTime) }) : t("panel.noExpiry")}</p>
      <div className="approval-actions">
        <button type="button" disabled={!onResolve} title={!onResolve ? t("panel.approvalUnavailable") : undefined} onClick={() => onResolve?.(approval.id, "approve-once")}>{t("panel.approveOnce")}</button>
        <button type="button" disabled={!onResolve} title={!onResolve ? t("panel.approvalUnavailable") : undefined} onClick={() => onResolve?.(approval.id, "approve-route")}>{t("panel.approveRoute")}</button>
        <button type="button" disabled={!onResolve} title={!onResolve ? t("panel.approvalUnavailable") : undefined} className="danger-command" aria-label={t("panel.denyApproval")} onClick={() => onResolve?.(approval.id, "deny")}>{t("panel.deny")}</button>
      </div>
    </article>)}</div>
  </section>;
}

function formatApprovalDate(value: string, dateTime: (value: string | number | Date, options?: Intl.DateTimeFormatOptions) => string) {
  try {
    return dateTime(value, { dateStyle: "medium", timeStyle: "short" });
  } catch {
    return value;
  }
}
