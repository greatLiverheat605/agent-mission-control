import { useRef } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { EN_MESSAGES } from "./catalogs/en-US";
import { ZH_MESSAGES } from "./catalogs/zh-CN";
import { LOCALE_STORAGE_KEY, LocaleProvider, LocaleSwitcher, resolveInitialLocale, useLocale } from "./LocaleProvider";
import { MissionShell } from "../shell/MissionShell";
import { NewMission } from "../features/onboarding/NewMission";
import { CommandPalette } from "../interaction/CommandPalette";
import { ApprovalDock } from "../features/approval/ApprovalDock";
import { EmergencyPause } from "../features/mission/EmergencyPause";

afterEach(() => { cleanup(); localStorage.clear(); });

function Harness() {
  const canvas = useRef<HTMLCanvasElement>(null);
  const { t } = useLocale();
  return <><LocaleSwitcher /><span>{t("shell.display")}</span><canvas ref={canvas} data-testid="scene" /></>;
}

describe("typed cockpit i18n", () => {
  it("keeps both catalog key sets identical", () => {
    expect(Object.keys(ZH_MESSAGES).sort()).toEqual(Object.keys(EN_MESSAGES).sort());
  });

  it("fails safe for invalid storage values", () => {
    expect(resolveInitialLocale({ getItem: () => "invalid" }, "zh-CN")).toBe("zh-CN");
    expect(resolveInitialLocale({ getItem: () => "invalid" }, "fr-FR")).toBe("en-US");
  });

  it("switches visible and aria text, persists locale, and preserves Canvas identity", () => {
    render(<LocaleProvider initialLocale="en-US"><Harness /></LocaleProvider>);
    const canvas = screen.getByTestId("scene");
    fireEvent.click(screen.getByRole("button", { name: "Switch language" }));
    expect(screen.getByText("驾驶舱多功能显示器")).toBeTruthy();
    expect(screen.getByRole("button", { name: "切换语言" })).toBeTruthy();
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh-CN");
    expect(screen.getByTestId("scene")).toBe(canvas);
  });

  it("localizes shell landmarks, preflight, palette, disabled reasons, and emergency confirmation", () => {
    render(<LocaleProvider initialLocale="zh-CN">
      <MissionShell
        activeView="nav"
        onViewChange={() => undefined}
        beam={<span>beam</span>}
        portConsole={<span>port</span>}
        display={<span>display</span>}
        starboardConsole={<span>starboard</span>}
        commandConsole={<span>command</span>}
        flightHelm={<span>helm</span>}
        emergencyControl={<span>emergency</span>}
      />
      <NewMission onCreate={() => undefined} />
      <CommandPalette open commands={[]} onClose={() => undefined} onRequestConfirmation={() => undefined} />
      <ApprovalDock approvals={[{ id: "approval-1", action: "Install dependency", scope: "Single action", expiresAt: null }]} />
      <EmergencyPause disabled={false} onPause={() => undefined} onForceTerminate={() => undefined} />
    </LocaleProvider>);

    expect(screen.getByRole("main", { name: "驾驶舱多功能显示器" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "导航显示器" })).toBeTruthy();
    expect(screen.getByLabelText("任务目标")).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "指令面板" })).toBeTruthy();
    expect(screen.getAllByTitle("审批指令不可用")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "安全盖板" }));
    fireEvent.click(screen.getByRole("button", { name: "强制终止智能单元" }));
    expect(screen.getByRole("heading", { name: "确认强制终止" })).toBeTruthy();
    expect(screen.getByText(/立即停止本任务拥有的智能单元进程树/)).toBeTruthy();
  });
});
