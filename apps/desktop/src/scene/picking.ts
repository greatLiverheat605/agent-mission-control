export type PointerRect = { left: number; top: number; width: number; height: number };

export function toNdc(clientX: number, clientY: number, rect: PointerRect): { x: number; y: number } {
  return {
    x: ((clientX - rect.left) / rect.width) * 2 - 1,
    y: -((clientY - rect.top) / rect.height) * 2 + 1,
  };
}
