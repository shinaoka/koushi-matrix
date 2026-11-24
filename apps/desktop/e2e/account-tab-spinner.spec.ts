import { expect, test } from "@playwright/test";
import type { AccountTabsSnapshot } from "../src/domain/types";
import { gotoReadyShell } from "./support/basicOperations";

// #1544: the account-tab activity spinner is a flex item next to the
// (ellipsized) account label and server name. With a long label the tab hits
// its max-width and the spinner shrank horizontally, so it rendered as an
// oval whose rotation wobbled instead of turning around its center.
function restoringTabs(): AccountTabsSnapshot {
  return {
    selectedTabId: "harness-account-tab",
    tabs: [
      {
        id: "harness-account-tab",
        accountKey: "@harness-user:example.invalid",
        homeserver: "https://harness.example.invalid",
        displayName: "Harness",
        avatarSourceRef: null,
        avatarMxcUri: null,
        status: "ready",
        unreadCount: 0
      },
      {
        id: "restoring-account-tab",
        accountKey: "@a-very-long-account-localpart-name:a-long-homeserver-name.example.invalid",
        homeserver: "https://a-long-homeserver-name.example.invalid",
        displayName: "A very long account display name that overflows the tab",
        avatarSourceRef: null,
        avatarMxcUri: null,
        status: "restoring",
        unreadCount: 0
      }
    ],
    badgeCount: 0
  };
}

test("account-tab spinner stays a centered circle beside a long account name (#1544)", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate((tabs) => {
    void (window.__harness as any).pushAccountTabs(tabs);
  }, restoringTabs());

  const spinner = page.locator(".account-tab-spinner");
  await expect(spinner).toBeVisible();

  const geometry = await spinner.evaluate((element) => {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    const parentRect = element.parentElement!.getBoundingClientRect();
    return {
      // Layout box, unaffected by the running rotate() transform.
      width: (element as HTMLElement).offsetWidth,
      height: (element as HTMLElement).offsetHeight,
      transformOrigin: style.transformOrigin,
      centerY: rect.top + rect.height / 2,
      parentCenterY: parentRect.top + parentRect.height / 2
    };
  });

  expect(geometry.width).toBe(13);
  expect(geometry.height).toBe(13);
  // The default origin is the box center; with a square box that is (6.5, 6.5).
  expect(geometry.transformOrigin).toBe("6.5px 6.5px");
  expect(Math.abs(geometry.centerY - geometry.parentCenterY)).toBeLessThanOrEqual(1);
});
