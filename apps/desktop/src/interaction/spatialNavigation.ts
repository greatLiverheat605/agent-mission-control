export type SpatialDirection = "left" | "right" | "up" | "down";
export type SpatialItem = { id: string; x: number; y: number };

export function spatialNext(currentId: string, direction: SpatialDirection, items: SpatialItem[]): string {
  const current = items.find((item) => item.id === currentId);
  if (!current) return currentId;
  const candidates = items.flatMap((item) => {
    if (item.id === currentId) return [];
    const dx = item.x - current.x;
    const dy = item.y - current.y;
    const forward = direction === "right" ? dx : direction === "left" ? -dx : direction === "down" ? dy : -dy;
    if (forward <= 0) return [];
    const cross = direction === "left" || direction === "right" ? Math.abs(dy) : Math.abs(dx);
    if (forward < cross * 0.5) return [];
    return [{ id: item.id, score: forward + cross * 2 }];
  });
  candidates.sort((left, right) => left.score - right.score);
  return candidates[0]?.id ?? currentId;
}
