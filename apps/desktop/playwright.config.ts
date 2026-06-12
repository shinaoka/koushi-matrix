/**
 * Playwright config for the headless UI DOM tier (QA Model layer 4).
 *
 * Drives headless Chromium against the Vite-served harness page
 * (harness.html → src/test/harnessMain.tsx → real TimelineView + mock IPC).
 *
 * ABSOLUTELY NO GUI: headless only, no Tauri app, no native window. The Vite
 * dev server is started by Playwright on port 5183 (NOT the canonical 5173,
 * so a developer's running dev server is never clobbered) and is torn down
 * by Playwright when the run ends.
 */

import { defineConfig } from "@playwright/test";

const HARNESS_PORT = 5183;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${HARNESS_PORT}`,
    headless: true
  },
  webServer: {
    command: `npx vite --port ${HARNESS_PORT}`,
    url: `http://127.0.0.1:${HARNESS_PORT}/harness.html`,
    reuseExistingServer: false,
    timeout: 30_000
  }
});
