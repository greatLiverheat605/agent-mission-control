import { expect, test } from "@playwright/test";

test.describe("mission session continuity", () => {
  test("surfaces provenance, recovery metadata, and explicit provider handoff", async ({ page }) => {
    await page.goto("http://127.0.0.1:1420/?visual-fixture=mission&view=systems&contentCase=long&locale=en-US");

    await expect(page.getByRole("heading", { name: "Memory review" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Recall inspector" })).toBeVisible();
    await expect(page.getByText("event-visual-1")).toBeVisible();
    await expect(page.getByText("fixture-context-v1")).toBeVisible();
    await expect(page.getByText(/key|credential|secret/i)).toHaveCount(0);

    await page.goto("http://127.0.0.1:1420/?visual-fixture=mission&view=authority&contentCase=long&locale=en-US");
    await expect(page.getByRole("heading", { name: "Continue on another provider" })).toBeVisible();
    const handoff = page.getByRole("region", { name: "Continue on another provider" });
    await expect(handoff.getByRole("combobox", { name: "Target provider" })).toHaveValue("claude");
    await expect(handoff.getByRole("button", { name: "Prepare handoff" })).toBeEnabled();
    await handoff.getByRole("button", { name: "Prepare handoff" }).click();
    await expect(handoff.getByRole("button", { name: "Confirm handoff" })).toBeVisible();
  });
});
