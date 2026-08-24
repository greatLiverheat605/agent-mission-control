import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { act, cleanup, render, screen } from "@testing-library/react";
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
});

afterEach(() => {
  vi.useRealTimers();
});

test("initially shows only the local supervisor connection status", () => {
  invokeMock.mockReturnValue(new Promise(() => undefined));
  render(<App />);

  expect(screen.getByRole("status").textContent).toBe(
    "Connecting to Local Supervisor",
  );
  expect(document.body.textContent?.trim()).toBe(
    "Connecting to Local Supervisor",
  );
  expect(screen.queryByRole("textbox")).toBeNull();
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

  await screen.findByText("Local Supervisor Disconnected", {}, { timeout: 1900 });
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
    minWidth: 1180,
    minHeight: 680,
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
