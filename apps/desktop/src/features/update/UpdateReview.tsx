import { Check, CircleAlert, PackageCheck, RotateCcw, ShieldAlert } from "@mission-control/ui";

export type UpdateReviewManifest = {
  channel: string;
  version: string;
  artifactSha256: string;
  signerFingerprint: string;
  schemaVersion: number;
  minSchemaVersion: number;
};

type UpdateReviewProps = {
  manifest: UpdateReviewManifest | null;
  verified?: boolean;
  activeMission?: boolean;
  onVerify?: () => void;
  onInstall?: () => void;
  onRollback?: () => void;
};

export function UpdateReview({ manifest, verified = false, activeMission = false, onVerify, onInstall, onRollback }: UpdateReviewProps) {
  return <section className="orbit-panel continuity-panel update-review" aria-labelledby="update-review-title">
    <header className="panel-heading">
      <span className="panel-kicker">Signed preview</span>
      <h2 id="update-review-title"><PackageCheck aria-hidden="true" size={16} /> Update review</h2>
    </header>
    {!manifest ? <p className="panel-empty">No update manifest is available.</p> : <>
      <div className={`continuity-status ${verified ? "is-verified" : "is-pending"}`} role="status">
        {verified ? <Check aria-hidden="true" size={16} /> : <CircleAlert aria-hidden="true" size={16} />}
        <span>{verified ? "Signature and artifact verified" : "Signature verification required"}</span>
      </div>
      <dl className="continuity-metadata">
        <div><dt>Channel</dt><dd>{manifest.channel}</dd></div>
        <div><dt>Version</dt><dd>{manifest.version}</dd></div>
        <div><dt>Artifact SHA-256</dt><dd title={manifest.artifactSha256}>{manifest.artifactSha256.slice(0, 16)}</dd></div>
        <div><dt>Signer</dt><dd title={manifest.signerFingerprint}>{manifest.signerFingerprint}</dd></div>
        <div><dt>Schema</dt><dd>{manifest.schemaVersion} (min {manifest.minSchemaVersion})</dd></div>
      </dl>
      {activeMission && <p className="update-review__warning"><ShieldAlert aria-hidden="true" size={15} /> Active Mission must be paused before updating.</p>}
      <div className="continuity-actions">
        <button type="button" disabled={!onVerify || verified} onClick={onVerify}><RotateCcw aria-hidden="true" size={15} /> Verify</button>
        <button type="button" disabled={!onInstall || !verified || activeMission} onClick={onInstall}><PackageCheck aria-hidden="true" size={15} /> Install after backup</button>
        {onRollback && <button type="button" onClick={onRollback}><RotateCcw aria-hidden="true" size={15} /> Roll back</button>}
      </div>
    </>}
  </section>;
}
