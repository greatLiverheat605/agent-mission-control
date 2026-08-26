import { Line } from "@react-three/drei/core/Line.js";
import type { FlightViewModel, RouteState } from "@mission-control/mission-store";
import { statusVisual } from "@mission-control/ui";
import { stagePosition } from "./MissionSpine";
import { useCssColor } from "./sceneTokens";

export function RouteBranch({ route, index, stageCount }: { route: FlightViewModel["derivedRoutes"][number]; index: number; stageCount: number }) {
  const visual = routeBranchVisual(route.state);
  const color = useCssColor(statusVisual(route.state).colorToken);
  const start = stagePosition(Math.min(3, stageCount - 1), stageCount);
  const end: [number, number, number] = [start[0] + 0.9 + index * 0.35, start[1] + 0.8 + index * 0.32, start[2] - 0.5];
  return <group name={`route-branch:${route.id}`} userData={{ objectId: `route:${route.id}`, connected: visual.connected }}>
    <Line points={[start, end]} color={color} lineWidth={1.2} dashed={visual.lineStyle !== "solid"} dashSize={0.12} gapSize={visual.connected ? 0.08 : 0.2} transparent opacity={visual.connected ? 0.7 : 0.5} />
    <mesh position={end}>
      <boxGeometry args={[0.13, 0.13, 0.13]} />
      <meshBasicMaterial color={color} wireframe={!visual.connected} />
    </mesh>
  </group>;
}

export function routeBranchVisual(state: RouteState): { connected: boolean; lineStyle: "solid" | "dashed" | "broken" } {
  if (state === "Abandoned") return { connected: false, lineStyle: "broken" };
  if (state === "Blocked" || state === "Paused" || state === "Unknown") return { connected: true, lineStyle: "dashed" };
  return { connected: true, lineStyle: "solid" };
}
