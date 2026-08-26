import { ROUTE_STATES } from "@mission-control/mission-store";
import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  VISUAL_CONTENT_CASES,
  VISUAL_MOTION_MODES,
  VISUAL_WEBGL_MODES,
} from "../../src/dev/visualFixture";
import { LOCALES } from "../../src/i18n/types";
import { COCKPIT_VIEW_IDS } from "../../src/shell/cockpitViewIds";
import { analyzeCanvas, expectCanvasSignal, frameDifference } from "./canvas-pixels";
import { openMissionFixture, PAIRWISE_VISUAL_CASES, visualCase } from "./fixtures";

const viewports = [
  { name: "1024x640", width: 1024, height: 640 },
  { name: "1280x720", width: 1280, height: 720 },
  { name: "1440x900", width: 1440, height: 900 },
  { name: "1920x1080", width: 1920, height: 1080 },
  { name: "2560x1080", width: 2560, height: 1080 },
];

test("direct development root renders the integrated cockpit", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto("/");
  await page.waitForLoadState("domcontentloaded");

  await expect(page.locator("[data-cockpit-shell]")).toBeVisible();
  await expect(page.locator("[data-structural-beam]")).toBeVisible();
  await expect(page.locator("[data-console='port']")).toBeVisible();
  await expect(page.locator("[data-mfd]")).toBeVisible();
  await expect(page.locator("[data-console='starboard']")).toBeVisible();
  await expect(page.locator("[data-flight-helm]")).toBeVisible();
  await expect(page.locator("[data-emergency-control]")).toBeVisible();
  await expect(page.locator(".mission-shell__softkeys").getByRole("tab")).toHaveCount(6);
  await expect(page.locator("canvas")).toHaveCount(1);
  expectCanvasSignal(await analyzeCanvas(page.locator("canvas")));
  await assertCockpitLayout(page);
  await testInfo.attach("direct-root-cockpit", { body: await page.screenshot({ fullPage: true }), contentType: "image/png" });
});

test("keeps the right drawer label upright when the cockpit collapses", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 640 });
  await openMissionFixture(page, visualCase("responsive-long-content", { contentCase: "long" }).config);

  const trigger = page.locator('.mission-shell__starboard .mc-responsive-drawer__trigger');
  await expect(trigger).toBeVisible();
  await expect.poll(() => trigger.evaluate((element) => {
    const style = getComputedStyle(element);
    return { transform: style.transform, writingMode: style.writingMode };
  })).toEqual({ transform: "none", writingMode: "vertical-rl" });
});

test("pairwise visual cases cover every fixture dimension value", () => {
  const configs = PAIRWISE_VISUAL_CASES.map(({ config }) => config);
  expect(new Set(configs.map(({ routeState }) => routeState))).toEqual(new Set(ROUTE_STATES));
  expect(new Set(configs.map(({ view }) => view))).toEqual(new Set(COCKPIT_VIEW_IDS));
  expect(new Set(configs.map(({ locale }) => locale))).toEqual(new Set(LOCALES));
  expect(new Set(configs.map(({ webgl }) => webgl))).toEqual(new Set(VISUAL_WEBGL_MODES));
  expect(new Set(configs.map(({ motion }) => motion))).toEqual(new Set(VISUAL_MOTION_MODES));
  expect(new Set(configs.map(({ contentCase }) => contentCase))).toEqual(new Set(VISUAL_CONTENT_CASES));
});

for (const viewport of viewports) {
  test(`mission flight remains complete at ${viewport.name}`, async ({ browser }, testInfo) => {
    const context = await browser.newContext({ viewport: { width: viewport.width, height: viewport.height }, reducedMotion: "no-preference" });
    const page = await context.newPage();
    const fixture = visualCase("responsive-long-content", { contentCase: "long" }).config;
    await openMissionFixture(page, fixture);

    await expect(page.locator(".mission-shell__softkeys").getByRole("tab")).toHaveCount(6);
    await expect(page.locator(".flight-helm-commands .hold-command")).toBeVisible();
    if (viewport.width >= 1280) await expect(page.locator(".beam-cell--sequence")).toBeVisible();
    await expect(page.getByRole("img", { name: /Control Coding.*Executing.*Run visual release gates/i })).toBeVisible();
    const portDrawerOpened = await openDrawerIfNeeded(page, "Mission registry");
    await expect(page.locator("#mission-port-console .project-mission")).toHaveCount(11);
    if (portDrawerOpened) await page.getByRole("button", { name: "Mission registry", exact: true }).click();
    const taskDrawerOpened = await openDrawerIfNeeded(page, "Task console");
    await expect(page.locator("#mission-starboard-console")).toContainText("Current waypoint");
    if (taskDrawerOpened) await page.getByRole("button", { name: "Task console", exact: true }).click();

    const canvas = page.locator("canvas");
    const pixels = await analyzeCanvas(canvas);
    expectCanvasSignal(pixels);
    await expect.poll(async () => {
      await page.waitForTimeout(180);
      const next = await analyzeCanvas(canvas);
      return frameDifference(pixels.buffer, next.buffer);
    }, { timeout: 2_000 }).toBeGreaterThan(0.00002);

    await assertCockpitLayout(page);
    await assertViewReachability(page);

    const screenshot = await page.screenshot({ fullPage: true });
    await testInfo.attach(`mission-flight-${viewport.name}`, { body: screenshot, contentType: "image/png" });
    await context.close();
  });
}

for (const fixture of PAIRWISE_VISUAL_CASES) {
  test(`renders pairwise fixture ${fixture.name}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openMissionFixture(page, fixture.config);
    const effectiveView = fixture.config.contentCase === "offline" ? "systems" : fixture.config.view;

    await expect(page.locator("html")).toHaveAttribute("lang", fixture.config.locale);
    await expect(page.locator(".mission-shell")).toHaveAttribute("data-active-view", effectiveView);
    await expect(page.locator(`[data-cockpit-view="${effectiveView}"]`)).toBeVisible();
    await expect(page.locator(".mission-shell")).toHaveAttribute("data-route-state", fixture.config.routeState);

    if (fixture.config.contentCase === "long") await expect(page.locator(".beam-section--identity .beam-cell").first().locator("strong")).toContainText("超长任务名称");
    if (fixture.config.contentCase === "error") await expect(page.getByText("Supervisor channel failed during evidence synchronization").first()).toBeVisible();
    if (fixture.config.contentCase === "offline") await expect(page.locator(".recovery-actions")).toBeVisible();
    if (fixture.config.contentCase === "empty" && effectiveView === "records") await expect(page.locator(".panel-empty")).toBeVisible();
    if (fixture.config.webgl === "fallback") await expect(page.locator(".scene-fallback")).toBeVisible();

    if (effectiveView === "nav" && fixture.config.webgl === "enabled") expectCanvasSignal(await analyzeCanvas(page.locator("canvas")));
    await assertCockpitLayout(page);
    await testInfo.attach(fixture.name, { body: await page.screenshot({ fullPage: true }), contentType: "image/png" });
  });
}

for (const zoom of [0.8, 1, 1.25, 1.5]) {
  test(`cockpit survives ${Math.round(zoom * 100)}% CSS browser zoom stress`, async ({ page }) => {
    await page.setViewportSize({ width: 1920, height: 1080 });
    await openMissionFixture(page, visualCase("zoom").config);
    await page.evaluate((scale) => { document.documentElement.style.zoom = String(scale); }, zoom);
    await expect(page.locator(".mission-shell__softkeys").getByRole("tab")).toHaveCount(6);
    await expect(page.getByRole("button", { name: "Request safe pause" })).toBeVisible();
    await assertNoPageOverflow(page);
    await assertNoEmergencyOverlap(page);
  });
}

test("switches to an information-equivalent 2D spine without WebGL", async ({ page }) => {
  await openMissionFixture(page, visualCase("2d-blocked", { routeState: "Blocked", webgl: "fallback" }).config);
  await expect(page.getByTestId("fallback-agent")).toBeVisible();
  await expect(page.getByRole("button", { name: "Focus stage Executing" })).toBeVisible();
  await expect(page.locator(".scene-fallback__plot").getByText("Abandoned route route-abandoned")).toBeVisible();
  await page.getByRole("tab", { name: "Systems" }).click();
  await expect(page.getByText("2D fallback")).toBeVisible();
});

async function assertViewReachability(page: Page) {
  for (const view of COCKPIT_VIEW_IDS.filter((view) => view !== "nav")) {
    await page.locator(`.mission-shell__softkeys [data-view-id="${view}"]`).click();
    await expect(page.locator(`[data-cockpit-view="${view}"]`)).toBeVisible();
  }
}

async function openDrawerIfNeeded(page: Page, name: string) {
  const trigger = page.getByRole("button", { name, exact: true });
  if (!(await trigger.isVisible())) return false;
  await trigger.click();
  return true;
}

async function assertCockpitLayout(page: Page) {
  await assertNoPageOverflow(page);
  await assertTargets(page.locator("button:visible"));
  await assertNoEmergencyOverlap(page);
}

async function assertNoPageOverflow(page: Page) {
  const dimensions = await page.evaluate(() => ({ clientWidth: document.documentElement.clientWidth, scrollWidth: document.documentElement.scrollWidth }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
}

async function assertNoEmergencyOverlap(page: Page) {
  const emergency = await page.locator(".mission-shell__emergency").boundingBox();
  const helmButtons = await page.locator(".mission-shell__helm button:visible").all();
  for (const button of helmButtons) {
    const box = await button.boundingBox();
    expect(overlapArea(emergency, box), `Emergency control overlaps Helm button ${await button.textContent()}`).toBe(0);
  }
}

async function assertTargets(locator: Locator) {
  const count = await locator.count();
  for (let index = 0; index < count; index += 1) {
    const target = locator.nth(index);
    const box = await target.boundingBox();
    if (!box) continue;
    expect(box.width, `button width: ${await target.getAttribute("aria-label") ?? await target.textContent()}`).toBeGreaterThanOrEqual(44);
    expect(box.height, `button height: ${await target.getAttribute("aria-label") ?? await target.textContent()}`).toBeGreaterThanOrEqual(44);
  }
}

function overlapArea(left: { x: number; y: number; width: number; height: number } | null, right: { x: number; y: number; width: number; height: number } | null) {
  if (!left || !right) return 0;
  const width = Math.max(0, Math.min(left.x + left.width, right.x + right.width) - Math.max(left.x, right.x));
  const height = Math.max(0, Math.min(left.y + left.height, right.y + right.height) - Math.max(left.y, right.y));
  return width * height;
}
