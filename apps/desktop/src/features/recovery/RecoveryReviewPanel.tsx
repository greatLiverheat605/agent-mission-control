import { Check, CircleAlert, RotateCcw, X } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

import "../continuity.css";

export type RecoveryReviewManifest = {
  missionId: string;
  routeId: string;
  schemaVersion: number;
  contractVersion: number;
  checkpointId: string;
  ledgerSequence: number;
  loadoutFingerprint: string;
  contextPackHash: string;
  pendingApprovalHash?: string | null;
  entryHash: string;
};

type RecoveryReviewPanelProps = {
  manifest: RecoveryReviewManifest;
  verified?: boolean;
  verifying?: boolean;
  onVerify?: () => void;
  onResume?: () => void;
  onDiscard?: () => void;
};

export function RecoveryReviewPanel({ manifest, verified = false, verifying = false, onVerify, onResume, onDiscard }: RecoveryReviewPanelProps) {
  const { t } = useLocale();
  return (
    <section className="orbit-panel continuity-panel recovery-review-panel" aria-labelledby="recovery-review-title">
      <header className="panel-heading">
        <span className="panel-kicker">{t("recovery.kicker")}</span>
        <h2 id="recovery-review-title">{t("recovery.reviewTitle")}</h2>
      </header>
      <div className={`continuity-status ${verified ? "is-verified" : "is-pending"}`} role="status">
        {verified ? <Check aria-hidden="true" size={16} /> : <CircleAlert aria-hidden="true" size={16} />}
        <span>{verified ? t("recovery.verified") : t("recovery.unverified")}</span>
      </div>
      <dl className="continuity-metadata continuity-metadata--manifest">
        <div><dt>{t("memory.version")}</dt><dd>{manifest.contractVersion}</dd></div>
        <div><dt>{t("recovery.schema")}</dt><dd>{manifest.schemaVersion}</dd></div>
        <div><dt>{t("recovery.route")}</dt><dd>{manifest.routeId}</dd></div>
        <div><dt>{t("recovery.checkpoint")}</dt><dd>{manifest.checkpointId}</dd></div>
        <div><dt>{t("recovery.sequence")}</dt><dd>{manifest.ledgerSequence}</dd></div>
        <div><dt>{t("recovery.loadout")}</dt><dd>{manifest.loadoutFingerprint}</dd></div>
        <div><dt>{t("recovery.context")}</dt><dd>{manifest.contextPackHash}</dd></div>
        <div><dt>{t("recovery.approval")}</dt><dd>{manifest.pendingApprovalHash ?? "-"}</dd></div>
        <div><dt>{t("recovery.entry")}</dt><dd>{manifest.entryHash}</dd></div>
      </dl>
      <div className="continuity-actions">
        <button type="button" disabled={!onVerify || verifying || verified} onClick={onVerify}>
          <RotateCcw aria-hidden="true" size={15} />{verifying ? t("recovery.verifyPending") : t("recovery.verify")}
        </button>
        <button type="button" disabled={!onResume || !verified} onClick={onResume}>
          <Check aria-hidden="true" size={15} />{t("recovery.resumeVerified")}
        </button>
        <button type="button" className="danger-command" disabled={!onDiscard} onClick={onDiscard}>
          <X aria-hidden="true" size={15} />{t("recovery.discard")}
        </button>
      </div>
    </section>
  );
}
