import { useRef } from "react";
import { useFrame } from "@react-three/fiber";
import { statusVisual } from "@mission-control/ui";
import type { FlightViewModel } from "@mission-control/mission-store";
import type * as THREE from "three";
import { useCssColor } from "./sceneTokens";

export function FlightVehicle({ flight, reducedMotion }: { flight: FlightViewModel; reducedMotion: boolean }) {
  const vessel = useRef<THREE.Group>(null);
  const signal = useCssColor(statusVisual(flight.primaryRoute.state).colorToken);
  useFrame(({ clock }) => {
    if (!vessel.current || reducedMotion) return;
    vessel.current.rotation.z = Math.sin(clock.elapsedTime * 0.62) * 0.018;
    vessel.current.position.y = -0.58 + Math.sin(clock.elapsedTime * 0.8) * 0.018;
  });
  return <group ref={vessel} position={[0, -0.58, 0.45]} scale={0.9} name={`vessel:${flight.primaryRoute.id}`}>
    <mesh position={[0, 0, -0.12]}>
      <boxGeometry args={[0.22, 0.12, 0.68]} />
      <meshStandardMaterial color="#15202a" emissive={signal} emissiveIntensity={0.16} metalness={0.72} roughness={0.3} />
    </mesh>
    <mesh position={[0, 0, -0.52]} rotation={[-Math.PI / 2, 0, 0]}>
      <coneGeometry args={[0.11, 0.3, 4]} />
      <meshStandardMaterial color="#283947" emissive={signal} emissiveIntensity={0.12} metalness={0.82} roughness={0.24} />
    </mesh>
    <mesh position={[-0.3, -0.015, 0.02]} rotation={[0, 0.16, -0.04]}>
      <boxGeometry args={[0.48, 0.045, 0.34]} />
      <meshStandardMaterial color="#101922" metalness={0.76} roughness={0.34} />
    </mesh>
    <mesh position={[0.3, -0.015, 0.02]} rotation={[0, -0.16, 0.04]}>
      <boxGeometry args={[0.48, 0.045, 0.34]} />
      <meshStandardMaterial color="#101922" metalness={0.76} roughness={0.34} />
    </mesh>
    {[-0.08, 0.08].map((x) => <group key={x} position={[x, -0.025, 0.27]}>
      <mesh>
        <cylinderGeometry args={[0.035, 0.045, 0.12, 16]} />
        <meshStandardMaterial color="#334651" metalness={0.8} roughness={0.24} />
      </mesh>
      <mesh position={[0, 0.075, 0]}>
        <sphereGeometry args={[0.033, 16, 16]} />
        <meshBasicMaterial color={signal} transparent opacity={0.9} />
      </mesh>
      {!reducedMotion && <pointLight position={[0, 0.12, 0]} color={signal} intensity={0.45} distance={1.6} />}
    </group>)}
  </group>;
}
