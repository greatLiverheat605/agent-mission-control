import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AdaptivePerformance, performanceProfile } from "./performanceProfile";
import { useReducedMotion } from "./useReducedMotion";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("visual performance", () => {
  it("uses explicit profiles and hysteresis for adaptive recovery", () => {
    expect(performanceProfile("high")).toMatchObject({ dpr: 2, shadows: true });
    expect(performanceProfile("low")).toMatchObject({ dpr: 1, particles: 280, cameraMotion: false });
    const adaptive = new AdaptivePerformance(4, 2);
    for (const sample of [30, 29, 28, 27]) adaptive.record(sample);
    expect(adaptive.level).toBe("low");
    for (const sample of [10, 10, 10, 10]) adaptive.record(sample);
    expect(adaptive.level).toBe("low");
    for (const sample of [10, 10, 10, 10]) adaptive.record(sample);
    expect(adaptive.level).toBe("adaptive");
  });

  it("reacts when reduced-motion changes during the session", () => {
    let listener: ((event: MediaQueryListEvent) => void) | undefined;
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, media: "", onchange: null, addListener: vi.fn(), removeListener: vi.fn(), addEventListener: (_type: string, next: EventListenerOrEventListenerObject) => { listener = next as (event: MediaQueryListEvent) => void; }, removeEventListener: vi.fn(), dispatchEvent: vi.fn() } as MediaQueryList)));
    const { result } = renderHook(() => useReducedMotion());
    expect(result.current).toBe(false);
    act(() => listener?.({ matches: true } as MediaQueryListEvent));
    expect(result.current).toBe(true);
  });
});
