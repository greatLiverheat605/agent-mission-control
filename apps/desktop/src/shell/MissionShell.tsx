import { useEffect, useRef, useState, type ReactNode } from "react";
import { CockpitFrame, ResponsiveDrawer, ViewSwitcher } from "@mission-control/ui";
import { cockpitViewItems, type CockpitViewId } from "./cockpitViews";
import { useLocale } from "../i18n/LocaleProvider";
import "./MissionShell.css";

export type MissionShellProps = {
  activeView: CockpitViewId;
  onViewChange: (view: CockpitViewId) => void;
  beam: ReactNode;
  portConsole: ReactNode;
  display: ReactNode;
  starboardConsole: ReactNode;
  commandConsole: ReactNode;
  flightHelm: ReactNode;
  emergencyControl: ReactNode;
  connectionState?: "connected" | "connecting" | "disconnected";
  missionState?: string;
  routeState?: string;
  motion?: "full" | "reduced";
  renderMode?: "3d" | "fallback";
};

export function MissionShell({
  activeView,
  onViewChange,
  beam,
  portConsole,
  display,
  starboardConsole,
  commandConsole,
  flightHelm,
  emergencyControl,
  connectionState = "connected",
  missionState = "active",
  routeState = "Unknown",
  motion = "full",
  renderMode = "3d",
}: MissionShellProps) {
  const { t } = useLocale();
  const [portOpen, setPortOpen] = useState(false);
  const [starboardOpen, setStarboardOpen] = useState(false);
  const displayRoot = useRef<HTMLDivElement>(null);
  const previousView = useRef(activeView);
  const focusByView = useRef<Partial<Record<CockpitViewId, HTMLElement>>>({});

  useEffect(() => {
    if (previousView.current === activeView) return;
    previousView.current = activeView;
    queueMicrotask(() => {
      const remembered = focusByView.current[activeView];
      (remembered?.isConnected ? remembered : displayRoot.current)?.focus();
    });
  }, [activeView]);

  const switchView = (view: CockpitViewId) => {
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && displayRoot.current?.contains(focused)) focusByView.current[activeView] = focused;
    setPortOpen(false);
    setStarboardOpen(false);
    onViewChange(view);
  };

  const localizedViewItems = cockpitViewItems.map((item) => ({ ...item, label: t(`view.${item.id}`) }));
  return <CockpitFrame
    className="mission-shell"
    data-active-view={activeView}
    data-connection={connectionState}
    data-mission-state={missionState}
    data-route-state={routeState}
    data-motion={motion}
    data-render={renderMode}
  >
    <header className="mission-shell__beam beam" data-structural-beam aria-label={t("shell.beam")}>{beam}</header>
    <nav className="mission-shell__port port panel" data-console="port" aria-label={t("shell.registry")}>
      <ResponsiveDrawer id="mission-port-console" label={t("shell.registry")} side="left" open={portOpen} onOpenChange={setPortOpen}>{portConsole}</ResponsiveDrawer>
    </nav>
    <main className="mission-shell__display mfd" data-mfd aria-label={t("shell.display")}>
      <ViewSwitcher className="mission-shell__softkeys" label={t("shell.displayView")} items={localizedViewItems} value={activeView} onChange={switchView} />
      <div
        ref={displayRoot}
        className="mission-shell__view"
        role="region"
        aria-label={t(`view.${activeView}.focus`)}
        data-cockpit-view={activeView}
        tabIndex={-1}
      >{display}</div>
      <div className="mission-shell__scan" aria-hidden="true" />
      <div className="mission-shell__bolts" aria-hidden="true"><i /><i /><i /><i /></div>
    </main>
    <aside className="mission-shell__starboard starboard panel" data-console="starboard" aria-label={t("shell.taskConsole")}>
      <ResponsiveDrawer id="mission-starboard-console" label={t("shell.taskConsole")} side="right" open={starboardOpen} onOpenChange={setStarboardOpen}>{starboardConsole}</ResponsiveDrawer>
    </aside>
    <footer className="mission-shell__helm helm" data-flight-helm aria-label={t("shell.flightHelm")}>
      <section className="mission-shell__command panel" aria-label={t("shell.commandConsole")}>{commandConsole}</section>
      <section className="mission-shell__flight panel">{flightHelm}</section>
    </footer>
    <div className="mission-shell__emergency emergency panel" data-emergency-control>{emergencyControl}</div>
  </CockpitFrame>;
}
