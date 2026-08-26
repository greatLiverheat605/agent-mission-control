import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const tauriConfig = JSON.stringify({
  identifier: "com.openai.agent-mission-control.e2e",
  app: {
    windows: [{
      label: "main",
      additionalBrowserArgs: "--remote-debugging-port=9333 --remote-allow-origins=*",
    }],
  },
});
const result = spawnSync("cargo", ["build", "-p", "mission-supervisor", "-p", "mission-control-desktop"], {
  cwd: root,
  env: { ...process.env, CARGO_TARGET_DIR: resolve(root, "target/e2e"), TAURI_CONFIG: tauriConfig },
  stdio: "inherit",
});
process.exit(result.status ?? 1);
