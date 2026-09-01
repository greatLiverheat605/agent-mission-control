import { ShieldCheck } from "@mission-control/ui";
import { formatBytes } from "../storage/StorageManager";

export type DiagnosticPreviewData = {
  missionId: string;
  eventCount: number;
  exportHash: string;
  redactionCategories: string[];
  telemetryEnabled: boolean;
  includesSource: boolean;
  includesProviderPayload: boolean;
  ledger?: { lastCommittedSequence: number; recoveryRequired: boolean };
};

export function DiagnosticPreview({ preview }: { preview: DiagnosticPreviewData | null }) {
  return <section className="orbit-panel diagnostic-preview" aria-labelledby="diagnostic-preview-title">
    <header className="panel-heading"><span className="panel-kicker">Local evidence</span><h2 id="diagnostic-preview-title"><ShieldCheck aria-hidden="true" size={16} /> Diagnostic preview</h2></header>
    {!preview ? <p className="panel-empty">Diagnostics unavailable until the Supervisor responds.</p> : <dl className="diagnostic-preview__readouts">
      <div><dt>Mission</dt><dd>{preview.missionId}</dd></div>
      <div><dt>Events</dt><dd>{preview.eventCount}</dd></div>
      <div><dt>Ledger sequence</dt><dd>{preview.ledger?.lastCommittedSequence ?? "-"}</dd></div>
      <div><dt>Telemetry</dt><dd>{preview.telemetryEnabled ? "On" : "Off"}</dd></div>
      <div><dt>Source</dt><dd>{preview.includesSource ? "Blocked" : "Excluded"}</dd></div>
      <div><dt>Provider payload</dt><dd>{preview.includesProviderPayload ? "Blocked" : "Excluded"}</dd></div>
      <div><dt>Export hash</dt><dd title={preview.exportHash}>{preview.exportHash.slice(0, 16)}</dd></div>
      <div><dt>Redaction fields</dt><dd>{preview.redactionCategories.length}</dd></div>
    </dl>}
  </section>;
}

export function diagnosticSizeLabel(bytes: number): string {
  return formatBytes(bytes);
}
