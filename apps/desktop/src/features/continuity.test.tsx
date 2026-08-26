import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { LocaleProvider } from "../i18n/LocaleProvider";
import { MemoryReviewPanel, type MemoryReviewItem } from "./memory/MemoryReviewPanel";
import { RecallInspector, type RecallEvidence } from "./memory/RecallInspector";
import { RecoveryReviewPanel, type RecoveryReviewManifest } from "./recovery/RecoveryReviewPanel";

function renderWithLocale(node: React.ReactNode) {
  return render(<LocaleProvider initialLocale="en-US">{node}</LocaleProvider>);
}

const memory: MemoryReviewItem = {
  id: "memory-1",
  kind: "constraint",
  content: "Keep the workspace read-only",
  sourceEventIds: ["event-12"],
  scope: "mission",
  freshness: "current",
  version: 2,
  status: "pending",
  author: "user",
};

const evidence: RecallEvidence = {
  id: "memory-1",
  content: memory.content,
  sourceEventIds: memory.sourceEventIds,
  scope: memory.scope,
  freshness: memory.freshness,
  version: memory.version,
};

const manifest: RecoveryReviewManifest = {
  missionId: "mission-1",
  routeId: "route-1",
  schemaVersion: 1,
  contractVersion: 3,
  checkpointId: "checkpoint-3",
  ledgerSequence: 18,
  loadoutFingerprint: "loadout-hash",
  contextPackHash: "context-hash",
  pendingApprovalHash: "approval-hash",
  entryHash: "entry-hash",
};

describe("continuity review panels", () => {
  test("memory review exposes provenance and dispatches an explicit decision", () => {
    const onDecision = vi.fn();
    renderWithLocale(<MemoryReviewPanel items={[memory]} onDecision={onDecision} />);

    const item = screen.getByRole("article", { name: "memory-1" });
    expect(within(item).getByText("event-12")).toBeTruthy();
    expect(within(item).getByText("mission")).toBeTruthy();
    fireEvent.click(within(item).getByRole("button", { name: "Confirm memory-1" }));
    expect(onDecision).toHaveBeenCalledWith("memory-1", "confirm");
  });

  test("recall inspector labels retrieval evidence without claiming hidden reasoning", () => {
    renderWithLocale(<RecallInspector evidence={[evidence]} />);

    expect(screen.getByRole("heading", { name: "Recall inspector" })).toBeTruthy();
    expect(screen.getByText("Retrieval evidence")).toBeTruthy();
    expect(screen.getByText("No hidden reasoning is shown")).toBeTruthy();
    expect(screen.queryByText(/chain.of.thought/i)).toBeNull();
  });

  test("recovery review gates resume on package verification", () => {
    const onVerify = vi.fn();
    const onResume = vi.fn();
    renderWithLocale(
      <RecoveryReviewPanel manifest={manifest} onVerify={onVerify} onResume={onResume} />,
    );

    expect(screen.getByText("loadout-hash")).toBeTruthy();
    expect(screen.getByText("context-hash")).toBeTruthy();
    expect(screen.queryByText(/key|credential|secret/i)).toBeNull();
    const resume = screen.getByRole("button", { name: "Resume from verified checkpoint" });
    expect((resume as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Verify recovery package" }));
    expect(onVerify).toHaveBeenCalledTimes(1);
    expect((resume as HTMLButtonElement).disabled).toBe(true);
  });
});
