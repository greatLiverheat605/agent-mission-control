import { ROUTE_STATES } from "@mission-control/mission-store";
import { describe, expect, it } from "vitest";
import { COCKPIT_VIEW_IDS } from "../shell/cockpitViewIds";
import { LOCALES } from "../i18n/types";
import { fixtureMission } from "./VisualFixtureApp";
import {
  DEFAULT_VISUAL_FIXTURE,
  parseVisualFixture,
  visualFixtureDataMatrix,
  visualFixtureSearch,
  type VisualFixtureConfig,
} from "./visualFixture";

describe("visual fixture contract", () => {
  it("round-trips all 11 x 6 x 2 route, view, and locale combinations", () => {
    const matrix = visualFixtureDataMatrix();
    expect(matrix).toHaveLength(ROUTE_STATES.length * COCKPIT_VIEW_IDS.length * LOCALES.length);
    expect(new Set(matrix.map(({ routeState, view, locale }) => `${routeState}:${view}:${locale}`)).size).toBe(matrix.length);
    for (const config of matrix) expect(parseVisualFixture(`?${visualFixtureSearch(config)}`)).toEqual(config);
  });

  it("fails malformed query values back to the deterministic baseline", () => {
    expect(parseVisualFixture("?visual-fixture=mission&routeState=made-up&view=bridge&locale=pirate&webgl=maybe&motion=spin&contentCase=huge")).toEqual(DEFAULT_VISUAL_FIXTURE);
    expect(parseVisualFixture("?routeState=Blocked")).toBeNull();
  });

  it.each([
    ["empty", 2, "running"],
    ["error", 9, "running"],
    ["offline", 9, "paused"],
    ["long", 8, "running"],
  ] satisfies Array<[VisualFixtureConfig["contentCase"], number, string]>)
  ("projects the %s content case without inventing a second data model", (contentCase, minimumEvents, status) => {
    const mission = fixtureMission({ ...DEFAULT_VISUAL_FIXTURE, contentCase });
    expect(mission.events.length).toBeGreaterThanOrEqual(minimumEvents);
    expect(mission.status).toBe(status);
    expect(mission.events[1].payload.state).toBe("Executing");
  });
});
