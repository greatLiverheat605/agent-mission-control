import { ROUTE_STATES, type RouteState } from "@mission-control/mission-store";
import { LOCALES, type Locale } from "../i18n/types";
import { COCKPIT_VIEW_IDS, type CockpitViewId } from "../shell/cockpitViewIds";

export const VISUAL_WEBGL_MODES = ["enabled", "fallback"] as const;
export const VISUAL_MOTION_MODES = ["full", "reduced"] as const;
export const VISUAL_CONTENT_CASES = ["standard", "long", "empty", "error", "offline"] as const;

export type VisualWebGlMode = typeof VISUAL_WEBGL_MODES[number];
export type VisualMotionMode = typeof VISUAL_MOTION_MODES[number];
export type VisualContentCase = typeof VISUAL_CONTENT_CASES[number];

export type VisualFixtureConfig = {
  routeState: RouteState;
  view: CockpitViewId;
  locale: Locale;
  webgl: VisualWebGlMode;
  motion: VisualMotionMode;
  contentCase: VisualContentCase;
};

export const DEFAULT_VISUAL_FIXTURE: VisualFixtureConfig = {
  routeState: "Executing",
  view: "nav",
  locale: "en-US",
  webgl: "enabled",
  motion: "full",
  contentCase: "standard",
};

export const VISUAL_ROUTE_EVENT = "mission-control:visual-route-state";

export function parseVisualFixture(search: string): VisualFixtureConfig | null {
  const params = new URLSearchParams(search);
  if (!params.has("visual-fixture")) return null;
  return {
    routeState: member(params.get("routeState"), ROUTE_STATES, DEFAULT_VISUAL_FIXTURE.routeState),
    view: member(params.get("view"), COCKPIT_VIEW_IDS, DEFAULT_VISUAL_FIXTURE.view),
    locale: member(params.get("locale"), LOCALES, DEFAULT_VISUAL_FIXTURE.locale),
    webgl: member(params.get("webgl"), VISUAL_WEBGL_MODES, DEFAULT_VISUAL_FIXTURE.webgl),
    motion: member(params.get("motion"), VISUAL_MOTION_MODES, DEFAULT_VISUAL_FIXTURE.motion),
    contentCase: member(params.get("contentCase"), VISUAL_CONTENT_CASES, DEFAULT_VISUAL_FIXTURE.contentCase),
  };
}

export function visualFixtureSearch(config: VisualFixtureConfig): string {
  return new URLSearchParams({
    "visual-fixture": "mission",
    routeState: config.routeState,
    view: config.view,
    locale: config.locale,
    webgl: config.webgl,
    motion: config.motion,
    contentCase: config.contentCase,
  }).toString();
}

export function visualFixtureDataMatrix(): VisualFixtureConfig[] {
  return ROUTE_STATES.flatMap((routeState) => COCKPIT_VIEW_IDS.flatMap((view) => LOCALES.map((locale) => ({
    ...DEFAULT_VISUAL_FIXTURE,
    routeState,
    view,
    locale,
  }))));
}

function member<const T extends readonly string[]>(value: string | null, values: T, fallback: T[number]): T[number] {
  return value !== null && (values as readonly string[]).includes(value) ? value as T[number] : fallback;
}
