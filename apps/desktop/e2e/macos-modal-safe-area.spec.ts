import { expect, test } from "@playwright/test";

test("settings reserve native-button space without a mounted titlebar", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 320 });
  await page.goto("/appHarness.html");
  await page.getByRole("button", { name: "User settings", exact: true }).click();
  await page.evaluate(() => {
    document.documentElement.dataset.platform = "macos";
    document.querySelector(".titlebar")?.remove();
  });
  const dialog = page.getByRole("dialog", { name: "User settings" });
  const rect = await dialog.boundingBox();
  expect(rect!.y).toBeGreaterThanOrEqual(44);
  expect(rect!.y + rect!.height).toBeLessThanOrEqual(320);
});

for (const platform of ["macos", "windows", "linux"]) {
  for (const zoom of [0.75, 1, 1.5]) {
    test(`${platform} overlays fit a short window at zoom ${zoom}`, async ({ page }) => {
      await page.setViewportSize({ width: 700, height: 320 });
      await page.goto("/e2e/modalSafeAreaHarness.html");
      await page.evaluate(({ platform, zoom }) => {
        document.documentElement.dataset.platform = platform;
        document.documentElement.style.setProperty("--webview-zoom", String(zoom));
        document.documentElement.dir = "rtl";
      }, { platform, zoom });
      const safeTop = platform === "macos" ? Math.max(44, 44 / zoom) : 0;
      for (const opener of ["Open parent", "Open legacy", "media-viewer-backdrop", "timeline-media-viewer-overlay", "dialog-overlay upload-staging-overlay"]) {
        await page.getByRole("button", { name: opener, exact: true }).click();
        const modal = page.locator("dialog:modal");
        await expect(modal).toHaveCount(1);
        const content = modal.locator(".app-modal-header, .dialog-box, .media-viewer, .timeline-media-viewer, .upload-staging-dialog").first();
        const bounds = await content.boundingBox();
        expect(bounds!.y).toBeGreaterThanOrEqual(safeTop - 1);
        expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(321);
        const last = modal.getByRole("button").last();
        await last.focus();
        await last.scrollIntoViewIfNeeded();
        const action = await last.boundingBox();
        expect(action!.y).toBeGreaterThanOrEqual(safeTop - 1);
        expect(action!.y + action!.height).toBeLessThanOrEqual(321);
        await page.keyboard.press("Escape");
        await expect(modal).toHaveCount(0);
      }
    });
  }
}

test("nested dialogs preserve drafts, trap focus, guard IME/busy dismissal, and restore focus", async ({ page }) => {
  await page.goto("/e2e/modalSafeAreaHarness.html");
  const opener = page.getByRole("button", { name: "Open parent", exact: true });
  await opener.click();
  await page.getByRole("textbox", { name: "Draft", exact: true }).fill("Synthetic draft");
  const childOpener = page.getByRole("button", { name: "Open child", exact: true });
  await childOpener.click();
  const child = page.getByRole("dialog", { name: "Child", exact: true });
  const input = child.getByRole("textbox");
  await input.focus();
  await input.dispatchEvent("compositionstart");
  await page.keyboard.press("Escape");
  await expect(child).toBeVisible();
  await input.dispatchEvent("compositionend");
  await child.getByRole("button", { name: "Toggle busy" }).click();
  await page.keyboard.press("Escape");
  await expect(child).toBeVisible();
  await child.getByRole("button", { name: "Toggle busy" }).click();
  await child.getByRole("button", { name: "Child last action" }).focus();
  await page.keyboard.press("Tab");
  await expect(child.getByRole("button", { name: "Close Child" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(child.getByRole("button", { name: "Child last action" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(child).toHaveCount(0);
  await expect(childOpener).toBeFocused();
  await expect(page.getByRole("textbox", { name: "Draft", exact: true })).toHaveValue("Synthetic draft");
  await page.keyboard.press("Escape");
  await expect(opener).toBeFocused();
});

test("floating content launched inside a native modal stays interactive and safe", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 320 });
  await page.goto("/e2e/modalSafeAreaHarness.html");
  await page.evaluate(() => { document.documentElement.dataset.platform = "macos"; });
  await page.getByRole("button", { name: "Open parent", exact: true }).click();
  await page.getByRole("button", { name: "Open popup", exact: true }).click();
  const popup = page.getByTestId("floating");
  expect(await popup.evaluate(el => !!el.closest("dialog:modal"))).toBe(true);
  const bounds = await popup.boundingBox();
  expect(bounds!.y).toBeGreaterThanOrEqual(44);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(320);
  await popup.getByRole("button").click();
  await expect(popup).toHaveCount(0);
  await expect(page.getByRole("dialog", { name: "Parent", exact: true })).toBeVisible();
});

test("real emoji picker autofocus and Escape are local to the parent modal", async ({ page }) => {
  await page.goto("/e2e/modalSafeAreaHarness.html");
  await page.getByRole("button", { name: "Open parent", exact: true }).click();
  await page.getByRole("button", { name: "Open modal emoji", exact: true }).click();
  const picker = page.locator(".emoji-picker");
  await expect(picker.getByRole("searchbox")).toBeFocused();
  await picker.getByRole("searchbox").dispatchEvent("keydown", { key: "Escape", isComposing: true });
  await expect(picker).toBeVisible();
  await picker.getByRole("searchbox").fill("smile");
  await page.setViewportSize({ width: 700, height: 400 });
  await expect(picker.getByRole("searchbox")).toHaveValue("smile");
  await expect(picker.getByRole("searchbox")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(picker).toHaveCount(0);
  await expect(page.getByRole("dialog", { name: "Parent", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open modal emoji", exact: true })).toBeFocused();
});

test("session status respects changing native safe space and stays scrollable in a short window", async ({ page }) => {
  await page.setViewportSize({ width: 820, height: 320 });
  await page.goto("/appHarness.html");
  await page.getByRole("button", { name: "Open session status", exact: true }).click();
  await page.evaluate(() => {
    document.documentElement.dataset.platform = "macos";
    document.documentElement.style.setProperty("--webview-zoom", "0.5");
  });
  const popup = page.locator(".session-status-popover");
  await expect.poll(async () => (await popup.boundingBox())!.y).toBeGreaterThanOrEqual(88);
  const bounds = (await popup.boundingBox())!;
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(320);
  const last = popup.getByRole("button").last();
  await last.focus();
  await expect(last).toBeInViewport();
  await page.keyboard.press("Escape");
  await expect(popup).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Open session status", exact: true })).toBeFocused();
});

test("root settings, native Help, and create dialogs restore pointer origins", async ({ page }) => {
  await page.goto("/appHarness.html");
  const settings = page.getByRole("button", { name: "User settings", exact: true });
  await settings.click();
  await expect(page.getByRole("dialog", { name: "User settings", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(settings).toBeFocused();
  // The native menu has no WebView DOM button. Preserve the most recent app
  // focus origin when its callback opens Help.
  await page.evaluate(() => window.__harness.pushDesktopMenu("showHelp"));
  await expect(page.getByRole("dialog", { name: "Koushi Help" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(settings).toBeFocused();
  const create = page.getByRole("button", { name: "Create space", exact: true });
  await create.click();
  await expect(page.getByRole("dialog", { name: "Create space", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(create).toBeFocused();
});
