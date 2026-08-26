import { Component, useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { toFlightViewModel, type MissionEvent, type MissionReadModel } from "@mission-control/mission-store";
import { AlertStrip, Command as CommandIcon, DockSection, Navigation as NavigationIcon, Radar, SquareTerminal, TelemetryReadout, ViewSwitcher } from "@mission-control/ui";
import { CommandPalette, type MissionCommand } from "../../interaction/CommandPalette";
import { useMissionKeyboard } from "../../interaction/keyboard";
import { MissionScene } from "../../scene/MissionScene";
import { AdaptivePerformance, performanceProfile, type AdaptiveLevel, type PerformanceMode } from "../../scene/performanceProfile";
import { SceneFallback } from "../../scene/SceneFallback";
import { sceneCanvasAvailable } from "../../scene/MissionScene";
import { useReducedMotion } from "../../scene/useReducedMotion";
import { VisualPerformance } from "../../settings/VisualPerformance";
import { MissionShell } from "../../shell/MissionShell";
import {
  COCKPIT_VIEW_IDS,
  COCKPIT_VIEWS,
  navigationCameraItems,
  type CockpitViewId,
  type NavigationCameraId,
} from "../../shell/cockpitViews";
import { ApprovalDock } from "../approval/ApprovalDock";
import { BudgetTelemetry } from "../budget/BudgetTelemetry";
import { ContractOrbit } from "../contract/ContractOrbit";
import { EvidenceBay } from "../evidence/EvidenceBay";
import { ProjectOrbit } from "../galaxy/ProjectOrbit";
import { LoadoutPanel } from "../loadout/LoadoutPanel";
import { MemoryReviewPanel, type MemoryDecision, type MemoryReviewItem } from "../memory";
import { RecallInspector, type RecallEvidence } from "../memory";
import { RecoveryReviewPanel, type RecoveryReviewManifest } from "../recovery";
import { EmergencyPause } from "./EmergencyPause";
import { EventEvidence } from "./EventEvidence";
import { LocaleSwitcher, useLocale } from "../../i18n/LocaleProvider";
import "../panels.css";

type BasicMissionFlightProps = {
  mission: MissionReadModel;
  events: MissionEvent[];
  initialView?: CockpitViewId;
  forceSceneFallback?: boolean;
  forceReducedMotion?: boolean;
  onPause?: () => void;
  onReconnect: () => void;
  onDiscard: () => void;
  onNewMission?: () => void;
  connectionState?: "connected" | "connecting" | "disconnected";
  onForceTerminate?: () => void;
  displayOverride?: ReactNode;
  recoveryPackage?: RecoveryReviewManifest | null;
  recoveryVerified?: boolean;
  recoveryBuilding?: boolean;
  onBuildRecovery?: () => void;
  onVerifyRecovery?: () => void;
  onResumeRecovery?: () => void;
  onMemoryDecision?: (id: string, decision: MemoryDecision) => void;
  onHandoff?: (provider: "codex" | "claude") => void;
};

export function BasicMissionFlight({ mission, events, initialView = "nav", forceSceneFallback = false, forceReducedMotion, onPause, onReconnect, onDiscard, onNewMission, connectionState, onForceTerminate, displayOverride, recoveryPackage = null, recoveryVerified = false, recoveryBuilding = false, onBuildRecovery, onVerifyRecovery, onResumeRecovery, onMemoryDecision, onHandoff }: BasicMissionFlightProps) {
  const { t } = useLocale();
  const recovering = mission.status === "paused" && mission.reason?.toLowerCase().includes("ui disconnected");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [activeView, setActiveView] = useState<CockpitViewId>(() => recovering ? "systems" : initialView);
  const [cameraView, setCameraView] = useState<NavigationCameraId>("fwd");
  const [performanceMode, setPerformanceMode] = useState<PerformanceMode>("adaptive");
  const [adaptiveLevel, setAdaptiveLevel] = useState<AdaptiveLevel>("adaptive");
  const [sceneUnavailable, setSceneUnavailable] = useState(() => forceSceneFallback || !sceneCanvasAvailable());
  const [adaptivePerformance] = useState(() => new AdaptivePerformance());
  const detectedReducedMotion = useReducedMotion();
  const reducedMotion = forceReducedMotion ?? detectedReducedMotion;
  const flight = toFlightViewModel({ ...mission, events });
  const memoryItems = memoryItemsFromEvents(events);
  const recallEvidence = memoryItems.filter((item) => item.status === "confirmed").map(recallEvidenceFromMemory);
  const recoveryManifest = recoveryPackage ?? placeholderRecoveryManifest(mission, flight, events);
  const profile = performanceProfile(performanceMode === "adaptive" ? adaptiveLevel : performanceMode);
  const openPalette = useCallback(() => setPaletteOpen(true), []);
  useEffect(() => { if (recovering) setActiveView("systems"); }, [recovering]);
  useMissionKeyboard(openPalette);
  const focus = useCallback((selector: string) => (document.querySelector<HTMLElement>(selector)?.focus()), []);
  const openViewAndFocus = useCallback((view: CockpitViewId, selector: string) => {
    setActiveView(view);
    requestAnimationFrame(() => focus(selector));
  }, [focus]);
  const commands = useMemo<MissionCommand[]>(() => [
    ...COCKPIT_VIEW_IDS.map((id): MissionCommand => ({ id: `view-${id}`, label: t("palette.switchView", { view: t(`view.${id}`) }), kind: "view", keywords: [id, COCKPIT_VIEWS[id].shortLabel], run: () => setActiveView(id) })),
    { id: "mission", label: t("palette.openMission", { mission: flight.mission.label }), kind: "mission", keywords: [flight.primaryRoute.id], run: () => focus("[data-stage-detail]") },
    { id: "contract", label: t("palette.openContract"), kind: "route", keywords: [flight.contract.goal], run: () => openViewAndFocus("mission", "[aria-labelledby='contract-title']") },
    { id: "evidence", label: t("palette.openEvidence"), kind: "evidence", keywords: flight.evidenceBatches.map((batch) => batch.summary), run: () => openViewAndFocus("records", "[aria-labelledby='evidence-bay-title']") },
    { id: "approval", label: t("palette.openApprovals"), kind: "approval", keywords: flight.pendingApprovals.map((approval) => approval.action), enabled: flight.pendingApprovals.length > 0, run: () => openViewAndFocus("authority", "[aria-labelledby='approval-title']") },
    { id: "pause", label: t("palette.pause"), kind: "command", keywords: ["stop", "wait"], enabled: Boolean(onPause) && mission.status !== "paused" && mission.status !== "completed", run: onPause ?? noop },
  ], [flight, focus, mission.status, onPause, openViewAndFocus, t]);

  const resolvedConnection = connectionState ?? (recovering ? "disconnected" : "connected");
  const navigation = <div className="navigation-display nav-display" data-camera-view={cameraView}>
    <div className="navigation-camera-bar camera-bar"><span>{t(`camera.${cameraView}`)}</span><ViewSwitcher label={t("camera.label")} items={navigationCameraItems} value={cameraView} onChange={setCameraView} /></div>
    {sceneUnavailable
      ? <SceneFallback flight={flight} onStageSelect={() => focus("[data-stage-detail]")} />
      : <SceneFailureBoundary onFailure={() => setSceneUnavailable(true)}><MissionScene
        flight={flight}
        cameraView={cameraView}
        dpr={profile.dpr}
        particleCount={profile.particles}
        shadows={profile.shadows}
        reducedMotion={reducedMotion}
        adaptivePerformance={performanceMode === "adaptive" ? adaptivePerformance : undefined}
        onAdaptiveLevel={setAdaptiveLevel}
        onUnavailable={() => setSceneUnavailable(true)}
        onStageSelect={() => focus("[data-stage-detail]")}
      /></SceneFailureBoundary>}
    <div className="flight-instrument-overlay" aria-hidden="true">
      <div className="heading-tape"><span>330</span><span>345</span><strong>000</strong><span>015</span><span>030</span><i /></div>
      <div className="flight-reticle"><i /><i /><b /></div>
      <div className="flight-deck-grid" />
    </div>
    <div className="navigation-hud hud-tl"><span>{t("view.nav")}</span><strong>{t(`camera.${cameraView}`)}</strong><small>{flight.primaryRoute.id}</small></div>
    <div className="navigation-hud hud-tr"><span>{t("beam.state")}</span><strong>{t(`status.${flight.primaryRoute.state}`)}</strong><small>{flight.renderConfidence}</small></div>
    <div className="navigation-hud hud-bl"><span>{t("console.currentWaypoint")}</span><strong>{Math.max(1, flight.agentPosition.stageIndex + 1).toString().padStart(2, "0")} / {flight.stages.length.toString().padStart(2, "0")}</strong></div>
    <div className="navigation-hud hud-br"><span>{t("console.evidenceBatches")}</span><strong>{flight.evidenceBatches.length.toString().padStart(3, "0")}</strong><small>{flight.pendingApprovals.length} {t("panel.approvals")}</small></div>
    <section className="scene-status-readout" data-stage-detail tabIndex={-1} aria-label={t("scene.selectedStage")}><span className="panel-kicker">{t(`status.${flight.primaryRoute.state}`)} · {t("beam.sequence")} {mission.lastSequence}</span><h1>{mission.phase}</h1><p>{flight.currentAction.summary}</p>{flight.derivedRoutes.length > 0 && <ul className="scene-route-summary">{flight.derivedRoutes.map((route) => <li key={route.id}>{t("scene.routeSummary", { state: t(`status.${route.state}`), route: route.id })}</li>)}</ul>}<button type="button" className="scene-palette-trigger icon-command" aria-label={t("palette.open")} title={t("palette.open")} onClick={openPalette}><CommandIcon aria-hidden="true" size={18} /></button></section>
  </div>;
  const projectMissions = flight.projectMissions ?? [{ id: flight.mission.id, label: flight.mission.label, routeState: flight.primaryRoute.state, action: flight.currentAction.summary }];
  const viewDisplays = {
    nav: navigation,
    sector: <CockpitViewFrame view="sector"><ProjectOrbit missions={projectMissions} selectedId={flight.mission.id} /></CockpitViewFrame>,
    mission: <CockpitViewFrame view="mission"><div className="cockpit-view-grid"><ContractOrbit flight={flight} /><BudgetTelemetry budget={flight.budget} /></div></CockpitViewFrame>,
    records: <CockpitViewFrame view="records"><EvidenceBay batches={flight.evidenceBatches} /></CockpitViewFrame>,
    systems: <CockpitViewFrame view="systems"><div className="cockpit-view-grid"><LoadoutPanel loadout={flight.loadout} /><section className="orbit-panel systems-console" aria-labelledby="systems-console-title"><header className="panel-heading"><span className="panel-kicker">{t("systems.vessel")}</span><h2 id="systems-console-title">{t("systems.renderRecovery")}</h2></header><VisualPerformance value={performanceMode} fallback={sceneUnavailable} reducedMotion={reducedMotion} onChange={(mode) => { setPerformanceMode(mode); if (mode === "adaptive") setAdaptiveLevel(adaptivePerformance.level); }} />{recovering ? <RecoveryActions reason={mission.reason} onReconnect={onReconnect} onDiscard={onDiscard} /> : <AlertStrip title={t("systems.recovery")} tone="verified">{t("systems.noRecovery")}</AlertStrip>}</section><MemoryReviewPanel items={memoryItems} onDecision={onMemoryDecision} /><RecallInspector evidence={recallEvidence} /><RecoveryReviewPanel manifest={recoveryManifest} verified={recoveryVerified} onBuild={onBuildRecovery} building={recoveryBuilding} onVerify={onVerifyRecovery} onResume={onResumeRecovery} onDiscard={recovering ? undefined : onDiscard} /></div></CockpitViewFrame>,
    authority: <CockpitViewFrame view="authority"><div className="cockpit-view-grid"><ContractOrbit flight={flight} /><ProviderHandoffPanel currentProvider={flight.loadout.provider} onHandoff={onHandoff} />{flight.pendingApprovals.length ? <ApprovalDock approvals={flight.pendingApprovals} /> : <section className="orbit-panel"><AlertStrip title={t("authority.clear")} tone="verified">{t("authority.none")}</AlertStrip></section>}</div></CockpitViewFrame>,
  } satisfies Record<CockpitViewId, ReactNode>;
  const effectiveView = recovering ? "systems" : activeView;
  const activeDisplay = displayOverride ?? viewDisplays[effectiveView];
  const contextBudget = flight.budget.find((item) => item.dimension.toLowerCase().includes("context"));
  const contextUsage = contextBudget?.used != null && contextBudget.limit ? Math.round(contextBudget.used / contextBudget.limit * 100) : null;

  return <MissionShell
    activeView={effectiveView}
    onViewChange={setActiveView}
    beam={<VesselBeam mission={flight.mission.label} route={flight.primaryRoute.id} routeState={flight.primaryRoute.state} state={t(`status.${flight.primaryRoute.state}`)} phase={mission.phase} sequence={mission.lastSequence} contextUsage={contextUsage} connectionState={resolvedConnection} />}
    portConsole={<MissionRegistry missions={projectMissions} selectedId={flight.mission.id} onNewMission={onNewMission} />}
    display={<>{activeDisplay}<CommandPalette open={paletteOpen} commands={commands} onClose={() => setPaletteOpen(false)} onRequestConfirmation={() => undefined} /></>}
    starboardConsole={<TaskConsole mission={mission} flight={flight} />}
    commandConsole={<CommandConsole missionId={flight.mission.id} commands={commands} onViewChange={setActiveView} onCameraChange={(camera) => { setCameraView(camera); setActiveView("nav"); }} onOpenPalette={openPalette} onShowStatus={() => { setActiveView("nav"); requestAnimationFrame(() => focus("[data-stage-detail]")); }} />}
    flightHelm={<FlightHelm flight={flight} paused={!onPause || mission.status === "paused" || mission.status === "completed"} onViewChange={setActiveView} onPause={onPause ?? noop} onOpenPalette={openPalette} />}
    emergencyControl={<EmergencyPause disabled={!onPause || mission.status === "paused" || mission.status === "completed"} showPause={false} onPause={onPause ?? noop} onForceTerminate={onForceTerminate} />}
    connectionState={resolvedConnection}
    missionState={mission.status}
    routeState={flight.primaryRoute.state}
    motion={reducedMotion ? "reduced" : "full"}
    renderMode={sceneUnavailable ? "fallback" : "3d"}
  />;
}

function noop() {}

function VesselBeam({ mission, route, routeState, state, phase, sequence, contextUsage, connectionState }: { mission: string; route: string; routeState: string; state: string; phase: string; sequence: number; contextUsage: number | null; connectionState: "connected" | "connecting" | "disconnected" }) {
  const { t } = useLocale();
  const [clock, setClock] = useState(() => utcClock());
  useEffect(() => {
    const timer = window.setInterval(() => setClock(utcClock()), 1_000);
    return () => window.clearInterval(timer);
  }, []);
  const progress = Math.min(100, Math.max(4, sequence * 8));
  const phaseTone = routeState === "Completed" ? "done" : routeState === "Blocked" || routeState === "Abandoned" ? "danger" : routeState === "Paused" || routeState.includes("Awaiting") ? "warning" : "active";
  const connectionLabel = connectionState === "connected" ? t("app.connected") : connectionState === "connecting" ? t("app.connecting") : t("app.disconnected");
  return <div className="vessel-beam">
    <div className="beam-section beam-section--identity">
      <div className="vessel-brand"><NavigationIcon aria-hidden="true" size={28} /><div><strong>{t("beam.vesselName")}</strong><span>{t("beam.control")}</span></div></div>
      <i className="beam-divider" aria-hidden="true" />
      <div className="beam-cell"><span>{t("beam.mission")}</span><strong title={mission}>{mission}</strong></div>
      <div className="beam-cell beam-cell--route"><span>{t("beam.route")}</span><strong title={route}>{route}</strong></div>
    </div>
    <div className="beam-section beam-section--flight">
      <div className="phase-chip" data-tone={phaseTone}><i className="status-led" aria-hidden="true" /><strong title={state}>{state}</strong></div>
      <div className="beam-cell beam-cell--phase"><span>{t("beam.phase")}</span><strong title={phase}>{phase}</strong></div>
      <div className="beam-progress"><span>{t("beam.sequence")}</span><div><i style={{ width: `${progress}%` }} /></div><strong>{progress}%</strong></div>
    </div>
    <div className="beam-section beam-section--systems">
      {contextUsage != null && <div className="beam-context"><span>{t("beam.context")}</span><div><i style={{ width: `${Math.min(100, contextUsage)}%` }} /></div><strong>{contextUsage}%</strong></div>}
      <div className="beam-cell beam-cell--sequence"><span>{t("beam.sequence")}</span><strong>{sequence.toString().padStart(4, "0")}</strong></div>
      <i className="beam-divider" aria-hidden="true" />
      <div className="beam-clock"><span>{t("beam.phase")}</span><strong>{clock}</strong></div>
      <div className="supervisor-pill"><i className="status-led" aria-hidden="true" /><span>{connectionLabel}</span></div>
      <LocaleSwitcher className="vessel-language" />
    </div>
  </div>;
}

function utcClock() {
  return `${new Date().toISOString().slice(11, 19)}Z`;
}

function MissionRegistry({ missions, selectedId, onNewMission }: { missions: NonNullable<ReturnType<typeof toFlightViewModel>["projectMissions"]>; selectedId: string; onNewMission?: () => void }) {
  const { t } = useLocale();
  return <div className="mission-registry">
    <header className="panel-heading"><h2>{t("panel.projectOrbit")}</h2><span className="panel-meta">{missions.length.toString().padStart(2, "0")}</span></header>
    <div className="radar-scope" role="img" aria-label={t("shell.registry")}><Radar aria-hidden="true" size={18} /><i /><i /><i />{missions.slice(0, 6).map((mission, index) => <span key={mission.id} data-contact={index} data-active={mission.id === selectedId} />)}<small>{t("view.sector")} · 12.4 AU</small></div>
    <ProjectOrbit missions={missions} selectedId={selectedId} />
    <div className="new-mission-control"><button type="button" disabled={!onNewMission} title={!onNewMission ? t("panel.approvalUnavailable") : undefined} onClick={onNewMission}><span><SquareTerminal aria-hidden="true" size={18} /><strong>{t("new.title")}</strong></span></button></div>
  </div>;
}

function CommandConsole({ missionId, commands, onViewChange, onCameraChange, onOpenPalette, onShowStatus }: { missionId: string; commands: MissionCommand[]; onViewChange: (view: CockpitViewId) => void; onCameraChange: (camera: NavigationCameraId) => void; onOpenPalette: () => void; onShowStatus: () => void }) {
  const { t } = useLocale();
  const [value, setValue] = useState("");
  const run = () => {
    const command = value.trim().replace(/^\//, "").toLowerCase();
    setValue("");
    if (!command) return;
    if (COCKPIT_VIEW_IDS.includes(command as CockpitViewId)) onViewChange(command as CockpitViewId);
    else if (navigationCameraItems.some((item) => item.id === command)) onCameraChange(command as NavigationCameraId);
    else if (command === "status") onShowStatus();
    else if (command === "hold" || command === "pause") {
      const pause = commands.find((item) => item.id === "pause");
      if (pause?.enabled !== false) pause?.run();
      else onOpenPalette();
    } else onOpenPalette();
  };
  return <div className="helm-command-console">
    <span className="helm-slab">{t("shell.commandConsole")}</span>
    <form className="helm-command-line" onSubmit={(event) => { event.preventDefault(); run(); }}>
      <strong>{missionId} <i>▸</i></strong>
      <input value={value} onChange={(event) => setValue(event.target.value)} placeholder={t("helm.commandPlaceholder")} aria-label={t("shell.commandConsole")} autoComplete="off" spellCheck={false} />
    </form>
    <div className="helm-command-hints"><button type="button" onClick={onOpenPalette}>F1 {t("palette.open")}</button>{["/status", "/hold", "/records"].map((hint) => <button key={hint} type="button" onClick={() => setValue(hint)}>{hint}</button>)}</div>
  </div>;
}

function CockpitViewFrame({ view, children }: { view: Exclude<CockpitViewId, "nav">; children: ReactNode }) {
  const { t } = useLocale();
  const definition = COCKPIT_VIEWS[view];
  const Icon = definition.icon;
  return <section className="cockpit-view-frame view-frame" data-view-id={view} aria-labelledby={`${view}-view-title`}><header className="cockpit-view-header view-header"><Icon aria-hidden="true" size={18} /><div><span className="panel-kicker">{t("view.mfd")}</span><h1 id={`${view}-view-title`}>{t(`view.${view}`)}</h1><p>{t(`view.${view}.summary`)}</p></div></header><div className="cockpit-view-content view-body">{children}</div></section>;
}

function TaskConsole({ mission, flight }: { mission: MissionReadModel; flight: ReturnType<typeof toFlightViewModel> }) {
  const { t } = useLocale();
  const stage = flight.stages[Math.max(0, flight.agentPosition.stageIndex)];
  return <DockSection className="task-console" title={t("shell.taskConsole")} meta={flight.primaryRoute.id}>
    <div className="task-console-readouts">
      <TelemetryReadout label={t("console.currentWaypoint")} value={stage ? t(`status.${stage.routeState}`) : t("console.unresolved")} detail={mission.phase} tone="active" />
      <TelemetryReadout label={t("console.agentOperation")} value={flight.currentAction.summary} detail={flight.currentAction.explanation} />
      <TelemetryReadout label={t("console.evidenceBatches")} value={flight.evidenceBatches.length} detail={t("console.approvalsPending", { count: flight.pendingApprovals.length })} tone={flight.pendingApprovals.length ? "warning" : "verified"} />
      <TelemetryReadout label={t("console.renderConfidence")} value={flight.renderConfidence} tone={flight.renderConfidence === "trusted" ? "verified" : "degraded"} />
    </div>
    <EventEvidence events={mission.events.slice(-12).reverse()} />
  </DockSection>;
}

function FlightHelm({ flight, paused, onViewChange, onPause, onOpenPalette }: { flight: ReturnType<typeof toFlightViewModel>; paused: boolean; onViewChange: (view: CockpitViewId) => void; onPause: () => void; onOpenPalette: () => void }) {
  const { t } = useLocale();
  return <div className="flight-helm-console">
    <div className="flight-helm-heading"><span>{t("helm.routeStages")}</span><strong>{flight.primaryRoute.id}</strong><small>{t(`status.${flight.primaryRoute.state}`)}</small></div>
    <div className="flight-helm-route stage-track" aria-label={t("helm.routeStages")}>{flight.stages.map((stage, index) => <button key={stage.id} type="button" data-stage-state={stage.state} aria-current={stage.state === "current" ? "step" : undefined} onClick={() => onViewChange("nav")}><span>{String(index + 1).padStart(2, "0")}</span><strong>{t(`status.${stage.routeState}`)}</strong><small>{t(`stage.${stage.state}`)}</small></button>)}</div>
    <div className="flight-helm-commands commands" aria-label={t("helm.commands")}>
      <button type="button" className="primary-command" onClick={() => onViewChange("nav")}><span><strong>{t("palette.openMission", { mission: flight.mission.label })}</strong><small>{flight.currentAction.nextDecision}</small></span></button>
      <button type="button" onClick={() => onViewChange("records")}><span><strong>{t("helm.openRecords")}</strong><small>04</small></span></button>
      <button type="button" onClick={() => onViewChange("systems")}><span><strong>{t("helm.systems")}</strong><small>05</small></span></button>
      <button type="button" onClick={() => onViewChange("authority")}><span><strong>{t("helm.authority")}</strong><small>06</small></span></button>
      <button type="button" onClick={onOpenPalette}><span><strong><CommandIcon aria-hidden="true" size={16} />{t("helm.commandPalette")}</strong><small>CTRL+K</small></span></button>
      <button type="button" className="hold-command" disabled={paused} onClick={onPause}><span><strong>{t("helm.hold")}</strong><small>{t("emergency.pauseAria")}</small></span></button>
    </div>
  </div>;
}

class SceneFailureBoundary extends Component<{ children: ReactNode; onFailure: () => void }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch() {
    this.props.onFailure();
  }

  render() {
    return this.state.failed ? null : this.props.children;
  }
}

function RecoveryActions({ reason, onReconnect, onDiscard }: { reason: string | null; onReconnect: () => void; onDiscard: () => void }) {
  const { t } = useLocale();
  return <section className="recovery-actions" aria-labelledby="recovery-title">
    <h2 id="recovery-title">{t("recovery.title")}</h2>
    {reason && <p>{reason}</p>}
    <div><button type="button" onClick={onReconnect}>{t("recovery.reconnect")}</button><button type="button" disabled>{t("recovery.restart")}</button><button type="button" disabled>{t("recovery.resume")}</button><button type="button" onClick={onDiscard}>{t("recovery.discard")}</button></div>
  </section>;
}

function ProviderHandoffPanel({ currentProvider, onHandoff }: { currentProvider: string; onHandoff?: (provider: "codex" | "claude") => void }) {
  const { t } = useLocale();
  const normalized = currentProvider.toLowerCase();
  const initialTarget: "codex" | "claude" = normalized === "claude" ? "codex" : "claude";
  const [target, setTarget] = useState<"codex" | "claude">(initialTarget);
  const [confirming, setConfirming] = useState(false);
  const submit = () => {
    if (!onHandoff) return;
    if (!confirming) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    onHandoff(target);
  };
  return <section className="orbit-panel continuity-panel handoff-panel" aria-labelledby="handoff-title">
    <header className="panel-heading"><span className="panel-kicker">{t("handoff.kicker")}</span><h2 id="handoff-title">{t("handoff.title")}</h2></header>
    <p className="continuity-notice">{t("handoff.note")}</p>
    <label className="handoff-target">{t("handoff.target")}<select value={target} onChange={(event) => { setTarget(event.target.value as "codex" | "claude"); setConfirming(false); }} disabled={!onHandoff}><option value="codex">Codex</option><option value="claude">Claude</option></select></label>
    <div className="continuity-actions"><button type="button" disabled={!onHandoff || target === normalized} onClick={submit}>{confirming ? t("handoff.confirm") : t("handoff.prepare")}</button>{confirming && <button type="button" onClick={() => setConfirming(false)}>{t("handoff.cancel")}</button>}</div>
  </section>;
}

function memoryItemsFromEvents(events: MissionEvent[]): MemoryReviewItem[] {
  const latest = new Map<string, MemoryReviewItem>();
  for (const event of events) {
    if (event.kind !== "memory_item_changed") continue;
    const item = objectValue(event.payload.item);
    const id = stringValue(item.id) ?? `memory-${event.sequence}`;
    latest.set(id, {
      id,
      kind: stringValue(item.kind) ?? "fact",
      content: stringValue(item.content) ?? stringValue(event.payload.summary) ?? event.kind,
      sourceEventIds: stringArray(item.source_event_ids ?? item.sourceEventIds, [String(event.sequence)]),
      scope: stringValue(item.scope) ?? "route",
      freshness: stringValue(item.freshness) ?? "unknown",
      version: numberValue(item.version) ?? 1,
      status: memoryStatus(item.status),
      author: stringValue(item.author) ?? event.source ?? "system",
    });
  }
  return [...latest.values()].sort((left, right) => left.id.localeCompare(right.id));
}

function recallEvidenceFromMemory(item: MemoryReviewItem): RecallEvidence {
  return {
    id: item.id,
    content: item.content,
    sourceEventIds: item.sourceEventIds,
    scope: item.scope,
    freshness: item.freshness,
    version: item.version,
  };
}

function placeholderRecoveryManifest(mission: MissionReadModel, flight: ReturnType<typeof toFlightViewModel>, events: MissionEvent[]): RecoveryReviewManifest {
  const sequence = mission.lastSequence;
  const latest = (...kinds: string[]) => [...events].reverse().find((event) => kinds.includes(event.kind))?.payload ?? {};
  const loadout = latest("loadout_snapshot");
  const context = latest("context_pack_built");
  const checkpoint = latest("checkpoint_created");
  const contract = latest("contract_updated", "mission_created");
  return {
    missionId: mission.missionId,
    routeId: flight.primaryRoute.id,
    schemaVersion: 1,
    contractVersion: numberValue(contract.contract_version ?? contract.contractVersion) ?? flight.contract.version,
    checkpointId: stringValue(checkpoint.checkpoint_id ?? checkpoint.checkpointId) ?? `checkpoint-${sequence}`,
    ledgerSequence: sequence,
    loadoutFingerprint: stringValue(loadout.fingerprint) ?? flight.loadout.change?.next ?? "not-built",
    contextPackHash: stringValue(context.hash) ?? "not-built",
    pendingApprovalHash: null,
    entryHash: "not-built",
  };
}

function memoryStatus(value: unknown): MemoryReviewItem["status"] {
  if (value === "confirmed") return "confirmed";
  if (value === "rejected") return "rejected";
  if (value === "deferred") return "deferred";
  return "pending";
}

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function stringArray(value: unknown, fallback: string[] = []): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string" && item.trim().length > 0) : fallback;
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
