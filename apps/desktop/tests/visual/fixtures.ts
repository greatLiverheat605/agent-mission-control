import type { Page } from "@playwright/test";
import {
  DEFAULT_VISUAL_FIXTURE,
  visualFixtureSearch,
  type VisualFixtureConfig,
} from "../../src/dev/visualFixture";

export type VisualFixtureCase = {
  name: string;
  config: VisualFixtureConfig;
};

export const PAIRWISE_VISUAL_CASES: VisualFixtureCase[] = [
  visualCase("draft-nav-en", { routeState: "Draft", view: "nav" }),
  visualCase("exploration-sector-zh-long", { routeState: "ReadOnlyExploration", view: "sector", locale: "zh-CN", contentCase: "long" }),
  visualCase("plan-approval-authority-en", { routeState: "AwaitingPlanApproval", view: "authority" }),
  visualCase("executing-nav-zh", { routeState: "Executing", view: "nav", locale: "zh-CN" }),
  visualCase("verifying-records-en-error", { routeState: "Verifying", view: "records", contentCase: "error" }),
  visualCase("acceptance-mission-zh-long", { routeState: "AwaitingAcceptance", view: "mission", locale: "zh-CN", contentCase: "long" }),
  visualCase("completed-records-en-empty-reduced", { routeState: "Completed", view: "records", contentCase: "empty", motion: "reduced" }),
  visualCase("paused-systems-zh-offline", { routeState: "Paused", view: "systems", locale: "zh-CN", contentCase: "offline", motion: "reduced" }),
  visualCase("blocked-mission-en-error", { routeState: "Blocked", view: "mission", contentCase: "error" }),
  visualCase("abandoned-authority-zh-empty", { routeState: "Abandoned", view: "authority", locale: "zh-CN", contentCase: "empty" }),
  visualCase("unknown-nav-en-long-2d", { routeState: "Unknown", view: "nav", contentCase: "long", webgl: "fallback" }),
];

export function visualCase(name: string, overrides: Partial<VisualFixtureConfig> = {}): VisualFixtureCase {
  return { name, config: { ...DEFAULT_VISUAL_FIXTURE, ...overrides } };
}

export async function openMissionFixture(page: Page, fixture: VisualFixtureConfig): Promise<void> {
  await page.goto(`/?${visualFixtureSearch(fixture)}`);
  await page.locator(".mission-shell").waitFor();
  await page.locator(`[data-cockpit-view="${fixture.contentCase === "offline" ? "systems" : fixture.view}"]`).waitFor();
  if (fixture.view === "nav" && fixture.contentCase !== "offline") await page.locator("[data-scene-ready]").waitFor();
}
