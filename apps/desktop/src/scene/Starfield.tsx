import { useMemo, useRef } from "react";
import { useFrame } from "@react-three/fiber";
import * as THREE from "three";
import { useCssColor } from "./sceneTokens";

export function Starfield({ count = 900, reducedMotion = false }: { count?: number; reducedMotion?: boolean }) {
  const trails = useRef<THREE.LineSegments>(null);
  const color = useCssColor("--mc-text-muted");
  const geometry = useMemo(() => {
    const random = seededRandom(0x5f3759df);
    const positions = new Float32Array(count * 6);
    const speeds = new Float32Array(count);
    for (let index = 0; index < count; index += 1) {
      const z = -4 - random() * 42;
      const spread = 2.5 + Math.abs(z) * 0.24;
      const x = (random() - 0.5) * spread * 2;
      const y = (random() - 0.5) * spread * 1.15;
      const speed = 2.2 + random() * 5.8;
      const offset = index * 6;
      positions[offset] = x;
      positions[offset + 1] = y;
      positions[offset + 2] = z;
      positions[offset + 3] = x * 1.015;
      positions[offset + 4] = y * 1.015;
      positions[offset + 5] = z + 0.08 + speed * 0.055;
      speeds[index] = speed;
    }
    const next = new THREE.BufferGeometry();
    next.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    next.userData.speeds = speeds;
    return next;
  }, [count]);

  useFrame((_, delta) => {
    if (reducedMotion || !trails.current) return;
    const attribute = trails.current.geometry.getAttribute("position") as THREE.BufferAttribute;
    const speeds = trails.current.geometry.userData.speeds as Float32Array;
    for (let index = 0; index < count; index += 1) {
      const offset = index * 2;
      const speed = speeds[index] * delta;
      let z = attribute.getZ(offset) + speed;
      if (z > 8) z = -42;
      attribute.setZ(offset, z);
      attribute.setZ(offset + 1, z + 0.08 + speeds[index] * 0.055);
    }
    attribute.needsUpdate = true;
  });

  return <group>
    <lineSegments ref={trails} geometry={geometry} frustumCulled={false}>
      <lineBasicMaterial color={color} transparent opacity={0.5} depthWrite={false} />
    </lineSegments>
    <points geometry={geometry} frustumCulled={false}>
      <pointsMaterial color={color} size={0.025} sizeAttenuation transparent opacity={0.74} depthWrite={false} />
    </points>
  </group>;
}

function seededRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}
