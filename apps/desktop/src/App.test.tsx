import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import App from "./App";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

beforeEach(() => {
  cleanup();
  invokeMock.mockReset();
  window.localStorage.clear();
});

afterEach(() => {
  vi.useRealTimers();
});

test("keeps the cockpit mounted while the local supervisor is connecting", () => {
  invokeMock.mockReturnValue(new Promise(() => undefined));
  const { container } = render(<App />);

  expect(screen.getByRole("status").textContent).toBe(
    "Connecting to Local Supervisor",
  );
  expect(container.querySelector("[data-cockpit-shell]")?.getAttribute("data-connection")).toBe("connecting");
  expect(screen.getByRole("main", { name: "Cockpit multifunction display" })).toBeTruthy();
  expect(screen.getByRole("textbox", { name: "Command input" })).toBeTruthy();
  expect(screen.queryByLabelText("Mission objective")).toBeNull();
  expect(screen.queryByText(/token|data-dir|shell/i)).toBeNull();
});

test("shows the connected supervisor version returned by the native bridge", async () => {
  invokeMock.mockResolvedValue({
    connection: "connected",
    version: "0.1.0",
  });

  render(<App />);

  await screen.findByText("Connected to Local Supervisor - 0.1.0");
  expect(screen.getByRole("status").textContent).toBe(
    "Connected to Local Supervisor - 0.1.0",
  );
  expect(invokeMock).toHaveBeenCalledWith("supervisor_status");
});

test("shows disconnected within two seconds after a heartbeat fails", async () => {
  invokeMock
    .mockResolvedValueOnce({ connection: "connected", version: "0.1.0" })
    .mockResolvedValueOnce({ connection: "disconnected", version: null });

  render(<App />);
  await screen.findByText("Connected to Local Supervisor - 0.1.0");

  await waitFor(() => expect(screen.getByRole("status").textContent).toBe("Local Supervisor Disconnected"), { timeout: 1900 });
  expect(screen.getByRole("status").textContent).toBe(
    "Local Supervisor Disconnected",
  );
  expect(invokeMock).toHaveBeenCalledWith("ping_supervisor");
});

test("does not queue heartbeats while a native request is in flight", () => {
  vi.useFakeTimers();
  invokeMock.mockReturnValue(new Promise(() => undefined));

  render(<App />);
  expect(invokeMock).toHaveBeenCalledTimes(1);

  act(() => vi.advanceTimersByTime(3_000));

  expect(invokeMock).toHaveBeenCalledTimes(1);
});

test("shares the initial request across StrictMode effect replay", () => {
  invokeMock.mockReturnValue(new Promise(() => undefined));

  render(
    <StrictMode>
      <App />
    </StrictMode>,
  );

  expect(invokeMock).toHaveBeenCalledTimes(1);
});

test("desktop window and capability config keep the renderer restricted", () => {
  const tauriConfig = JSON.parse(
    readFileSync(resolve(process.cwd(), "src-tauri/tauri.conf.json"), "utf8"),
  );
  const capability = JSON.parse(
    readFileSync(
      resolve(process.cwd(), "src-tauri/capabilities/default.json"),
      "utf8",
    ),
  );
  const window = tauriConfig.app.windows[0];

  expect(window).toMatchObject({
    fullscreen: true,
    minWidth: 1024,
    minHeight: 640,
  });
  expect(tauriConfig.app.security.csp).toMatch(/script-src 'self'/);
  expect(tauriConfig.app.security.csp).not.toMatch(/script-src[^;]*https?:/);
  expect(tauriConfig.app.security.csp).not.toMatch(/127\.0\.0\.1|ws:/);
  expect(tauriConfig.app.security.csp).not.toMatch(
    /style-src[^;]*'unsafe-inline'/,
  );
  expect(tauriConfig.app.security.devCsp).toMatch(
    /style-src[^;]*'unsafe-inline'/,
  );
  expect(tauriConfig.app.security.devCsp).toMatch(
    /http:\/\/127\.0\.0\.1:1420.*ws:\/\/127\.0\.0\.1:1420/,
  );
  expect(JSON.stringify(capability.permissions)).not.toMatch(
    /shell|(^|[^a-z])fs([^a-z]|$)|http/i,
  );
});

test("mission reconnect subscribes after the last sequence without resuming", async () => {
  const missionId = "mission-reconnect";
  const routeId = "route-reconnect";
  invokeMock.mockImplementation((command: string) => {
    if (command === "supervisor_status" || command === "ping_supervisor") {
      return Promise.resolve({ connection: "connected", version: "0.1.0" });
    }
    if (command === "create_mission") {
      return Promise.resolve({
        accepted: true,
        missionId,
        routeId,
        sequence: 2,
        events: [
          { mission_id: missionId, route_id: routeId, sequence: 1, kind: "mission_created", payload: {}, source: "supervisor" },
          { mission_id: missionId, route_id: routeId, sequence: 2, kind: "route_created", payload: {}, source: "supervisor" },
        ],
      });
    }
    if (command === "launch_route" || command === "subscribe_mission") {
      return Promise.resolve({
        accepted: true,
        missionId,
        routeId,
        sequence: 3,
        events: [
          { mission_id: missionId, route_id: routeId, sequence: 3, kind: "agent_run_started", payload: {}, source: "agent" },
        ],
      });
    }
    return Promise.resolve({ accepted: true, missionId, routeId, sequence: 3, events: [] });
  });

  render(<App />);
  await screen.findByText("Connected to Local Supervisor - 0.1.0");
  fireEvent.change(screen.getByLabelText("Project folder"), { target: { value: "C:\\workspace" } });
  fireEvent.change(screen.getByLabelText("Mission objective"), { target: { value: "Inspect" } });
  fireEvent.click(screen.getByRole("button", { name: "Review contract" }));

  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith(
      "subscribe_mission",
      expect.objectContaining({ request: expect.objectContaining({ missionId }) }),
    );
  });
  const commandNames = invokeMock.mock.calls.map(([command]) => command);
  expect(commandNames).not.toContain("resume_agent");
  expect(commandNames).not.toContain("resume_route");
});

test("restart rebuilds a disconnected mission after the persisted sequence without resuming", async () => {
  window.localStorage.setItem("mission-control.active-mission.v1", JSON.stringify({
    draft: { projectRoot: "C:\\workspace", goal: "Inspect", agent: "codex" },
    missionId: "mission-restart",
    routeId: "route-restart",
    lastSequence: 3,
    phase: "Exploring",
    status: "running",
    currentAction: "Inspecting project",
    reason: null,
  }));
  invokeMock.mockImplementation((command: string, args?: { request?: { expectedVersion?: number } }) => {
    if (command === "supervisor_status" || command === "ping_supervisor") {
      return Promise.resolve({ connection: "connected", version: "0.1.0" });
    }
    if (command === "subscribe_mission") {
      expect(args?.request?.expectedVersion).toBe(3);
      return Promise.resolve({
        accepted: true,
        missionId: "mission-restart",
        routeId: "route-restart",
        sequence: 4,
        events: [{
          mission_id: "mission-restart",
          route_id: "route-restart",
          sequence: 4,
          kind: "pause_requested",
          payload: { reason: "ui disconnected" },
          source: "supervisor",
        }],
      });
    }
    throw new Error(`unexpected command: ${command}`);
  });

  render(<App />);

  const recovery = (await screen.findByRole("heading", { name: "Recovery required" })).closest("section");
  expect(recovery).not.toBeNull();
  expect(within(recovery!).getByText("ui disconnected")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Reconnect" })).toBeTruthy();
  expect((screen.getByRole("button", { name: "Restart Agent" }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole("button", { name: "Resume from checkpoint" }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole("button", { name: "Discard" })).toBeTruthy();
  expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain("resume_route");
});

test("legacy active mission locator performs a full replay", async () => {
  window.localStorage.setItem("mission-control.active-mission.v1", JSON.stringify({
    draft: { projectRoot: "C:\\workspace", goal: "Inspect", agent: "codex" },
    missionId: "mission-legacy",
    routeId: "route-legacy",
    lastSequence: 3,
  }));
  invokeMock.mockImplementation((command: string, args?: { request?: { expectedVersion?: number } }) => {
    if (command === "supervisor_status" || command === "ping_supervisor") {
      return Promise.resolve({ connection: "connected", version: "0.1.0" });
    }
    if (command === "subscribe_mission") {
      expect(args?.request?.expectedVersion).toBe(0);
      return Promise.resolve({
        accepted: true,
        missionId: "mission-legacy",
        routeId: "route-legacy",
        sequence: 4,
        events: [
          { mission_id: "mission-legacy", sequence: 1, kind: "mission_created", payload: {}, source: "supervisor" },
          { mission_id: "mission-legacy", sequence: 2, kind: "route_created", payload: {}, source: "supervisor" },
          { mission_id: "mission-legacy", sequence: 3, kind: "agent_run_started", payload: {}, source: "agent" },
          { mission_id: "mission-legacy", sequence: 4, kind: "pause_requested", payload: { reason: "ui disconnected" }, source: "supervisor" },
        ],
      });
    }
    throw new Error(`unexpected command: ${command}`);
  });

  render(<App />);

  await screen.findByRole("heading", { name: "Recovery required" });
  expect(invokeMock).toHaveBeenCalledWith(
    "subscribe_mission",
    expect.objectContaining({ request: expect.objectContaining({ expectedVersion: 0 }) }),
  );
});
