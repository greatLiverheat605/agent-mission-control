import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { StorageManager } from "./StorageManager";

afterEach(cleanup);

it("requires a second confirmation after showing the delete impact plan", () => {
  const onDelete = vi.fn();
  const onExport = vi.fn();
  render(
    <StorageManager
      snapshot={{ missionId: "mission-1", usedBytes: 2048, eventCount: 4, archived: false }}
      impact={{
        impactHash: "impact-1234567890",
        projectedBytes: 2048,
        affectedEvents: 4,
        affectedBlobs: 1,
        automaticDeletion: false,
        blobs: [{ hash: "blob-abcdef123456", size: 512, willRemove: true }],
        plan: { mission_id: "mission-1", impact_hash: "impact-1234567890" },
      }}
      onDelete={onDelete}
      onExport={onExport}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: /^Delete$/ }));
  expect(screen.getByRole("alertdialog", { name: "Confirm mission deletion" })).toBeTruthy();
  expect(within(screen.getByRole("alertdialog")).getByText(/4 events/)).toBeTruthy();
  expect(onDelete).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: /Confirm delete/ }));
  expect(onDelete).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", { name: /Export/ }));
  expect(onExport).toHaveBeenCalledTimes(1);
});
