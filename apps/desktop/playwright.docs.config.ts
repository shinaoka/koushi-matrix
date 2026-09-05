import { defineConfig } from "@playwright/test";

const DOCS_SCREENSHOT_PORT = 5184;

export default defineConfig({
  testDir: "./e2e-docs",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${DOCS_SCREENSHOT_PORT}`,
    headless: true,
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 2,
    locale: "en-US",
    timezoneId: "UTC",
    colorScheme: "light",
    reducedMotion: "reduce"
  },
  webServer: {
    command: `npx vite --port ${DOCS_SCREENSHOT_PORT}`,
    url: `http://127.0.0.1:${DOCS_SCREENSHOT_PORT}/appHarness.html`,
    reuseExistingServer: false,
    timeout: 30_000
  }
});
