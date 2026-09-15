import { expect, test } from "@playwright/test";
import { gotoReadyShell } from "./support/basicOperations";

for (const viewport of [{ width: 1334, height: 852 }, { width: 700, height: 480 }]) {
  test(`settings stay in the viewport and contain keyboard focus at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await gotoReadyShell(page);
    await expect(page.getByRole("button", { name: "Keyboard settings" })).toHaveCount(0);
    const opener = page.getByRole("button", { name: "User settings", exact: true });
    await opener.click();
    const dialog = page.getByRole("dialog", { name: "User settings" });
    await expect(dialog).toBeVisible();
    expect(await dialog.evaluate(el => el.matches(":modal"))).toBe(true);
    const bounds = await dialog.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(viewport.width);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height);
    await expect(page.locator(".app-grid-right-resizer")).toHaveCount(0);
    await expect(dialog.getByRole("group", { name: "Language" })).toBeVisible();
    await dialog.getByRole("tab", { name: "Account", exact: true }).focus();
    await page.keyboard.press("ArrowDown");
    await expect(dialog.getByRole("tab", { name: "Sessions", exact: true })).toBeFocused();
    await expect(dialog.getByRole("tabpanel", { name: "Sessions", exact: true })).toBeVisible();
    await page.keyboard.press("End");
    await expect(dialog.getByRole("tabpanel", { name: "Help & About", exact: true })).toBeVisible();
    await dialog.getByRole("tab", { name: "Keyboard", exact: true }).click();
    await expect(dialog.getByText("Composer send shortcut", { exact: true })).toBeVisible();
    await expect(dialog.getByRole("tabpanel")).toHaveCount(1);
    for (let i = 0; i < 16; i++) {
      await page.keyboard.press("Tab");
      expect(await dialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
    }
    // Window-level shortcuts must not navigate the shell under a modal.
    await page.keyboard.press("Control+k");
    await expect(dialog).toBeVisible();
    expect(await dialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(opener).toBeFocused();
  });
}

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
