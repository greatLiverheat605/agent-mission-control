export const COCKPIT_VIEW_IDS = ["nav", "sector", "mission", "records", "systems", "authority"] as const;
export type CockpitViewId = typeof COCKPIT_VIEW_IDS[number];
