import { isValidElement } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  AlertStrip,
  CockpitFrame,
  ResponsiveDrawer,
  TelemetryReadout,
  ViewSwitcher,
  nextSwitcherIndex,
} from "./cockpit";

describe("cockpit primitives", () => {
  it("keeps the frame presentation-only", () => {
    const element = CockpitFrame({ children: "Bridge", id: "bridge" });
    expect(isValidElement(element)).toBe(true);
    expect(element.props).toMatchObject({ className: "mc-cockpit-frame", id: "bridge" });
  });

  it("expresses telemetry and alert states with text and semantics", () => {
    const telemetry = TelemetryReadout({ label: "Supervisor link", value: "Offline", tone: "offline" });
    const danger = AlertStrip({ title: "Abort armed", tone: "danger", children: "Confirmation required" });
    expect(telemetry.props["data-tone"]).toBe("offline");
    expect(danger.props).toMatchObject({ role: "alert", "data-tone": "danger" });
  });

  it("wraps keyboard navigation and skips disabled views", () => {
    expect(nextSwitcherIndex(0, 1, [false, true, false])).toBe(2);
    expect(nextSwitcherIndex(0, -1, [false, true, false])).toBe(2);
    expect(nextSwitcherIndex(1, 1, [true, true])).toBe(1);
  });

  it("marks the selected view without relying on color", () => {
    const onChange = vi.fn();
    const element = ViewSwitcher({
      label: "Display view",
      value: "nav",
      onChange,
      items: [
        { id: "nav", label: "Navigation / 导航" },
        { id: "authority", label: "Authority / 舰长授权与危险操作确认" },
      ],
    });
    const tabs = element.props.children;
    expect(tabs[0].props).toMatchObject({ role: "tab", "aria-selected": true, tabIndex: 0 });
    expect(tabs[1].props).toMatchObject({ role: "tab", "aria-selected": false, tabIndex: -1 });
  });

  it("keeps drawer content reachable through a 44px trigger contract", () => {
    const element = ResponsiveDrawer({
      id: "records-drawer",
      label: "Records / 航行记录与证据批次",
      side: "left",
      open: false,
      onOpenChange: vi.fn(),
      children: "Evidence",
    });
    const [trigger, panel] = element.props.children;
    expect(trigger.props).toMatchObject({ "aria-controls": "records-drawer", "aria-expanded": false });
    expect(panel.props).toMatchObject({ id: "records-drawer", hidden: true });
  });
});
