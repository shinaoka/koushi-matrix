import { expect, test } from "@playwright/test";
import { gotoReadyShell, invocationCount } from "./support/basicOperations";

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

test("Cmd+, opens App Settings while Account Settings is open", async ({ page }) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "Account Settings", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Account Settings", exact: true })).toBeVisible();

  await page.keyboard.press("Meta+,");

  await expect(page.getByRole("dialog", { name: "App Settings", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Account Settings", exact: true })).toHaveCount(0);
});

test("a stale Account Settings open cannot override App Settings after focused-context cleanup settles", async ({ page }) => {
  await gotoReadyShell(page);

  // Put the right panel into a focused-context-visible mode (search) and make
  // the search result carry an open focused context. Account Settings must
  // close that context first; defer the cleanup so the Account -> App switch
  // happens while it is in flight.
  await page.evaluate(() => {
    const harness = window.__harness;
    harness.setCommandResponse("submit_search", () => {
      const snapshot = harness.currentSnapshot();
      return {
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            focused_context: {
              kind: "open",
              room_id: "!harness-room:example.invalid",
              event_id: "$seed-event:example.invalid"
            }
          }
        }
      };
    });
    harness.deferCommand("close_focused_context");
    harness.clearInvocations();
  });

  const searchInput = page.getByRole("textbox", { name: "Search" });
  await searchInput.fill("Alpha");
  await searchInput.press("Enter");
  await expect(page.locator(".focused-context-panel")).toBeVisible();

  await page.evaluate(() => window.__harness.pushDesktopMenu("openAccountSettings"));
  await expect.poll(() => invocationCount(page, "close_focused_context")).toBe(1);

  await page.evaluate(() => window.__harness.pushDesktopMenu("openAppSettings"));
  await expect(page.getByRole("dialog", { name: "App Settings", exact: true })).toBeVisible();

  // Let the stale account open settle; it must not resurrect Account Settings.
  await page.evaluate(() => window.__harness.resolveDeferredCommand("close_focused_context", 0, {
    protocolVersion: 1,
    publishedGeneration: 1
  }));

  await expect(page.getByRole("dialog", { name: "App Settings", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Account Settings", exact: true })).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(1);
});

test("Account Settings to App Settings and back replaces the surface directly", async ({ page }) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "Account Settings", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Account Settings", exact: true })).toBeVisible();

  await page.evaluate(() => window.__harness.pushDesktopMenu("openAppSettings"));
  await expect(page.getByRole("dialog", { name: "App Settings", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Account Settings", exact: true })).toHaveCount(0);

  await page.evaluate(() => window.__harness.pushDesktopMenu("openAccountSettings"));
  await expect(page.getByRole("dialog", { name: "Account Settings", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "App Settings", exact: true })).toHaveCount(0);
});

test("language round trip persists the explicit choice across Settings reopen", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  const appDialog = page.getByRole("dialog", { name: "App Settings", exact: true });
  await expect(appDialog).toBeVisible();
  const languageSelect = page.locator("dialog.user-settings-modal .profile-settings-field select");

  // An unset language_tag shows English and writes nothing on open.
  await expect(languageSelect).toHaveValue("en");
  expect(await invocationCount(page, "update_settings")).toBe(0);

  // Selecting Japanese updates the visible selection and the running UI without
  // a restart, and sends the supported explicit tag with the unchanged direction.
  await languageSelect.selectOption("ja-JP");
  await expect(languageSelect).toHaveValue("ja-JP");
  await expect.poll(() => page.evaluate(() => document.documentElement.lang)).toBe("ja");
  expect(
    await page.evaluate(() => window.__harness.invocationsOf("update_settings").at(-1)?.args)
  ).toEqual({
    patch: { locale: { language_tag: "ja-JP", text_direction: "auto" }, scope: "app" }
  });

  // Selecting English persists the explicit English tag.
  await languageSelect.selectOption("en");
  await expect(languageSelect).toHaveValue("en");
  expect(
    await page.evaluate(() => window.__harness.invocationsOf("update_settings").at(-1)?.args)
  ).toEqual({
    patch: { locale: { language_tag: "en", text_direction: "auto" }, scope: "app" }
  });

  // The selection is retained when Settings is closed and reopened.
  await page.keyboard.press("Escape");
  await expect(appDialog).toHaveCount(0);
  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "App Settings", exact: true })).toBeVisible();
  await expect(
    page.locator("dialog.user-settings-modal .profile-settings-field select")
  ).toHaveValue("en");
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
