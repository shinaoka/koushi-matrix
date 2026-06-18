/**
 * Headless spec: Search history (crawler) settings UI (#77).
 *
 * Proves that:
 *  1. The "Search history" section is reachable from User Settings.
 *  2. Changing the speed segmented control dispatches typed update_settings.
 *  3. The UI reflects the new speed only AFTER the Rust-shaped snapshot
 *     updates — not as local React state.
 *  4. Toggling "Index media captions" dispatches the correct patch.
 *  5. Toggling "Index file names" dispatches the correct patch.
 *  6. A room in "failed" state shows the coarse kind label, not raw SDK errors.
 *  7. A room in "running" state shows the Stop button; "idle" shows Start.
 *  8. React does not apply the setting immediately — it waits for the snapshot.
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";
import type { DesktopSnapshot } from "../src/domain/types";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";

async function gotoReadyShell(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

async function openUserSettings(page: import("@playwright/test").Page): Promise<void> {
  await page.getByRole("button", { name: t("workspace.userSettings") }).click();
  // Wait for the settings panel to be open and the search history section to be present
  await expect(page.getByRole("region", { name: t("settings.searchHistory") })).toBeVisible();
}

/** Count harness invocations for a command. */
async function invocationCount(
  page: import("@playwright/test").Page,
  command: string
): Promise<number> {
  return page.evaluate(
    (cmd) =>
      (
        window as unknown as {
          __harness: { invocationsOf(c: string): unknown[] };
        }
      ).__harness.invocationsOf(cmd).length,
    command
  );
}

/** Read the args of the latest update_settings call. */
async function latestUpdateSettingsArgs(
  page: import("@playwright/test").Page
): Promise<unknown> {
  return page.evaluate(
    () =>
      (
        window as unknown as {
          __harness: { invocationsOf(c: string): { args: unknown }[] };
        }
      ).__harness.invocationsOf("update_settings").at(-1)?.args
  );
}

// ─────────────────────────────────────────────────────────────
//  Section navigation
// ─────────────────────────────────────────────────────────────

test("Search history nav item opens the section in User Settings", async ({ page }) => {
  await gotoReadyShell(page);
  await openUserSettings(page);

  const section = page.getByRole("region", { name: t("settings.searchHistory") });
  await expect(section).toBeVisible();

  // Speed control group should be visible
  await expect(
    section.getByRole("group", { name: t("settings.searchHistorySpeed") })
  ).toBeVisible();
});

// ─────────────────────────────────────────────────────────────
//  Speed segmented control
// ─────────────────────────────────────────────────────────────

test("selecting Fast speed dispatches update_settings with search_crawler.speed=fast", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() =>
    (
      window as unknown as { __harness: { clearInvocations(): void } }
    ).__harness.clearInvocations()
  );
  await openUserSettings(page);

  const speedGroup = page.getByRole("group", { name: t("settings.searchHistorySpeed") });
  const fastButton = speedGroup.getByRole("button", {
    name: t("settings.searchHistorySpeedFast")
  });
  await expect(fastButton).toBeVisible();
  await fastButton.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);

  const args = await latestUpdateSettingsArgs(page);
  expect(args).toEqual({
    patch: {
      search_crawler: {
        speed: "fast",
        include_media_captions: true,
        include_filenames: true
      }
    }
  });
});

test("speed button shows aria-pressed=true for the active speed from the Rust snapshot", async ({
  page
}) => {
  await gotoReadyShell(page);
  await openUserSettings(page);

  // Default snapshot has speed: "standard"
  const speedGroup = page.getByRole("group", { name: t("settings.searchHistorySpeed") });
  const standardButton = speedGroup.getByRole("button", {
    name: t("settings.searchHistorySpeedStandard")
  });
  await expect(standardButton).toHaveAttribute("aria-pressed", "true");

  const fastButton = speedGroup.getByRole("button", {
    name: t("settings.searchHistorySpeedFast")
  });
  await expect(fastButton).toHaveAttribute("aria-pressed", "false");
});

test("speed button pressed state updates only after the Rust snapshot changes", async ({
  page
}) => {
  await gotoReadyShell(page);

  // Override update_settings to return the *current* snapshot without applying the patch,
  // simulating a Rust round-trip that hasn't settled yet.
  await page.evaluate(() => {
    (
      window as unknown as {
        __harness: {
          setCommandResponse(cmd: string, handler: () => unknown): void;
          currentSnapshot(): unknown;
        };
      }
    ).__harness.setCommandResponse("update_settings", () =>
      (
        window as unknown as {
          __harness: { currentSnapshot(): unknown };
        }
      ).__harness.currentSnapshot()
    );
  });

  await openUserSettings(page);

  const speedGroup = page.getByRole("group", { name: t("settings.searchHistorySpeed") });
  const fastButton = speedGroup.getByRole("button", {
    name: t("settings.searchHistorySpeedFast")
  });
  const standardButton = speedGroup.getByRole("button", {
    name: t("settings.searchHistorySpeedStandard")
  });

  await fastButton.click();

  // Standard should still show as pressed because the snapshot didn't change
  await expect(standardButton).toHaveAttribute("aria-pressed", "true");
  await expect(fastButton).toHaveAttribute("aria-pressed", "false");
});

// ─────────────────────────────────────────────────────────────
//  Toggle switches
// ─────────────────────────────────────────────────────────────

test("toggling 'Index media captions' dispatches update_settings with the flipped value", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() =>
    (
      window as unknown as { __harness: { clearInvocations(): void } }
    ).__harness.clearInvocations()
  );
  await openUserSettings(page);

  const toggle = page.getByRole("switch", { name: t("settings.searchHistoryIncludeCaptions") });
  // Default: include_media_captions: true
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);

  const args = await latestUpdateSettingsArgs(page);
  expect(args).toEqual({
    patch: {
      search_crawler: {
        speed: "standard",
        include_media_captions: false,
        include_filenames: true
      }
    }
  });
});

test("toggling 'Index file names' dispatches update_settings with the flipped value", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() =>
    (
      window as unknown as { __harness: { clearInvocations(): void } }
    ).__harness.clearInvocations()
  );
  await openUserSettings(page);

  const toggle = page.getByRole("switch", { name: t("settings.searchHistoryIncludeFilenames") });
  // Default: include_filenames: true
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();

  await expect.poll(() => invocationCount(page, "update_settings")).toBeGreaterThanOrEqual(1);

  const args = await latestUpdateSettingsArgs(page);
  expect(args).toEqual({
    patch: {
      search_crawler: {
        speed: "standard",
        include_media_captions: true,
        include_filenames: false
      }
    }
  });
});

test("switch checked state updates only after the Rust snapshot changes", async ({ page }) => {
  await gotoReadyShell(page);

  // Make update_settings a no-op (returns unchanged snapshot)
  await page.evaluate(() => {
    (
      window as unknown as {
        __harness: {
          setCommandResponse(cmd: string, handler: () => unknown): void;
          currentSnapshot(): unknown;
        };
      }
    ).__harness.setCommandResponse("update_settings", () =>
      (
        window as unknown as {
          __harness: { currentSnapshot(): unknown };
        }
      ).__harness.currentSnapshot()
    );
  });

  await openUserSettings(page);

  const toggle = page.getByRole("switch", { name: t("settings.searchHistoryIncludeCaptions") });
  await expect(toggle).toHaveAttribute("aria-checked", "true");

  await toggle.click();

  // Still true because the snapshot didn't change
  await expect(toggle).toHaveAttribute("aria-checked", "true");
});

// ─────────────────────────────────────────────────────────────
//  Per-room crawler status rows
// ─────────────────────────────────────────────────────────────

test("room in 'running' state shows Stop button and processed/indexed counts", async ({
  page
}) => {
  await gotoReadyShell(page);

  // Inject a running room into search_crawler state
  await page.evaluate((roomId) => {
    const snap = (
      window as unknown as {
        __harness: {
          currentSnapshot(): DesktopSnapshot;
          setSnapshot(s: DesktopSnapshot): void;
        };
      }
    ).__harness.currentSnapshot();
    (
      window as unknown as {
        __harness: { setSnapshot(s: DesktopSnapshot): void };
      }
    ).__harness.setSnapshot({
      ...snap,
      state: {
        ...snap.state,
        search_crawler: {
          rooms: {
            [roomId]: { kind: "running", processed: 10, indexed: 8 }
          }
        }
      }
    });
  }, HARNESS_ROOM_ID);

  await openUserSettings(page);

  const statusSection = page.getByRole("region", { name: t("settings.searchHistoryRoomStatus") });
  await expect(statusSection).toBeVisible();

  // The status label should contain running info
  const runningRow = statusSection.locator(`[data-crawler-room-kind="running"]`);
  await expect(runningRow).toBeVisible();

  // The Stop button should be present and the Start button absent
  await expect(
    statusSection.getByRole("button", { name: t("settings.searchHistoryStopRoom") })
  ).toBeVisible();
  await expect(
    statusSection.getByRole("button", { name: t("settings.searchHistoryStartRoom") })
  ).not.toBeVisible();
});

test("room in 'idle' state shows Start button and no Stop button", async ({ page }) => {
  await gotoReadyShell(page);

  await page.evaluate((roomId) => {
    const snap = (
      window as unknown as {
        __harness: {
          currentSnapshot(): DesktopSnapshot;
          setSnapshot(s: DesktopSnapshot): void;
        };
      }
    ).__harness.currentSnapshot();
    (
      window as unknown as {
        __harness: { setSnapshot(s: DesktopSnapshot): void };
      }
    ).__harness.setSnapshot({
      ...snap,
      state: {
        ...snap.state,
        search_crawler: {
          rooms: {
            [roomId]: { kind: "idle" }
          }
        }
      }
    });
  }, HARNESS_ROOM_ID);

  await openUserSettings(page);

  const statusSection = page.getByRole("region", { name: t("settings.searchHistoryRoomStatus") });
  await expect(statusSection).toBeVisible();

  await expect(
    statusSection.getByRole("button", { name: t("settings.searchHistoryStartRoom") })
  ).toBeVisible();
  await expect(
    statusSection.getByRole("button", { name: t("settings.searchHistoryStopRoom") })
  ).not.toBeVisible();
});

test("room in 'completed' state shows neither Start nor Stop button", async ({ page }) => {
  await gotoReadyShell(page);

  await page.evaluate((roomId) => {
    const snap = (
      window as unknown as {
        __harness: {
          currentSnapshot(): DesktopSnapshot;
          setSnapshot(s: DesktopSnapshot): void;
        };
      }
    ).__harness.currentSnapshot();
    (
      window as unknown as {
        __harness: { setSnapshot(s: DesktopSnapshot): void };
      }
    ).__harness.setSnapshot({
      ...snap,
      state: {
        ...snap.state,
        search_crawler: {
          rooms: {
            [roomId]: { kind: "completed", indexed: 42 }
          }
        }
      }
    });
  }, HARNESS_ROOM_ID);

  await openUserSettings(page);

  const statusSection = page.getByRole("region", { name: t("settings.searchHistoryRoomStatus") });
  await expect(statusSection).toBeVisible();

  const completedRow = statusSection.locator(`[data-crawler-room-kind="completed"]`);
  await expect(completedRow).toBeVisible();

  await expect(
    statusSection.getByRole("button", { name: t("settings.searchHistoryStartRoom") })
  ).not.toBeVisible();
  await expect(
    statusSection.getByRole("button", { name: t("settings.searchHistoryStopRoom") })
  ).not.toBeVisible();
});

test("room in 'failed' state shows coarse kind label without raw SDK errors", async ({ page }) => {
  await gotoReadyShell(page);

  await page.evaluate((roomId) => {
    const snap = (
      window as unknown as {
        __harness: {
          currentSnapshot(): DesktopSnapshot;
          setSnapshot(s: DesktopSnapshot): void;
        };
      }
    ).__harness.currentSnapshot();
    (
      window as unknown as {
        __harness: { setSnapshot(s: DesktopSnapshot): void };
      }
    ).__harness.setSnapshot({
      ...snap,
      state: {
        ...snap.state,
        search_crawler: {
          rooms: {
            [roomId]: { kind: "failed", failureKind: "sdk" }
          }
        }
      }
    });
  }, HARNESS_ROOM_ID);

  await openUserSettings(page);

  const statusSection = page.getByRole("region", { name: t("settings.searchHistoryRoomStatus") });
  await expect(statusSection).toBeVisible();

  const failedRow = statusSection.locator(`[data-crawler-room-kind="failed"]`);
  await expect(failedRow).toBeVisible();

  // Must contain the coarse "Failed" label
  await expect(failedRow).toContainText(t("settings.searchHistoryRoomFailed"));

  // Must contain the coarse kind "sdk" — not a raw SDK error message
  await expect(failedRow).toContainText("sdk");

  // Must NOT contain anything that looks like a raw error trace
  const text = await failedRow.textContent();
  expect(text).not.toContain("Error:");
  expect(text).not.toContain("panic");
  expect(text).not.toContain("matrix_sdk");
});

test("no rooms in search_crawler.rooms hides the Room index status section", async ({
  page
}) => {
  await gotoReadyShell(page);
  // Default snapshot has search_crawler: { rooms: {} }
  await openUserSettings(page);

  const statusSection = page.locator(
    `[aria-label="${t("settings.searchHistoryRoomStatus")}"]`
  );
  await expect(statusSection).not.toBeVisible();
});
