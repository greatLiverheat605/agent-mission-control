import { useEffect } from "react";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import type { FlightViewModel } from "@mission-control/mission-store";
import type { NavigationCameraId } from "../shell/cockpitViews";
import { useLocale } from "../i18n/LocaleProvider";
import { AgentMarker } from "./AgentMarker";
import { FlightVehicle } from "./FlightVehicle";
import { missionCamera, missionCameraPosition } from "./camera";
import { MissionSpine } from "./MissionSpine";
import { AdaptivePerformance, type AdaptiveLevel } from "./performanceProfile";
import { RouteBranch } from "./RouteBranch";
import { SceneRuntime } from "./SceneRuntime";
import { Starfield } from "./Starfield";
import "./missionScene.css";

export function MissionScene({
  flight,
  dpr = 1.5,
  reducedMotion = false,
  particleCount = 900,
  shadows = true,
  adaptivePerformance,
  onAdaptiveLevel,
  onUnavailable,
  onStageSelect,
  cameraView = "fwd",
}: {
  flight: FlightViewModel;
  dpr?: number;
  reducedMotion?: boolean;
  particleCount?: number;
  shadows?: boolean;
  adaptivePerformance?: AdaptivePerformance;
  onAdaptiveLevel?: (level: AdaptiveLevel) => void;
  onUnavailable?: () => void;
  onStageSelect?: (stageId: string) => void;
  cameraView?: NavigationCameraId;
}) {
  const { t } = useLocale();
  const ariaLabel = t("scene.aria", { mission: flight.mission.label, state: t(`status.${flight.primaryRoute.state}`), summary: flight.currentAction.summary, decision: flight.currentAction.nextDecision });
  if (!sceneCanvasAvailable()) return <div className="mission-scene mission-scene--unavailable" role="img" aria-label={ariaLabel} data-scene-ready="false" />;
  return <div className="mission-scene" data-route-state={flight.primaryRoute.state}>
    <Canvas
      role="img"
      aria-label={ariaLabel}
      camera={missionCamera(flight.stages.length)}
      dpr={[1, Math.min(2, Math.max(1, dpr))]}
      shadows={shadows ? "basic" : false}
      gl={{ antialias: true, alpha: true, powerPreference: "high-performance" }}
      onCreated={({ gl }) => { gl.domElement.dataset.sceneReady = "true"; }}
    >
      <fog attach="fog" args={["black", 9, 30]} />
      <ambientLight intensity={0.32} />
      <directionalLight position={[3, 5, 4]} intensity={0.7} castShadow={shadows} />
      <Starfield count={particleCount} reducedMotion={reducedMotion} />
      {cameraView === "tac" ? <>
        <MissionSpine flight={flight} onStageSelect={onStageSelect} />
        {flight.derivedRoutes.map((route, index) => <RouteBranch key={route.id} route={route} index={index} stageCount={flight.stages.length} />)}
        <AgentMarker flight={flight} reducedMotion={reducedMotion} />
      </> : <FlightVehicle flight={flight} reducedMotion={reducedMotion} />}
      <CameraViewRig view={cameraView} stageCount={flight.stages.length} reducedMotion={reducedMotion} />
      {adaptivePerformance && onAdaptiveLevel && <FramePerformanceMonitor performance={adaptivePerformance} onLevel={onAdaptiveLevel} />}
      <SceneLifecycle onUnavailable={onUnavailable} />
    </Canvas>
  </div>;
}

function CameraViewRig({ view, stageCount, reducedMotion }: { view: NavigationCameraId; stageCount: number; reducedMotion: boolean }) {
  const camera = useThree((state) => state.camera);
  useEffect(() => {
    const preset = missionCameraPosition(view, stageCount);
    camera.position.set(...preset.position);
    camera.lookAt(...preset.target);
    camera.updateProjectionMatrix();
  }, [camera, reducedMotion, stageCount, view]);
  return null;
}

function FramePerformanceMonitor({ performance, onLevel }: { performance: AdaptivePerformance; onLevel: (level: AdaptiveLevel) => void }) {
  useFrame((_, delta) => {
    const previous = performance.level;
    const next = performance.record(delta * 1000);
    if (next !== previous) onLevel(next);
  });
  return null;
}

export function sceneCanvasAvailable(): boolean {
  if (typeof window === "undefined" || typeof document === "undefined" || typeof window.ResizeObserver === "undefined") return false;
  try {
    const canvas = document.createElement("canvas");
    return Boolean(canvas.getContext("webgl2"));
  } catch {
    return false;
  }
}

function SceneLifecycle({ onUnavailable }: { onUnavailable?: () => void }) {
  const gl = useThree((state) => state.gl);
  const setFrameloop = useThree((state) => state.setFrameloop);
  useEffect(() => {
    const runtime = new SceneRuntime();
    const canvas = gl.domElement;
    const contextLost = (event: Event) => {
      event.preventDefault();
      setFrameloop("never");
      onUnavailable?.();
    };
    const visibility = () => setFrameloop(document.hidden ? "never" : "always");
    canvas.addEventListener("webglcontextlost", contextLost);
    document.addEventListener("visibilitychange", visibility);
    runtime.listen(() => canvas.removeEventListener("webglcontextlost", contextLost));
    runtime.listen(() => document.removeEventListener("visibilitychange", visibility));
    return () => runtime.dispose();
  }, [gl, onUnavailable, setFrameloop]);
  return null;
}
