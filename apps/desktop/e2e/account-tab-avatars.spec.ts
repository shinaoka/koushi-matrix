/**
 * #2059: a background account's tab shows that account's own avatar.
 *
 * The selected account's timeline demands its own avatar thumbnail. A background
 * account had nothing to demand one through, so its tab stayed on initials until
 * the reader selected it. The strip now demands it from that account's own tab
 * id, and renders Core's answer: initials until the thumbnail is ready, the image
 * after.
 */

import { expect, test, type Page } from "@playwright/test";
import type { AccountTabsSnapshot } from "../src/domain/types";
import { gotoReadyShell } from "./support/basicOperations";

const SELECTED_ID = "harness-account-tab";
const BACKGROUND_ID = "background-account-tab";
const SELECTED_MXC = "mxc://example.invalid/selected-avatar";
const BACKGROUND_MXC = "mxc://example.invalid/background-avatar";
const BACKGROUND_SOURCE_REF = "avatar/0123456789abcdef";
const SELECTED_SOURCE_REF = "avatar/fedcba9876543210";

function tabs(
  ready: { background?: string | null; selected?: string | null; backgroundName?: string } = {}
): AccountTabsSnapshot {
  return {
    selectedTabId: SELECTED_ID,
    tabs: [
      {
        id: SELECTED_ID,
        accountKey: "harness-account",
        homeserver: "https://harness.example.invalid",
        displayName: "Harness",
        avatarMxcUri: SELECTED_MXC,
        avatarSourceRef: ready.selected ?? null,
        status: "ready",
        unreadCount: 0
      },
      {
        id: BACKGROUND_ID,
        accountKey: "@bob:example.invalid",
        homeserver: "https://bob.example.invalid",
        displayName: ready.backgroundName ?? "Bob",
        avatarMxcUri: BACKGROUND_MXC,
        avatarSourceRef: ready.background ?? null,
        status: "ready",
        unreadCount: 0
      }
    ],
    badgeCount: 0
  };
}

/**
 * The demands for one media resource, and the account each one addressed.
 * `invocations()` keeps the routing argument; `invocationsOf()` drops it.
 */
async function demandsFor(page: Page, mxcUri: string): Promise<Array<string | null>> {
  return page.evaluate(
    (uri) =>
      window.__harness
        .invocations()
        .filter((invocation) => invocation.command === "download_avatar_thumbnail")
        .filter((invocation) => invocation.args.mxcUri === uri)
        .map((invocation) => (invocation.args.accountTabId as string | null | undefined) ?? null),
    mxcUri
  );
}

test("a background account's tab shows its own avatar", async ({ page }) => {
  await gotoReadyShell(page);
  const backgroundTab = page.getByRole("button", { name: "Bob: Ready", exact: true });

  await page.evaluate((snapshot) => {
    void window.__harness.pushAccountTabs(snapshot);
  }, tabs());
  await expect(backgroundTab).toBeVisible();
  // Before Core has fetched anything, the tab shows the account's initials.
  await expect(backgroundTab.locator(".account-tab-avatar")).toHaveText("B");
  await expect(backgroundTab.locator("img")).toHaveCount(0);

  // The strip asked the background account itself: the demand names its tab id,
  // because the thumbnail is fetched with that account's own client.
  await expect.poll(() => demandsFor(page, BACKGROUND_MXC)).toEqual([BACKGROUND_ID]);
  // The selected account keeps its own demand path (`App.tsx`'s timeline
  // transport), so the strip never asks twice for it.
  expect(await demandsFor(page, SELECTED_MXC)).toEqual([]);

  // Core answers with a ready thumbnail in the next account-tab snapshot, and
  // the tab swaps its initials for the image.
  await page.evaluate((next) => {
    void window.__harness.pushAccountTabs(next);
  }, tabs({ background: BACKGROUND_SOURCE_REF }));
  await expect(backgroundTab.locator("img")).toHaveAttribute("src", /avatar\/0123456789abcdef$/);
  await expect(backgroundTab.locator(".account-tab-avatar")).toHaveText("");

  // The selected account has no ready thumbnail yet, so it keeps its initials
  // instead of borrowing the other account's image.
  const selectedTab = page.getByRole("button", { name: "Harness: Ready", exact: true });
  await expect(selectedTab.locator("img")).toHaveCount(0);
  await expect(selectedTab.locator(".account-tab-avatar")).toHaveText("H");

  // A ready thumbnail ends the demand: two more snapshots for the same account
  // must not ask again. The rename proves each snapshot was applied, so this
  // does not depend on a timer.
  for (const name of ["Robert", "Bobby"]) {
    await page.evaluate((next) => {
      void window.__harness.pushAccountTabs(next);
    }, tabs({ background: BACKGROUND_SOURCE_REF, backgroundName: name }));
    await expect(page.getByRole("button", { name: `${name}: Ready`, exact: true })).toBeVisible();
    expect(await demandsFor(page, BACKGROUND_MXC)).toEqual([BACKGROUND_ID]);
  }
});

test("the selected account's tab shows its avatar on this process's own demand", async ({ page }) => {
  // The restart case: hydration leaves the own avatar `notRequested`, so the tab
  // shows initials until the selected account's timeline demands it and Core
  // answers with a ready thumbnail.
  await gotoReadyShell(page);
  await page.evaluate(
    ({ snapshot, mxcUri }) => {
      const current = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...current,
        state: {
          ...current.state,
          domain: {
            ...current.state.domain,
            profile: {
              ...current.state.domain.profile,
              own: {
                ...current.state.domain.profile.own,
                avatar: { mxc_uri: mxcUri, thumbnail: { kind: "notRequested" as const } }
              }
            }
          }
        }
      });
      // `setSnapshot` only stages the next answer; the delta is what the App
      // actually receives, and the own-avatar demand reads it.
      window.__harness.pushStateUpdate();
      void window.__harness.pushAccountTabs(snapshot);
    },
    { snapshot: tabs(), mxcUri: SELECTED_MXC }
  );

  const selectedTab = page.getByRole("button", { name: "Harness: Ready", exact: true });
  await expect(selectedTab.locator(".account-tab-avatar")).toHaveText("H");

  // The demand goes through the selected account's transport, so it carries that
  // tab id too - it is the same account either way.
  await expect.poll(() => demandsFor(page, SELECTED_MXC)).toEqual([SELECTED_ID]);

  await page.evaluate((next) => {
    void window.__harness.pushAccountTabs(next);
  }, tabs({ selected: SELECTED_SOURCE_REF }));
  await expect(selectedTab.locator("img")).toHaveAttribute("src", /avatar\/fedcba9876543210$/);
});
