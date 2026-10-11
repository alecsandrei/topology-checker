import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:8080",
    headless: true,
    viewport: { width: 1440, height: 1000 },
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : {},
  },
  webServer: {
    command:
      process.env.TOPOLOGY_WEB_EXECUTABLE ??
      "./target/debug/topology-checker-web",
    url: "http://127.0.0.1:8080/api/health",
    reuseExistingServer: !process.env.CI,
    cwd: "..",
    timeout: 30000,
  },
});
