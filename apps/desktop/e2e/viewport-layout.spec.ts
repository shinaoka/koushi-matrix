import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import type { AccountTabsSnapshot } from "../src/domain/types";
import { gotoReadyShell } from "./support/basicOperations";

async function layoutGeometry(page: Page) {
  return page.evaluate(() => {
    const rect = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const box = element.getBoundingClientRect();
      return {
        top: box.top,
        left: box.left,
        right: box.right,
        bottom: box.bottom,
        width: box.width,
        height: box.height
      };
    };
    return {
      viewport: { width: window.innerWidth, height: window.innerHeight },
      document: {
        width: document.documentElement.clientWidth,
        height: document.documentElement.clientHeight
      },
      body: rect("body"),
      root: rect("#root"),
      desktop: rect(".desktop"),
      panelClose: rect(".thread-pane .thread-header button:last-child")
    };
  });
}

function expectRootAligned(geometry: Awaited<ReturnType<typeof layoutGeometry>>) {
  expect(geometry.body).not.toBeNull();
  expect(geometry.root).not.toBeNull();
  expect(geometry.desktop).not.toBeNull();
  for (const box of [geometry.body, geometry.root, geometry.desktop]) {
    expect(box!.top).toBeCloseTo(0, 0);
    expect(box!.left).toBeCloseTo(0, 0);
    expect(box!.width).toBeCloseTo(geometry.document.width, 0);
    expect(box!.height).toBeCloseTo(geometry.document.height, 0);
    expect(box!.right).toBeLessThanOrEqual(geometry.viewport.width + 1);
    expect(box!.bottom).toBeLessThanOrEqual(geometry.viewport.height + 1);
  }
}

test("narrow account tabs retain visible avatars and status with accessible names", async ({ page }) => {
  await gotoReadyShell(page);
  const singleTab = page.locator(".account-tab").first();
  await page.setViewportSize({ width: 761, height: 720 });
  await expect(singleTab.locator(".account-tab-label")).toBeVisible();
  await expect(singleTab.locator(".account-tab-ready-dot")).toBeVisible();
  await page.setViewportSize({ width: 760, height: 720 });
  const accountTabs: AccountTabsSnapshot = {
    selectedTabId: "harness-account-tab",
    tabs: [
      {
        id: "harness-account-tab",
        accountKey: "@harness-user:example.invalid",
        homeserver: "https://harness.example.invalid",
        displayName: "Harness",
        avatarSourceRef: null,
        status: "ready",
        unreadCount: 0
      },
      {
        id: "error-account-tab",
        accountKey: "@error:example.invalid",
        homeserver: "https://error.example.invalid",
        displayName: "Error account",
        avatarSourceRef: null,
        status: "error",
        unreadCount: 0
      },
      {
        id: "signed-out-account-tab",
        accountKey: "@signed-out:example.invalid",
        homeserver: "https://signed-out.example.invalid",
        displayName: "Signed out",
        avatarSourceRef: null,
        status: "signedOut",
        unreadCount: 0
      }
    ],
    badgeCount: 0
  };
  await page.evaluate(async (tabs) => {
    await (window as unknown as {
      __harness: { pushAccountTabs(snapshot: AccountTabsSnapshot): Promise<void> };
    }).__harness.pushAccountTabs(tabs);
  }, accountTabs);
  const tabs = page.locator(".account-tab");
  await expect(tabs).toHaveCount(3);
  for (const [index, statusClass, name] of [
    [0, "account-tab-ready-dot", "Harness: Ready"],
    [1, "account-tab-error-dot", "Error account: Account error"],
    [2, "account-tab-signed-out-dot", "Signed out: Sign in again"]
  ] as const) {
    const tab = tabs.nth(index);
    const label = tab.locator(".account-tab-label");
    const avatar = tab.locator(".account-tab-avatar");
    const status = tab.locator(`.${statusClass}`);
    expect(await label.evaluate((element) => getComputedStyle(element).display)).toBe("none");
    expect(await avatar.evaluate((element) => element.getBoundingClientRect().width)).toBeGreaterThan(0);
    expect(await status.evaluate((element) => element.getBoundingClientRect().width)).toBeGreaterThan(0);
    await expect(tab).toHaveAttribute("aria-label", name);
  }
  await expect(page.getByRole("button", { name: "Add account", exact: true })).toBeVisible();
});

test("right-panel header exposes no inert More action", async ({ page }) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: t("workspace.userSettings"), exact: true }).click();
  const contextPanel = page.getByRole("dialog", { name: "Account Settings" });
  await expect(contextPanel.getByRole("button", { name: "More", exact: true })).toHaveCount(0);
  const close = contextPanel.getByRole("button", {
    name: t("action.close", { title: t("panel.userSettings") }),
    exact: true
  });
  await close.focus();
  await close.press("Enter");
  await expect(page.locator(".app-grid")).toHaveClass(/(^|\s)thread-closed(\s|$)/);
});

test("settings categories change without scrolling the desktop", async ({ page }) => {
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "Account Settings", exact: true }).click();
  const accountDialog = page.getByRole("dialog", { name: "Account Settings" });
  for (const name of ["Account", "Sessions", "Notifications", "Security & Privacy", "Encryption", "Search history"]) {
    await accountDialog.getByRole("tab", { name, exact: true }).click();
    await expect(accountDialog.getByRole("tabpanel")).toHaveCount(1);
    await expect(accountDialog.getByRole("tabpanel", { name, exact: true })).toBeVisible();
    expect(await page.locator(".desktop").evaluate(element => element.scrollTop)).toBe(0);
  }
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  const appDialog = page.getByRole("dialog", { name: "App Settings" });
  for (const name of ["Appearance", "Notifications", "Preferences", "Keyboard", "Search history", "Help & About"]) {
    await appDialog.getByRole("tab", { name, exact: true }).click();
    await expect(appDialog.getByRole("tabpanel")).toHaveCount(1);
    await expect(appDialog.getByRole("tabpanel", { name, exact: true })).toBeVisible();
    expect(await page.locator(".desktop").evaluate(element => element.scrollTop)).toBe(0);
  }
});

test("density, browser resize, and right-panel resize preserve the root viewport", async ({
  page
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await gotoReadyShell(page);
  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  await page.getByRole("tab", { name: "Appearance", exact: true }).click();

  for (const density of ["Compact", "Default", "Comfortable"] as const) {
    await page.getByRole("button", { name: density, exact: true }).click();
    await expect(page.locator(`.desktop[data-density="${density.toLowerCase()}"]`)).toBeVisible();
    expectRootAligned(await layoutGeometry(page));
  }

  expectRootAligned(await layoutGeometry(page));
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Account Settings", exact: true }).click();
  const closePanel = page.getByRole("button", {
    name: t("action.close", { title: t("settings.accountSettings") }),
    exact: true
  });
  await closePanel.click();
  await expect(page.locator(".app-grid")).toHaveClass(/(^|\s)thread-closed(\s|$)/);
  await expect(closePanel).toBeHidden();
  await page.getByRole("button", { name: "Room info", exact: true }).click();
  const resizer = page.getByRole("button", { name: t("workspace.resizeRightPanel") });
  const beforePanel = await page
    .locator(".thread-pane")
    .evaluate((element) => element.getBoundingClientRect().width);
  const resizerBox = await resizer.boundingBox();
  expect(resizerBox).not.toBeNull();
  await page.mouse.move(resizerBox!.x + resizerBox!.width / 2, resizerBox!.y + 4);
  await page.mouse.down();
  await page.mouse.move(resizerBox!.x - 80, resizerBox!.y + 4);
  await page.mouse.up();
  const afterPanel = await page
    .locator(".thread-pane")
    .evaluate((element) => element.getBoundingClientRect().width);
  expect(afterPanel).not.toBe(beforePanel);
  expectRootAligned(await layoutGeometry(page));


  expectRootAligned(await layoutGeometry(page));

  await page.setViewportSize({ width: 1100, height: 720 });
  expectRootAligned(await layoutGeometry(page));
});
