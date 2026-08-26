import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { VISUAL_ROUTE_EVENT } from "../../src/dev/visualFixture";
import { analyzeCanvas, frameDifference } from "./canvas-pixels";
import { openMissionFixture, visualCase } from "./fixtures";

for (const fixture of [
  visualCase("axe-approval-en", { routeState: "AwaitingPlanApproval", view: "authority" }),
  visualCase("axe-blocked-zh", { routeState: "Blocked", view: "mission", locale: "zh-CN", contentCase: "error" }),
]) {
  test(`has no serious axe violations for ${fixture.name}`, async ({ page }) => {
    await openMissionFixture(page, fixture.config);
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations.filter((violation) => violation.impact === "critical" || violation.impact === "serious")).toEqual([]);

    const viewport = page.viewportSize()!;
    const pause = await page.locator(".flight-helm-commands .hold-command").boundingBox();
    const display = await page.locator(".mission-shell__display").boundingBox();
    expect(pause).not.toBeNull();
    expect(display).not.toBeNull();
    expect(pause!.x + pause!.width).toBeLessThanOrEqual(viewport.width);
    expect(pause!.y + pause!.height).toBeLessThanOrEqual(viewport.height);
    expect(display!.y).toBeGreaterThan(0);
  });
}

test("keyboard reaches commands, approval, evidence, and safe pause", async ({ page }) => {
  await openMissionFixture(page, visualCase("keyboard-approval", { routeState: "AwaitingPlanApproval", view: "authority" }).config);
  await page.keyboard.press("Control+K");
  const search = page.getByRole("combobox", { name: "Search missions and commands" });
  await expect(search).toBeFocused();
  await search.fill("pending approvals");
  await page.keyboard.press("Enter");
  await expect(page.locator("[aria-labelledby='approval-title']")).toBeFocused();
  await expect(page.getByRole("button", { name: "Approve once" })).toBeDisabled();

  await page.keyboard.press("Control+K");
  await search.fill("mission evidence");
  await page.keyboard.press("Enter");
  await expect(page.locator("[aria-labelledby='evidence-bay-title']")).toBeFocused();
  const evidence = page.locator(".evidence-batch summary").first();
  await evidence.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator(".evidence-batch").first()).toHaveAttribute("open", "");

  const pause = page.getByRole("button", { name: "Request safe pause" });
  await pause.focus();
  await page.keyboard.press("Enter");
  await expect(pause).toBeDisabled();
});

test("reduced motion stabilizes ambient frames while route state still updates", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openMissionFixture(page, visualCase("reduced-motion", { motion: "reduced" }).config);
  const canvas = page.locator("canvas");
  const first = await analyzeCanvas(canvas);
  await page.waitForTimeout(180);
  const second = await analyzeCanvas(canvas);
  expect(frameDifference(first.buffer, second.buffer)).toBeLessThan(0.0005);

  await page.evaluate(({ eventName, routeState }) => {
    window.dispatchEvent(new CustomEvent(eventName, { detail: routeState }));
  }, { eventName: VISUAL_ROUTE_EVENT, routeState: "Blocked" });
  await expect(page.getByRole("img", { name: /Blocked/i })).toBeVisible({ timeout: 3_000 });
});
