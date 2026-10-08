/**
 * Playwright config for the headless UI DOM tier (QA Model layer 4).
 *
 * Drives headless Chromium against the Vite-served harness page
 * (harness.html → src/test/harnessMain.tsx → real TimelineView + mock IPC).
 * A short allow-list of engine-sensitive specs also runs in headless WebKit.
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
  // One worker, not just one test per file. Playwright still spreads FILES
  // across workers when `fullyParallel` is false, and every flake recorded in
  // docs/agents/qa-lanes.md was traced to those workers contending for the one shared Vite
  // harness server — a stale `get_snapshot` landing in another file's test.
  // Serializing removes that contention by construction and keeps CI and a
  // local run identical; the whole suite still finishes in a few minutes.
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${HARNESS_PORT}`,
    headless: true,
    // Some headless environments (e.g. machines without a GPU/driver) stall
    // Chromium's compositor unless software GL is forced. Opt-in only via
    // KOUSHI_PLAYWRIGHT_EXTRA_ARGS (Koushi-prefixed per engineering-rules) so
    // the default run stays byte-identical:
    //   KOUSHI_PLAYWRIGHT_EXTRA_ARGS="--disable-gpu --enable-unsafe-swiftshader" \
    //     npx playwright test
    launchOptions: process.env.KOUSHI_PLAYWRIGHT_EXTRA_ARGS
      ? { args: process.env.KOUSHI_PLAYWRIGHT_EXTRA_ARGS.split(" ") }
      : undefined
  },
  projects: [
    { name: "chromium", use: { browserName: "chromium" } },
    // WebKit covers the macOS WKWebView and Linux WebKitGTK engine family for
    // specs whose defect is engine-native (search-field decorations, #1145).
    // Opt a spec in by adding it here; everything else stays Chromium-only.
    {
      name: "webkit",
      use: { browserName: "webkit" },
      testMatch: [/room-list-filter-native-clear\.spec\.ts$/, /upload-preview-heic\.spec\.ts$/]
    }
  ],
  webServer: {
    command: `npx vite --port ${HARNESS_PORT}`,
    url: `http://127.0.0.1:${HARNESS_PORT}/harness.html`,
    reuseExistingServer: false,
    timeout: 30_000
  }
});
