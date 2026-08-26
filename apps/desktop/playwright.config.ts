import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/visual",
  fullyParallel: false,
  workers: 1,
  reporter: "line",
  timeout: 30_000,
  use: {
    baseURL: "http://127.0.0.1:1420",
    colorScheme: "dark",
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "npm.cmd run dev -- --host 127.0.0.1 --port 1420",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
