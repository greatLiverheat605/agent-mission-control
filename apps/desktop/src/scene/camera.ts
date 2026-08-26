export type SceneCamera = { position: [number, number, number]; fov: number; near: number; far: number };

export function missionCamera(stageCount: number, aspect = 16 / 9): SceneCamera {
  const span = Math.max(6, stageCount * 1.4);
  return {
    position: [0, aspect < 1.4 ? 1.3 : 0.8, Math.max(7, span / Math.max(1, aspect) + 4.5)],
    fov: 42,
    near: 0.1,
    far: 100,
  };
}

export function missionCameraPosition(view: "fwd" | "trk" | "tac" | "aft", stageCount: number): { position: [number, number, number]; target: [number, number, number] } {
  const span = Math.max(6, stageCount * 1.4);
  if (view === "trk") return { position: [Math.max(6, span * 0.72), 3.2, Math.max(7, span * 0.65)], target: [0, 0, 0] };
  if (view === "tac") return { position: [0, Math.max(12, span), 0.01], target: [0, 0, 0] };
  if (view === "aft") return { position: [0, 1.2, -Math.max(9, span)], target: [0, 0, 0] };
  return { position: missionCamera(stageCount).position, target: [0, 0, 0] };
}
