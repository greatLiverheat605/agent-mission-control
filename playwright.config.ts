import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: false,
  workers: 1,
  reporter: "line",
  webServer: {
    command: "npm.cmd run dev --workspace @mission-control/desktop -- --host 127.0.0.1 --port 1420",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
