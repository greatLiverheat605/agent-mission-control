import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { PreviewMetrics, summarizePreviewMetrics, type PreviewParticipant } from "./PreviewMetrics";

const participants: PreviewParticipant[] = [
  { id: "pilot-1", projectType: "typescript", hardwareProfile: "minimum", firstLaunchCompleted: true, stateRecognitionCorrect: true, stateRecognitionSeconds: 8, guided: false },
  { id: "pilot-2", projectType: "rust", hardwareProfile: "minimum", firstLaunchCompleted: true, stateRecognitionCorrect: true, stateRecognitionSeconds: 7, guided: false },
  { id: "pilot-3", projectType: "mixed", hardwareProfile: "recommended", firstLaunchCompleted: true, stateRecognitionCorrect: true, stateRecognitionSeconds: 6, guided: false },
  { id: "pilot-4", projectType: "typescript", hardwareProfile: "recommended", firstLaunchCompleted: true, stateRecognitionCorrect: true, stateRecognitionSeconds: 9, guided: false },
  { id: "pilot-5", projectType: "rust", hardwareProfile: "minimum", firstLaunchCompleted: false, stateRecognitionCorrect: true, stateRecognitionSeconds: 10, guided: false },
];

describe("PreviewMetrics", () => {
  afterEach(() => cleanup());

  test("computes the 80/90 pilot gate locally", () => {
    expect(summarizePreviewMetrics(participants)).toEqual({ participants: 5, firstLaunchRate: 80, stateRecognitionRate: 100, p0p1Count: 0, expansionGate: true });
  });

  test("requires a redacted preview review before export confirmation", () => {
    const onExport = vi.fn();
    render(<PreviewMetrics data={{ participants, telemetryEnabled: false }} onExport={onExport} />);
    expect(screen.queryByRole("button", { name: "Confirm export" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Review redacted receipt" }));
    expect(screen.getByText("Redacted preview reviewed")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Confirm export" }));
    expect(onExport).toHaveBeenCalledTimes(1);
  });

  test("keeps telemetry toggle disabled without an explicit handler", () => {
    render(<PreviewMetrics data={{ participants: [], telemetryEnabled: false }} />);
    expect((screen.getByRole("checkbox") as HTMLInputElement).disabled).toBe(true);
  });
});
