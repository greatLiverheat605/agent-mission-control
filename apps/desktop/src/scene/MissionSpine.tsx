import { Line } from "@react-three/drei/core/Line.js";
import type { FlightStage, FlightViewModel } from "@mission-control/mission-store";
import { statusVisual } from "@mission-control/ui";
import { useCssColor } from "./sceneTokens";

export function MissionSpine({ flight, onStageSelect }: { flight: FlightViewModel; onStageSelect?: (stageId: string) => void }) {
  const lineColor = useCssColor("--mc-status-info");
  const points = flight.stages.map((_, index) => stagePosition(index, flight.stages.length));
  return <group name={`mission-spine:${flight.primaryRoute.id}`}>
    <Line points={points} color={lineColor} lineWidth={1.5} transparent opacity={0.46} />
    {flight.stages.map((stage, index) => <StageNode
      key={stage.id}
      routeId={flight.primaryRoute.id}
      stage={stage}
      position={points[index]}
      onSelect={onStageSelect}
    />)}
  </group>;
}

function StageNode({ routeId, stage, position, onSelect }: { routeId: string; stage: FlightStage; position: [number, number, number]; onSelect?: (stageId: string) => void }) {
  const visual = statusVisual(stage.state === "complete" ? "Completed" : stage.state === "current" ? stage.routeState : "Draft");
  const color = useCssColor(visual.colorToken);
  const scale = stage.state === "current" ? 1.25 : stage.state === "complete" ? 0.92 : 0.72;
  return <group position={position} name={stageObjectId(routeId, stage.id)} userData={{ objectId: stageObjectId(routeId, stage.id), stageId: stage.id }} onClick={(event) => { event.stopPropagation(); onSelect?.(stage.id); }}>
    <mesh scale={scale}>
      <sphereGeometry args={[0.11, 20, 20]} />
      <meshStandardMaterial color={color} emissive={color} emissiveIntensity={stage.state === "current" ? 1.4 : 0.32} roughness={0.54} />
    </mesh>
    {stage.state === "current" && <mesh rotation={[Math.PI / 2, 0, 0]}>
      <ringGeometry args={[0.18, 0.21, 40]} />
      <meshBasicMaterial color={color} transparent opacity={0.62} />
    </mesh>}
  </group>;
}

export function stagePosition(index: number, total: number): [number, number, number] {
  const x = (index - (total - 1) / 2) * 1.4;
  const y = Math.sin(index * 0.8) * 0.25;
  const z = -Math.cos(index * 0.55) * 0.18;
  return [round(x), round(y), round(z)];
}

export function stageObjectId(routeId: string, stageId: string): string {
  return `route:${routeId}:stage:${stageId}`;
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}
