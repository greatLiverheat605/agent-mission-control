import { expect, test } from "@playwright/test";
import { visualCase } from "../../apps/desktop/tests/visual/fixtures";
import { visualFixtureSearch } from "../../apps/desktop/src/dev/visualFixture";

test("Codex systems view exposes lifecycle previews without raw payloads", async ({ page }) => {
  const fixture = visualCase("data-lifecycle-codex", {
    routeState: "Verifying",
    view: "systems",
    contentCase: "long",
    webgl: "fallback",
  }).config;
  await page.goto(`http://127.0.0.1:1420/?${visualFixtureSearch(fixture)}`);
  await page.locator(".mission-shell").waitFor();
  await page.locator('[data-cockpit-view="systems"]').waitFor();

  await expect(page.getByRole("heading", { name: "Storage manager" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Export preview" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Diagnostic preview" })).toBeVisible();
  await expect(page.getByTitle("Delete this mission")).toBeDisabled();
  await expect(page.getByText(/token|cookie|authorization|provider payload/i)).toHaveCount(0);
});
