import { defineConfig } from "@playwright/test";
// The server reads the same variable, so tests can avoid an occupied port.
const origin = `http://127.0.0.1:${process.env.TOPOLOGY_WEB_PORT ?? "8080"}`;
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: origin,
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
    url: `${origin}/api/health`,
    reuseExistingServer: !process.env.CI,
    cwd: "..",
    timeout: 30000,
  },
});
