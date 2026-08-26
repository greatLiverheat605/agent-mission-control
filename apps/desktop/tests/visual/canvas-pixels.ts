import { expect, type Locator } from "@playwright/test";
import { PNG } from "pngjs";

export type CanvasPixels = {
  buffer: Buffer;
  nonBackgroundRatio: number;
  cyanFeaturePixels: number;
  width: number;
  height: number;
};

export function expectCanvasSignal(pixels: CanvasPixels): void {
  expect(pixels.width).toBeGreaterThan(0);
  expect(pixels.height).toBeGreaterThan(0);
  expect(pixels.nonBackgroundRatio).toBeGreaterThan(0.003);
  expect(pixels.cyanFeaturePixels).toBeGreaterThan(20);
}

export async function analyzeCanvas(canvas: Locator): Promise<CanvasPixels> {
  const buffer = await canvas.screenshot({ animations: "allow" });
  const image = PNG.sync.read(buffer);
  const corner = pixel(image, 2, 2);
  let nonBackground = 0;
  let cyanFeaturePixels = 0;
  for (let offset = 0; offset < image.data.length; offset += 4) {
    const red = image.data[offset];
    const green = image.data[offset + 1];
    const blue = image.data[offset + 2];
    const distance = Math.abs(red - corner[0]) + Math.abs(green - corner[1]) + Math.abs(blue - corner[2]);
    if (distance > 32) nonBackground += 1;
    if (green > 105 && blue > 115 && green - red > 24 && blue - red > 28) cyanFeaturePixels += 1;
  }
  return { buffer, nonBackgroundRatio: nonBackground / (image.width * image.height), cyanFeaturePixels, width: image.width, height: image.height };
}

export function frameDifference(leftBuffer: Buffer, rightBuffer: Buffer): number {
  const left = PNG.sync.read(leftBuffer);
  const right = PNG.sync.read(rightBuffer);
  expect([right.width, right.height]).toEqual([left.width, left.height]);
  let changed = 0;
  for (let offset = 0; offset < left.data.length; offset += 4) {
    const distance = Math.abs(left.data[offset] - right.data[offset])
      + Math.abs(left.data[offset + 1] - right.data[offset + 1])
      + Math.abs(left.data[offset + 2] - right.data[offset + 2]);
    if (distance > 18) changed += 1;
  }
  return changed / (left.width * left.height);
}

function pixel(image: PNG, x: number, y: number): [number, number, number] {
  const offset = (Math.min(image.height - 1, y) * image.width + Math.min(image.width - 1, x)) * 4;
  return [image.data[offset], image.data[offset + 1], image.data[offset + 2]];
}
