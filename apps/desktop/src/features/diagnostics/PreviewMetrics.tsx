import { useState } from "react";
import { Check, Download, Gauge, ShieldAlert } from "@mission-control/ui";

export type PreviewParticipant = {
  id: string;
  projectType: "typescript" | "rust" | "mixed";
  hardwareProfile: string;
  firstLaunchCompleted: boolean;
  stateRecognitionCorrect: boolean;
  stateRecognitionSeconds: number;
  guided: boolean;
  issueSeverity?: "P0" | "P1" | "P2" | null;
  failureReason?: string | null;
};

export type PreviewMetricsData = {
  participants: PreviewParticipant[];
  telemetryEnabled: boolean;
};

export type PreviewMetricSummary = {
  participants: number;
  firstLaunchRate: number;
  stateRecognitionRate: number;
  p0p1Count: number;
  expansionGate: boolean;
};

export function summarizePreviewMetrics(participants: PreviewParticipant[]): PreviewMetricSummary {
  const count = participants.length;
  const firstLaunchRate = count === 0 ? 0 : Math.round(participants.filter((participant) => participant.firstLaunchCompleted && !participant.guided).length / count * 100);
  const stateRecognitionRate = count === 0 ? 0 : Math.round(participants.filter((participant) => participant.stateRecognitionCorrect && participant.stateRecognitionSeconds <= 10).length / count * 100);
  const p0p1Count = participants.filter((participant) => participant.issueSeverity === "P0" || participant.issueSeverity === "P1").length;
  return { participants: count, firstLaunchRate, stateRecognitionRate, p0p1Count, expansionGate: firstLaunchRate >= 80 && stateRecognitionRate >= 90 && p0p1Count === 0 };
}

export function PreviewMetrics({ data, onTelemetryChange, onExport }: { data: PreviewMetricsData; onTelemetryChange?: (enabled: boolean) => void; onExport?: () => void }) {
  const summary = summarizePreviewMetrics(data.participants);
  const [reviewed, setReviewed] = useState(false);
  return <section className="orbit-panel preview-metrics" aria-labelledby="preview-metrics-title">
    <header className="panel-heading"><span className="panel-kicker">Local pilot</span><h2 id="preview-metrics-title"><Gauge aria-hidden="true" size={16} /> Preview metrics</h2></header>
    <div className="preview-metrics__gate" data-passed={summary.expansionGate} role="status">
      {summary.expansionGate ? <Check aria-hidden="true" size={15} /> : <ShieldAlert aria-hidden="true" size={15} />}
      <strong>{summary.expansionGate ? "Expansion gate passed" : "Internal pilot gate"}</strong>
    </div>
    <dl className="preview-metrics__readouts">
      <div><dt>Participants</dt><dd>{summary.participants}</dd></div>
      <div><dt>First launch</dt><dd>{summary.firstLaunchRate}% <small>(≥ 80%)</small></dd></div>
      <div><dt>State recognition</dt><dd>{summary.stateRecognitionRate}% <small>(≥ 90% ≤ 10s)</small></dd></div>
      <div><dt>P0/P1 findings</dt><dd>{summary.p0p1Count}</dd></div>
    </dl>
    <div className="preview-metrics__privacy">
      <label><input type="checkbox" checked={data.telemetryEnabled} disabled={!onTelemetryChange} onChange={(event) => onTelemetryChange?.(event.target.checked)} /> Telemetry {data.telemetryEnabled ? "on" : "off"}</label>
      <span>Source and secrets excluded</span>
    </div>
    <div className="preview-metrics__actions">
      {!reviewed
        ? <button type="button" onClick={() => setReviewed(true)}><Download aria-hidden="true" size={15} /> Review redacted receipt</button>
        : <><span className="preview-metrics__reviewed"><Check aria-hidden="true" size={14} /> Redacted preview reviewed</span><button type="button" onClick={onExport} disabled={!onExport}><Download aria-hidden="true" size={15} /> Confirm export</button></>}
    </div>
  </section>;
}
