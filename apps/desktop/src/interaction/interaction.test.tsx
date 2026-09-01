import { useState } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EmergencyPause } from "../features/mission/EmergencyPause";
import { CommandPalette, type MissionCommand } from "./CommandPalette";
import { spatialNext } from "./spatialNavigation";

afterEach(cleanup);

const commands: MissionCommand[] = [
  { id: "mission", label: "Open mission Policy UI", kind: "mission", keywords: ["route"] , run: vi.fn() },
  { id: "evidence", label: "Open evidence ev-8", kind: "evidence", keywords: ["test"], run: vi.fn() },
  { id: "terminate", label: "Force terminate", kind: "command", keywords: ["stop"], dangerous: true, run: vi.fn() },
  { id: "disabled", label: "Deploy", kind: "command", keywords: [], enabled: false, run: vi.fn() },
];

describe("mission interaction", () => {
  it("chooses the nearest item in a spatial direction", () => {
    const items = [{ id: "a", x: 0, y: 0 }, { id: "b", x: 20, y: 2 }, { id: "c", x: 8, y: 40 }];
    expect(spatialNext("a", "right", items)).toBe("b");
    expect(spatialNext("a", "down", items)).toBe("c");
  });

  it("filters commands by capability and routes danger through confirmation", () => {
    const close = vi.fn();
    const confirm = vi.fn();
    render(<CommandPalette open commands={commands} onClose={close} onRequestConfirmation={confirm} />);
    const input = screen.getByRole("combobox", { name: "Search missions and commands" });
    expect(screen.queryByText("Deploy")).toBeNull();
    fireEvent.change(input, { target: { value: "terminate" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(confirm).toHaveBeenCalledWith(commands[2]);
    expect(commands[2].run).not.toHaveBeenCalled();
    expect(close).toHaveBeenCalledOnce();
    fireEvent.keyDown(input, { key: "Escape" });
    expect(close).toHaveBeenCalledTimes(2);
  });

  it("requests safe pause first and confirms force terminate separately", () => {
    const pause = vi.fn();
    const terminate = vi.fn();
    render(<EmergencyPause onPause={pause} onForceTerminate={terminate} />);
    fireEvent.click(screen.getByRole("button", { name: "Request safe pause" }));
    expect(pause).toHaveBeenCalledOnce();
    expect(terminate).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Safety cover" }));
    fireEvent.click(screen.getByRole("button", { name: "Force terminate agent" }));
    expect(screen.getByRole("dialog", { name: "Confirm force terminate" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Confirm force terminate" }));
    expect(terminate).toHaveBeenCalledOnce();
  });

  it("traps emergency confirmation focus and closes on Escape", async () => {
    render(<EmergencyPause onPause={() => undefined} onForceTerminate={() => undefined} />);
    fireEvent.click(screen.getByRole("button", { name: "Safety cover" }));
    fireEvent.click(screen.getByRole("button", { name: "Force terminate agent" }));
    const dialog = screen.getByRole("dialog", { name: "Confirm force terminate" });
    const cancel = screen.getByRole("button", { name: "Keep running" });
    const confirm = screen.getByRole("button", { name: "Confirm force terminate" });
    await waitFor(() => expect(document.activeElement).toBe(cancel));
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(confirm);
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Confirm force terminate" })).toBeNull();
  });

  it("wraps command palette focus in both tab directions", async () => {
    render(<CommandPalette open commands={commands} onClose={() => undefined} onRequestConfirmation={() => undefined} />);
    const input = await screen.findByRole("combobox", { name: "Search missions and commands" });
    const first = screen.getByRole("option", { name: /Open mission Policy UImission/ });
    input.focus();
    fireEvent.keyDown(input, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    input.focus();
    fireEvent.keyDown(input, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByRole("option", { name: /Force terminatecommand/ }));
  });

  it("returns focus to the palette trigger", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return <><button type="button" onClick={() => setOpen(true)}>Open palette</button><CommandPalette open={open} commands={commands} onClose={() => setOpen(false)} onRequestConfirmation={() => undefined} /></>;
    }
    render(<Harness />);
    const trigger = screen.getByRole("button", { name: "Open palette" });
    trigger.focus();
    fireEvent.click(trigger);
    const input = await screen.findByRole("combobox", { name: "Search missions and commands" });
    fireEvent.keyDown(input, { key: "Escape" });
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });
});
