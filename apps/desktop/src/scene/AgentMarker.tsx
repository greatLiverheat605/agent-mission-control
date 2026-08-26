import { useRef } from "react";
import { useFrame } from "@react-three/fiber";
import type { FlightViewModel } from "@mission-control/mission-store";
import { statusVisual } from "@mission-control/ui";
import type * as THREE from "three";
import { stagePosition } from "./MissionSpine";
import { useCssColor } from "./sceneTokens";

export function AgentMarker({ flight, reducedMotion = false }: { flight: FlightViewModel; reducedMotion?: boolean }) {
  const marker = useRef<THREE.Group>(null);
  const color = useCssColor(statusVisual(flight.primaryRoute.state).colorToken);
  const position = agentPositionFor(flight);
  useFrame(({ clock }) => {
    if (!marker.current) return;
    const pulse = reducedMotion || flight.agentPosition.motionState === "paused" ? 1 : 1 + Math.sin(clock.elapsedTime * 3.2) * 0.08;
    marker.current.scale.setScalar(pulse);
  });
  return <group ref={marker} position={position} name={`agent:${flight.primaryRoute.id}`} userData={{ objectId: `agent:${flight.primaryRoute.id}` }}>
    <mesh rotation={[0, 0, Math.PI / 4]}>
      <octahedronGeometry args={[0.15, 0]} />
      <meshStandardMaterial color={color} emissive={color} emissiveIntensity={1.8} roughness={0.28} metalness={0.24} />
    </mesh>
  </group>;
}

export function agentPositionFor(flight: FlightViewModel): [number, number, number] {
  return stagePosition(Math.min(flight.agentPosition.stageIndex, Math.max(0, flight.stages.length - 1)), flight.stages.length);
}
