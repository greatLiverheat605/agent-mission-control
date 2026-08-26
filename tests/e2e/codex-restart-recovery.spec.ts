import { expect, test, type Browser, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtemp, mkdir, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const desktopExecutable = join(root, "target/e2e/debug/mission-control-desktop.exe");
const fakeCodex = join(root, "fixtures/agents/bin/fake-codex-app-server.cmd");
const processLogs = new WeakMap<ChildProcess, string>();
const debugPort = 9333;

test("Codex vertical slice survives UI disconnect and restarts from after_sequence", async () => {
  test.setTimeout(90_000);
  const profile = await mkdtemp(join(tmpdir(), "mission-control-e2e-"));
  const appData = join(profile, "appdata");
  const localAppData = join(profile, "localappdata");
  await mkdir(appData);
  await mkdir(localAppData);
  const isolationId = `mission-control-e2e-${basename(profile)}`;
  const environment = {
    ...process.env,
    APPDATA: appData,
    LOCALAPPDATA: localAppData,
    WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview2"),
    MISSION_CODEX_EXECUTABLE: fakeCodex,
    MISSION_DATA_DIR: join(profile, "mission-data"),
    MISSION_PIPE_NAME: isolationId,
    MISSION_INSTANCE_SCOPE: isolationId,
  };
  let desktop: ChildProcess | undefined;

  try {
    desktop = launchDesktop(environment);
    const firstConnection = await connectDesktop(debugPort, desktop);
    const page = firstConnection.page;
    try {
      await expect(page.locator("[data-cockpit-shell][data-connection='connected']")).toBeVisible({ timeout: 10_000 });
    } catch (error) {
      throw new Error(`${String(error)}\n${await connectionDiagnostics(profile, page, desktop, isolationId)}`);
    }
    const localeButton = page.getByRole("button", { name: /^(Switch language|切换语言)$/ });
    await expect(localeButton).toBeVisible();
    if ((await localeButton.locator("span").innerText()).trim() !== "English") await localeButton.click();
    await expect(page.locator("html")).toHaveAttribute("lang", "en-US");
    await expect(page.getByRole("heading", { name: "Initialize flight plan" })).toBeVisible();
    await page.getByLabel("Project folder").fill(root, { timeout: 10_000 });
    await page.getByLabel("Mission objective").fill("Inspect the repository without writing", { timeout: 10_000 });
    await page.getByRole("button", { name: "Review contract" }).click();
    await expect(page.getByRole("heading", { name: "Exploring" })).toBeVisible();
    const active = JSON.parse(await page.evaluate(() => localStorage.getItem("mission-control.active-mission.v1")) ?? "null");
    expect(active.lastSequence).toBeGreaterThanOrEqual(3);
    const supervisorPid = await readyPid(profile);

    await terminate(desktop);
    desktop = undefined;
    await delay(4_000);

    desktop = launchDesktop(environment);
    const secondConnection = await connectDesktop(debugPort, desktop);
    const recoveredPage = secondConnection.page;
    await expect(recoveredPage.locator("[data-cockpit-shell][data-connection='connected']")).toBeVisible();
    try {
      await expect(recoveredPage.getByRole("heading", { name: "Recovery required" })).toBeVisible({ timeout: 8_000 });
    } catch (error) {
      const diagnostics = await recoveredPage.evaluate(() => ({
        activeMission: localStorage.getItem("mission-control.active-mission.v1"),
        body: document.body.innerText,
      }));
      throw new Error(`${String(error)}\nDiagnostics: ${JSON.stringify(diagnostics)}\nDesktop: ${processLogs.get(desktop) ?? "no output"}`);
    }
    const recovery = recoveredPage.getByRole("region", { name: "Recovery required" });
    await expect(recovery.getByText("ui disconnected", { exact: true })).toBeVisible();
    await expect(recovery.getByRole("button", { name: "Reconnect" })).toBeEnabled();
    await expect(recovery.getByRole("button", { name: "Restart Agent" })).toBeDisabled();
    await expect(recovery.getByRole("button", { name: "Resume from checkpoint" })).toBeDisabled();
    await expect(recovery.getByRole("button", { name: "Discard" })).toBeEnabled();
    const recoveredSequence = recoveredPage
      .getByRole("banner", { name: "Vessel status beam" })
      .locator(".beam-cell--sequence > strong");
    expect(Number(await recoveredSequence.innerText({ timeout: 5_000 }))).toBeGreaterThan(active.lastSequence);
    expect(await readyPid(profile)).toBe(supervisorPid);
  } finally {
    if (desktop) await terminate(desktop);
    await terminateReadyProcess(profile);
    await rm(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
  }
});

function launchDesktop(environment: NodeJS.ProcessEnv): ChildProcess {
  const child = spawn(desktopExecutable, [], { cwd: root, env: environment, stdio: ["ignore", "pipe", "pipe"] });
  processLogs.set(child, "");
  for (const stream of [child.stdout, child.stderr]) {
    stream?.on("data", (chunk) => processLogs.set(child, `${processLogs.get(child) ?? ""}${String(chunk)}`));
  }
  return child;
}

async function connectDesktop(port: number, desktop: ChildProcess): Promise<{ browser: Browser; page: Page }> {
  let lastError: unknown;
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (desktop.exitCode !== null) {
      throw new Error(`Desktop exited with code ${desktop.exitCode}: ${processLogs.get(desktop) ?? "no output"}`);
    }
    try {
      const browser = await import("playwright").then(({ chromium }) => chromium.connectOverCDP(`http://127.0.0.1:${port}`));
      return { browser, page: await awaitDesktopPage(port, browser) };
    } catch (error) {
      lastError = error;
      await new Promise((resolveWait) => setTimeout(resolveWait, 100));
    }
  }
  throw lastError;
}

async function awaitDesktopPage(_port: number, browser?: Browser): Promise<Page> {
  if (!browser) throw new Error("desktop browser is required");
  for (let attempt = 0; attempt < 50; attempt += 1) {
    const page = browser.contexts().flatMap((context) => context.pages()).find((candidate) => candidate.url().includes("127.0.0.1:1420"));
    if (page) return page;
    await new Promise((resolveWait) => setTimeout(resolveWait, 100));
  }
  throw new Error("Tauri WebView page was not exposed through WebView2 CDP");
}

async function readyPid(profile: string): Promise<number> {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      const readyPath = await findReadyFile(profile);
      const ready = JSON.parse(await readFile(readyPath, "utf8"));
      if (Number.isInteger(ready.pid) && ready.pid > 0) return ready.pid;
    } catch {
      await new Promise((resolveWait) => setTimeout(resolveWait, 100));
    }
  }
  throw new Error("Supervisor ready file was not published");
}

async function terminate(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null) return;
  const pid = child.pid;
  if (!pid) throw new Error("Desktop PID is unavailable");
  const exited = new Promise<void>((resolveExit) => child.once("exit", () => resolveExit()));
  child.kill();
  if (await Promise.race([exited.then(() => true), delay(5_000).then(() => false)])) return;
  const killer = spawn("taskkill.exe", ["/PID", String(pid), "/F"], { stdio: "ignore" });
  await new Promise<void>((resolveExit, rejectExit) => {
    killer.once("error", rejectExit);
    killer.once("exit", (code) => code === 0 || child.exitCode !== null ? resolveExit() : rejectExit(new Error(`taskkill exited with code ${code}`)));
  });
  await Promise.race([exited, delay(5_000).then(() => { throw new Error(`Desktop ${pid} did not exit`); })]);
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolveWait) => setTimeout(resolveWait, milliseconds));
}

async function terminateReadyProcess(profile: string): Promise<void> {
  try {
    process.kill(await readyPid(profile));
  } catch {
    // The temporary Supervisor may already have exited.
  }
}

async function findReadyFile(directory: string): Promise<string> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      try {
        return await findReadyFile(path);
      } catch {
        continue;
      }
    }
    if (entry.name === "supervisor.ready") return path;
  }
  throw new Error("Supervisor ready file not found in temporary profile");
}

async function connectionDiagnostics(
  profile: string,
  page: Page,
  desktop: ChildProcess,
  isolationId: string,
): Promise<string> {
  const shell = await page.locator("[data-cockpit-shell]").evaluate((element) => ({
    className: element.className,
    text: element.textContent,
  })).catch((error) => ({ error: String(error) }));
  const files = await listProfileFiles(profile);
  let ready: unknown = "missing";
  try {
    ready = JSON.parse(await readFile(await findReadyFile(profile), "utf8"));
  } catch {
    // Absence is the useful diagnostic when the packaged Supervisor exits before readiness.
  }
  return `Connection diagnostics: ${JSON.stringify({
    expectedPipe: isolationId,
    expectedInstanceScope: isolationId,
    shell,
    ready,
    profileFiles: files,
    desktop: {
      pid: desktop.pid,
      exitCode: desktop.exitCode,
      signalCode: desktop.signalCode,
      output: processLogs.get(desktop) ?? "no output",
    },
  })}`;
}

async function listProfileFiles(directory: string): Promise<string[]> {
  const files: string[] = [];
  const visit = async (current: string): Promise<void> => {
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) await visit(path);
      else files.push(relative(directory, path));
    }
  };
  await visit(directory);
  return files.sort();
}
