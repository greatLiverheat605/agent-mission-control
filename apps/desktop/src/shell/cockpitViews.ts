import { createElement } from "react";
import type { LucideIcon } from "lucide-react";
import {
  ListTree,
  Navigation,
  Orbit,
  Route,
  Settings2,
  ShieldCheck,
} from "@mission-control/ui";
import { COCKPIT_VIEW_IDS, type CockpitViewId } from "./cockpitViewIds";

export { COCKPIT_VIEW_IDS, type CockpitViewId } from "./cockpitViewIds";

export type CockpitViewDefinition = {
  id: CockpitViewId;
  label: string;
  shortLabel: string;
  icon: LucideIcon;
  focusLabel: string;
};

export const COCKPIT_VIEWS = {
  nav: { id: "nav", label: "Navigation", shortLabel: "NAV", icon: Navigation, focusLabel: "Navigation display" },
  sector: { id: "sector", label: "Sector", shortLabel: "SECTOR", icon: Orbit, focusLabel: "Sector display" },
  mission: { id: "mission", label: "Mission", shortLabel: "MISSION", icon: Route, focusLabel: "Mission display" },
  records: { id: "records", label: "Records", shortLabel: "RECORDS", icon: ListTree, focusLabel: "Records display" },
  systems: { id: "systems", label: "Systems", shortLabel: "SYSTEMS", icon: Settings2, focusLabel: "Systems display" },
  authority: { id: "authority", label: "Authority", shortLabel: "AUTH", icon: ShieldCheck, focusLabel: "Authority display" },
} satisfies Record<CockpitViewId, CockpitViewDefinition>;

export const NAVIGATION_CAMERA_IDS = ["fwd", "trk", "tac", "aft"] as const;
export type NavigationCameraId = typeof NAVIGATION_CAMERA_IDS[number];

export const NAVIGATION_CAMERAS = {
  fwd: { id: "fwd", label: "FWD", description: "Forward viewport" },
  trk: { id: "trk", label: "TRK", description: "Tracking camera" },
  tac: { id: "tac", label: "TAC", description: "Tactical plot" },
  aft: { id: "aft", label: "AFT", description: "Aft viewport" },
} satisfies Record<NavigationCameraId, { id: NavigationCameraId; label: string; description: string }>;

export const cockpitViewItems = COCKPIT_VIEW_IDS.map((id) => ({
  id,
  label: COCKPIT_VIEWS[id].label,
  icon: createElement(COCKPIT_VIEWS[id].icon, { size: 16 }),
}));

export const navigationCameraItems = NAVIGATION_CAMERA_IDS.map((id) => ({
  id,
  label: NAVIGATION_CAMERAS[id].label,
}));
