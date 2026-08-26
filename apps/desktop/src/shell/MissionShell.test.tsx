import { useState } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { COCKPIT_VIEW_IDS, NAVIGATION_CAMERA_IDS, type CockpitViewId } from "./cockpitViews";
import { MissionShell } from "./MissionShell";

afterEach(cleanup);

function Harness() {
  const [view, setView] = useState<CockpitViewId>("nav");
  return <MissionShell
    activeView={view}
    onViewChange={setView}
    beam={<span>Vessel nominal</span>}
    portConsole={<span>Mission registry content</span>}
    display={view === "nav" ? <canvas aria-label="Flight scene" /> : <button type="button">{view} action</button>}
    starboardConsole={<span>Task console content</span>}
    commandConsole={<label>Command input<input /></label>}
    flightHelm={<span>Route helm</span>}
    emergencyControl={<button type="button">Pause mission</button>}
  />;
}

describe("MissionShell", () => {
  it("registers all views and navigation cameras exhaustively", () => {
    expect(COCKPIT_VIEW_IDS).toEqual(["nav", "sector", "mission", "records", "systems", "authority"]);
    expect(NAVIGATION_CAMERA_IDS).toEqual(["fwd", "trk", "tac", "aft"]);
  });

  it("keeps cockpit landmarks and emergency control reachable", () => {
    render(<Harness />);
    expect(screen.getByRole("banner", { name: "Vessel status beam" })).toBeTruthy();
    expect(screen.getByRole("navigation", { name: "Mission registry" })).toBeTruthy();
    expect(screen.getByRole("main", { name: "Cockpit multifunction display" })).toBeTruthy();
    expect(screen.getByRole("complementary", { name: "Task console" })).toBeTruthy();
    expect(screen.getByRole("contentinfo", { name: "Flight helm" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Command input" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Pause mission" })).toBeTruthy();
  });

  it("switches views with pointer and arrow keys, restores display focus, and never duplicates Canvas", async () => {
    const { container } = render(<Harness />);
    expect(container.querySelectorAll("canvas")).toHaveLength(1);
    fireEvent.click(screen.getByRole("tab", { name: /Sector/ }));
    expect(screen.getByRole("region", { name: "Sector display" })).toBeTruthy();
    expect(container.querySelectorAll("canvas")).toHaveLength(0);
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("region", { name: "Sector display" })));
    const sectorTab = screen.getByRole("tab", { name: /Sector/ });
    sectorTab.focus();
    fireEvent.keyDown(sectorTab, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: /Mission/, selected: true })).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /Navigation/ }));
    expect(container.querySelectorAll("canvas")).toHaveLength(1);
  });

  it("places the emergency layer last in DOM order", () => {
    const { container } = render(<Harness />);
    expect(container.firstElementChild?.lastElementChild).toBe(screen.getByRole("button", { name: "Pause mission" }).parentElement);
  });
});
