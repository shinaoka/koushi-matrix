import { expect, test } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

for (const viewport of [{ width: 1334, height: 852 }, { width: 700, height: 480 }]) {
  test(`settings stay in the viewport and contain keyboard focus at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await gotoReadyShell(page);
    await expect(page.getByRole("button", { name: "Keyboard settings" })).toHaveCount(0);

    const accountOpener = page.getByRole("button", { name: "Account Settings", exact: true });
    await accountOpener.click();
    const accountDialog = page.getByRole("dialog", { name: "Account Settings" });
    await expect(accountDialog).toBeVisible();
    expect(await accountDialog.evaluate(el => el.matches(":modal"))).toBe(true);
    const bounds = await accountDialog.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(viewport.width);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height);
    await expect(page.locator(".app-grid-right-resizer")).toHaveCount(0);
    await accountDialog.getByRole("tab", { name: "Account", exact: true }).focus();
    await page.keyboard.press("ArrowDown");
    await expect(accountDialog.getByRole("tab", { name: "Sessions", exact: true })).toBeFocused();
    await expect(accountDialog.getByRole("tabpanel", { name: "Sessions", exact: true })).toBeVisible();
    await page.keyboard.press("End");
    await expect(accountDialog.getByRole("tabpanel", { name: "Search history", exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(accountDialog).toHaveCount(0);
    await expect(accountOpener).toBeFocused();

    const appOpener = page.getByRole("button", { name: "App Settings", exact: true });
    await appOpener.click();
    const appDialog = page.getByRole("dialog", { name: "App Settings" });
    await expect(appDialog).toBeVisible();
    await expect(appDialog.getByRole("combobox", { name: "Language" })).toBeVisible();
    await appDialog.getByRole("tab", { name: "Keyboard", exact: true }).click();
    await expect(appDialog.getByText("Composer send shortcut", { exact: true })).toBeVisible();
    await expect(appDialog.getByRole("tabpanel")).toHaveCount(1);
    for (let i = 0; i < 16; i++) {
      await page.keyboard.press("Tab");
      expect(await appDialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
    }
    // Window-level shortcuts must not navigate the shell under a modal.
    await page.keyboard.press("Control+k");
    await expect(appDialog).toBeVisible();
    expect(await appDialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
    await page.keyboard.press("Escape");
    await expect(appDialog).toHaveCount(0);
    await expect(appOpener).toBeFocused();
  });
}

test("native Account Settings and App Settings menu items switch to the requested scope", async ({ page }) => {
  await gotoReadyShell(page);

  await page.evaluate(() => window.__harness.pushDesktopMenu("openAppSettings"));
  await expect(page.getByRole("dialog", { name: "App Settings" })).toBeVisible();

  // Choosing Account Settings while App Settings is open switches directly.
  await page.evaluate(() => window.__harness.pushDesktopMenu("openAccountSettings"));
  await expect(page.getByRole("dialog", { name: "Account Settings" })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "App Settings" })).toHaveCount(0);

  // And back again, without closing settings first.
  await page.evaluate(() => window.__harness.pushDesktopMenu("openAppSettings"));
  await expect(page.getByRole("dialog", { name: "App Settings" })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Account Settings" })).toHaveCount(0);
});

test("native Help opens a URL-copy dialog before sign-in", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({ ...snapshot, state: { ...snapshot.state, domain: {
      ...snapshot.state.domain, session: { kind: "signedOut" }, auth: { kind: "unknown" }, sync: "stopped"
    } } });
    window.__harness.pushStateUpdate();
  });
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toHaveCount(0);
  await page.evaluate(() => window.__harness.pushDesktopMenu("showHelp"));
  const dialog = page.getByRole("dialog", { name: "Koushi Help" });
  await expect(dialog.getByText(/ChatGPT/)).toBeVisible();
  await dialog.getByRole("button", { name: "Copy GitHub URL" }).click();
  await expect(dialog.getByRole("status")).toHaveText("URL copied");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("https://github.com/shinaoka/koushi-matrix");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});
