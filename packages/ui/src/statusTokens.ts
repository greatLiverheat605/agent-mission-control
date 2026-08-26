import type { RouteState } from "@mission-control/mission-store";
import { ROUTE_STATES } from "@mission-control/mission-store";

export { ROUTE_STATES };

export type StatusVisual = {
  label: string;
  icon: string;
  colorToken: `--mc-status-${string}`;
  lineStyle: "solid" | "dotted" | "dashed" | "double" | "broken";
  motionState: "still" | "scanning" | "waiting" | "moving" | "checking" | "settled" | "paused" | "interrupted" | "disconnected" | "unknown";
  ariaDescription: string;
};

export const STATUS_VISUALS = {
  Draft: { label: "Draft", icon: "CircleDotDashed", colorToken: "--mc-status-unknown", lineStyle: "dotted", motionState: "still", ariaDescription: "Route draft; execution has not started" },
  ReadOnlyExploration: { label: "Exploring", icon: "Search", colorToken: "--mc-status-info", lineStyle: "dotted", motionState: "scanning", ariaDescription: "Route is exploring the project without writes" },
  AwaitingPlanApproval: { label: "Plan approval", icon: "ShieldAlert", colorToken: "--mc-status-waiting", lineStyle: "dashed", motionState: "waiting", ariaDescription: "Route is waiting for plan approval" },
  Executing: { label: "Executing", icon: "Activity", colorToken: "--mc-status-info", lineStyle: "solid", motionState: "moving", ariaDescription: "Route is executing an approved action" },
  Verifying: { label: "Verifying", icon: "FileCheck2", colorToken: "--mc-status-verified", lineStyle: "solid", motionState: "checking", ariaDescription: "Route is verifying changes against evidence" },
  AwaitingAcceptance: { label: "Acceptance", icon: "CircleAlert", colorToken: "--mc-status-waiting", lineStyle: "dashed", motionState: "waiting", ariaDescription: "Route is waiting for user acceptance" },
  Completed: { label: "Completed", icon: "CircleCheck", colorToken: "--mc-status-verified", lineStyle: "solid", motionState: "settled", ariaDescription: "Route completed with acceptance evidence" },
  Paused: { label: "Paused", icon: "CirclePause", colorToken: "--mc-status-waiting", lineStyle: "double", motionState: "paused", ariaDescription: "Route is paused at a safe boundary" },
  Blocked: { label: "Blocked", icon: "Ban", colorToken: "--mc-status-waiting", lineStyle: "broken", motionState: "interrupted", ariaDescription: "Route is blocked and requires a decision or dependency" },
  Abandoned: { label: "Abandoned", icon: "OctagonX", colorToken: "--mc-status-danger", lineStyle: "broken", motionState: "disconnected", ariaDescription: "Route was abandoned; its checkpoint and evidence are retained" },
  Unknown: { label: "Unknown", icon: "CircleAlert", colorToken: "--mc-status-unknown", lineStyle: "dotted", motionState: "unknown", ariaDescription: "状态证据不完整; route state cannot be trusted" },
} satisfies Record<RouteState, StatusVisual>;

export function statusVisual(state: RouteState): StatusVisual {
  return STATUS_VISUALS[state];
}
