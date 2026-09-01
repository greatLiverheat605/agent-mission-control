import { Download, FileCheck2 } from "@mission-control/ui";
import { formatBytes } from "./StorageManager";

export type ExportPreviewData = {
  missionId: string;
  eventCount: number;
  sizeBytes: number;
  contentHash: string;
  categories: string[];
  containsRawProviderPayload: boolean;
};

export function ExportPreview({ preview, onExport }: { preview: ExportPreviewData | null; onExport?: () => void }) {
  return <section className="orbit-panel export-preview" aria-labelledby="export-preview-title">
    <header className="panel-heading"><span className="panel-kicker">Redacted export</span><h2 id="export-preview-title"><FileCheck2 aria-hidden="true" size={16} /> Export preview</h2></header>
    {!preview ? <p className="panel-empty">Preview unavailable until the Supervisor responds.</p> : <>
      <dl className="export-preview__readouts">
        <div><dt>Events</dt><dd>{preview.eventCount}</dd></div>
        <div><dt>Size</dt><dd>{formatBytes(preview.sizeBytes)}</dd></div>
        <div><dt>Hash</dt><dd title={preview.contentHash}>{preview.contentHash.slice(0, 16)}</dd></div>
      </dl>
      <p className="export-preview__status"><strong>{preview.containsRawProviderPayload ? "Blocked" : "Provider payload excluded"}</strong></p>
      {preview.categories.length > 0 && <ul className="export-preview__categories">{preview.categories.map((category) => <li key={category}>{category}</li>)}</ul>}
      <button type="button" onClick={onExport} disabled={!onExport} title="Export redacted mission evidence"><Download aria-hidden="true" size={15} /> Export redacted evidence</button>
    </>}
  </section>;
}
