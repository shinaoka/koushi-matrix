import { expect, test } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

test("Koushi update action opens results directly, including before sign-in", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    snapshot.state.domain.locale_profile.platform = "macos";
    snapshot.state.domain.session = { kind: "signedOut" };
    snapshot.state.domain.auth = { kind: "unknown" };
    snapshot.state.domain.sync = "stopped";
    window.__harness.setSnapshot(snapshot);
    window.__harness.setCommandResponse("check_for_desktop_update", null);
    window.__harness.clearInvocations();
  });
  await page.evaluate(() => window.__harness.pushDesktopMenu("checkForUpdates"));
  const dialog = page.getByRole("dialog", { name: "Software update", exact: true });
  await expect(dialog).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("check_for_desktop_update").length)).toBe(1);
  await page.evaluate(() => window.__harness.pushDesktopUpdate({ kind: "up_to_date", version: "1.2.3" }));
  await expect(dialog.getByText("Koushi is up to date (v1.2.3).")).toBeVisible();
  await expect(page.getByRole("dialog", { name: "User settings", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("manual checks keep the settings page underneath and never download or restart implicitly", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    snapshot.state.domain.locale_profile.platform = "macos";
    snapshot.state.domain.settings.values.updates = { auto_check: false, include_prereleases: false };
    window.__harness.setSnapshot(snapshot);
    for (const command of ["check_for_desktop_update", "download_desktop_update", "restart_to_install_desktop_update"]) {
      window.__harness.setCommandResponse(command, null);
    }
  });
  await page.getByRole("button", { name: "User settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "User settings", exact: true });
  await settings.getByRole("tab", { name: "Preferences", exact: true }).click();
  await expect(settings.getByRole("switch", { name: "Automatically check for updates" })).toHaveCount(0);
  await page.evaluate(() => window.__harness.pushDesktopMenu("checkForUpdates"));
  const dialog = page.getByRole("dialog", { name: "Software update", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("switch", { name: "Automatically check for updates" })).toHaveAttribute("aria-checked", "false");
  await page.evaluate(() => window.__harness.pushDesktopUpdate({ kind: "available", version: "1.2.4", generation: 7 }));
  await expect(dialog.getByRole("button", { name: "Download update" })).toBeVisible();
  expect(await page.evaluate(() => window.__harness.invocationsOf("download_desktop_update").length)).toBe(0);
  await dialog.getByRole("button", { name: "Download update" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("download_desktop_update").length)).toBe(1);
  await page.evaluate(() => window.__harness.pushDesktopUpdate({ kind: "ready", version: "1.2.4" }));
  expect(await page.evaluate(() => window.__harness.invocationsOf("restart_to_install_desktop_update").length)).toBe(0);
  await page.evaluate(() => window.__harness.pushDesktopMenu("checkForUpdates"));
  await expect(dialog).toHaveCount(1);
  await dialog.getByRole("button", { name: "Restart to install" }).click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("restart_to_install_desktop_update").length)).toBe(1);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(settings.getByRole("tabpanel", { name: "Preferences", exact: true })).toBeVisible();
});

test("automatic availability and unsupported builds use the same nonempty update screen", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.pushDesktopUpdate({ kind: "available", version: "1.2.4", generation: 7 }));
  const dialog = page.getByRole("dialog", { name: "Software update", exact: true });
  await expect(dialog.getByRole("button", { name: "Download update" })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.evaluate(() => window.__harness.pushDesktopUpdate({ kind: "available", version: "1.2.4", generation: 7 }));
  await expect(dialog).toHaveCount(0);
  await page.evaluate(() => window.__harness.pushDesktopMenu("checkForUpdates"));
  await expect(dialog).toBeVisible();
  await page.evaluate(() => window.__harness.pushDesktopUpdate({ kind: "unsupported" }));
  await expect(dialog.getByText("In-app updates are unavailable in this build.")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Download update" })).toHaveCount(0);
});
