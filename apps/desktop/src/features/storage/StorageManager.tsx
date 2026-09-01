import { useState } from "react";
import { Archive, Box, Database, Download, Trash2 } from "@mission-control/ui";

export type StorageSnapshot = {
  missionId: string;
  usedBytes: number;
  eventCount: number;
  archived: boolean;
  budgetBytes?: number | null;
};

export type StorageImpact = {
  impactHash: string;
  projectedBytes: number;
  affectedEvents: number;
  affectedBlobs: number;
  automaticDeletion: boolean;
  blobs?: Array<{ hash: string; size: number; willRemove: boolean }>;
  plan?: Record<string, unknown>;
};

type StorageManagerProps = {
  snapshot: StorageSnapshot;
  impact?: StorageImpact | null;
  onArchive?: () => void;
  onDelete?: () => void;
  onExport?: () => void;
};

export function StorageManager({ snapshot, impact, onArchive, onDelete, onExport }: StorageManagerProps) {
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const budget = snapshot.budgetBytes ?? null;
  const usagePercent = budget ? Math.min(100, Math.round(snapshot.usedBytes / budget * 100)) : null;
  return <section className="orbit-panel storage-manager" aria-labelledby="storage-manager-title">
    <header className="panel-heading">
      <span className="panel-kicker">Mission storage</span>
      <h2 id="storage-manager-title"><Database aria-hidden="true" size={16} /> Storage manager</h2>
      <span className="panel-meta">{snapshot.archived ? "ARCHIVED" : "ACTIVE"}</span>
    </header>
    <dl className="storage-manager__readouts">
      <div><dt>Mission</dt><dd title={snapshot.missionId}>{snapshot.missionId}</dd></div>
      <div><dt>Events</dt><dd>{snapshot.eventCount}</dd></div>
      <div><dt>Usage</dt><dd>{formatBytes(snapshot.usedBytes)}{usagePercent == null ? "" : ` · ${usagePercent}%`}</dd></div>
    </dl>
    {impact && <div className="storage-manager__impact" data-impact-hash={impact.impactHash}>
      <span><Box aria-hidden="true" size={14} /> Preview impact</span>
      <strong>{formatBytes(impact.projectedBytes)}</strong>
      <small>{impact.affectedEvents} events · {impact.affectedBlobs} blobs · hash {impact.impactHash.slice(0, 12)}</small>
      {!impact.automaticDeletion && <small className="storage-manager__warning">Automatic deletion disabled</small>}
    </div>}
    <div className="storage-manager__actions">
      <button type="button" onClick={onArchive} disabled={!onArchive || snapshot.archived} title="Archive this mission">
        <Archive aria-hidden="true" size={15} /> Archive
      </button>
      <button type="button" onClick={() => setConfirmingDelete(true)} disabled={!onDelete || !impact} title="Delete this mission">
        <Trash2 aria-hidden="true" size={15} /> Delete
      </button>
      {onExport && <button type="button" onClick={onExport} title="Export redacted mission evidence">
        <Download aria-hidden="true" size={15} /> Export
      </button>}
    </div>
    {confirmingDelete && impact && <div className="storage-manager__confirm" role="alertdialog" aria-label="Confirm mission deletion">
      <strong>Delete this mission?</strong>
      <p>{impact.affectedEvents} events ({formatBytes(impact.projectedBytes)}) and {impact.affectedBlobs} blob references are in this plan.</p>
      {impact.blobs?.filter((blob) => blob.willRemove).map((blob) => <small key={blob.hash}>Blob {blob.hash.slice(0, 12)} will be removed</small>)}
      <div className="storage-manager__actions">
        <button type="button" onClick={() => setConfirmingDelete(false)}>Cancel</button>
        <button type="button" onClick={() => { setConfirmingDelete(false); onDelete?.(); }}><Trash2 aria-hidden="true" size={15} /> Confirm delete</button>
      </div>
    </div>}
  </section>;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = "B";
  for (const next of units) {
    value /= 1024;
    unit = next;
    if (value < 1024) break;
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${unit}`;
}
