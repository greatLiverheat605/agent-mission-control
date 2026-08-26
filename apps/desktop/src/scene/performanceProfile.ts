export type PerformanceMode = "high" | "adaptive" | "low";
export type AdaptiveLevel = Extract<PerformanceMode, "adaptive" | "low">;

export type PerformanceProfile = {
  dpr: number;
  particles: number;
  shadows: boolean;
  postProcessing: boolean;
  backgroundFps: number;
  cameraMotion: boolean;
};

const PROFILES = {
  high: { dpr: 2, particles: 1400, shadows: true, postProcessing: true, backgroundFps: 30, cameraMotion: true },
  adaptive: { dpr: 1.5, particles: 900, shadows: true, postProcessing: false, backgroundFps: 12, cameraMotion: true },
  low: { dpr: 1, particles: 280, shadows: false, postProcessing: false, backgroundFps: 5, cameraMotion: false },
} satisfies Record<PerformanceMode, PerformanceProfile>;

export function performanceProfile(mode: PerformanceMode): PerformanceProfile {
  return PROFILES[mode];
}

export class AdaptivePerformance {
  level: AdaptiveLevel = "adaptive";
  private samples: number[] = [];
  private stableWindows = 0;

  constructor(
    private readonly windowSize = 60,
    private readonly recoveryWindows = 3,
  ) {}

  record(frameTimeMs: number): AdaptiveLevel {
    if (!Number.isFinite(frameTimeMs) || frameTimeMs < 0) return this.level;
    this.samples.push(frameTimeMs);
    if (this.samples.length < this.windowSize) return this.level;

    const average = this.samples.reduce((total, sample) => total + sample, 0) / this.samples.length;
    this.samples = [];
    if (average >= 24) {
      this.level = "low";
      this.stableWindows = 0;
    } else if (this.level === "low" && average <= 16) {
      this.stableWindows += 1;
      if (this.stableWindows >= this.recoveryWindows) {
        this.level = "adaptive";
        this.stableWindows = 0;
      }
    } else if (this.level === "low") {
      this.stableWindows = 0;
    }
    return this.level;
  }
}
