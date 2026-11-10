/**
 * Headless spec: the navigation-failure banner must not trap the right panel (#980).
 *
 * The banner was `position: fixed` at the window's top-right corner, which is
 * exactly where the right panel's header and its close button sit. It was drawn
 * above them with an opaque background and no `pointer-events` escape, so it
 * covered the close button and absorbed clicks aimed at it. It also had no
 * dismiss control and no timeout, so the Pinned messages panel could not be
 * closed at all from within that room.
 *
 * These tests measure rendered geometry and hit-testing rather than CSS values,
 * so they fail on the symptom (an unreachable close control) rather than on a
 * particular offset.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

/** Open the right context panel through the Search route the harness supports. */
async function openRightPanel(page: Page): Promise<void> {
  await page.evaluate(
    ({ roomId }) => {
      window.__harness.setCommandResponse("submit_search", ({ query }: { query?: string }) => {
        const next = window.__harness.currentSnapshot();
        return {
          ...next,
          state: {
            ...next.state,
            domain: {
              ...next.state.domain,
              search: {
                kind: "results",
                request_id: 980,
                query: String(query ?? "banner"),
                scope: "currentRoom",
                results: [
                  {
                    room_id: roomId,
                    event_id: "$navigation-failure-banner:example.invalid",
                    sender: "@harness-ada:example.invalid",
                    timestamp_ms: 1_800_000_004_000,
                    score_millis: 990,
                    snippet: "A result so the panel has content.",
                    match_field: "messageBody",
                    highlights: [],
                    match_kind: "exact"
                  }
                ]
              }
            }
          }
        };
      });
    },
    { roomId: HARNESS_ROOM_ID }
  );

  await page.locator(".top-search input").fill("banner");
  await expect(page.locator(".thread-pane")).toBeVisible();
}

async function publishNavigationFailure(page: Page): Promise<void> {
  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          navigation: {
            ...current.state.ui.navigation,
            event_navigation: {
              kind: "failed",
              generation: 2,
              source: "pinned",
              failureKind: "targetMissing"
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(page.getByRole("alert")).toContainText(t("navigation.failed"));
}

// The reporter's capture was ~1086px wide, and the right panel switches from an
// in-grid column to a fixed overlay below 1200px, so walk both sides of that.
for (const width of [1400, 1190, 1086]) {
  test(`the failure banner never overlaps the right panel's close control at ${width}px`, async ({
    page
  }) => {
    await page.setViewportSize({ width, height: 800 });
    await gotoReadyShell(page);
    await openRightPanel(page);
    await publishNavigationFailure(page);

    const overlap = await page.evaluate(() => {
      const banner = document.querySelector<HTMLElement>(".navigation-failure");
      const header = document.querySelector<HTMLElement>(".thread-pane .thread-header");
      const close = header?.querySelector<HTMLElement>("button");
      if (!banner || !close) {
        return null;
      }
      const a = banner.getBoundingClientRect();
      const b = close.getBoundingClientRect();
      const intersects =
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
      // What actually receives a click aimed at the close button's centre.
      const hit = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2);
      return { intersects, hitIsCloseButton: close.contains(hit) };
    });

    expect(overlap).not.toBeNull();
    expect(overlap!.intersects, "the banner overlaps the right panel close button").toBe(false);
    expect(
      overlap!.hitIsCloseButton,
      "a click aimed at the close button is absorbed by the banner"
    ).toBe(true);
  });
}

test("the right panel can still be closed while the failure banner is up", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await openRightPanel(page);
  await publishNavigationFailure(page);

  await page
    .locator(".thread-pane .thread-header button")
    .first()
    .click({ timeout: 2_000 });
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("close_search").length))
    .toBeGreaterThanOrEqual(1);
});

test("the failure banner carries its own dismiss control", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await openRightPanel(page);
  await publishNavigationFailure(page);

  await page.getByRole("button", { name: t("navigation.failedDismiss") }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("dismiss_event_navigation_failure").length
      )
    )
    .toBe(1);
});

// The target room is not in the harness room list, so the banner shows over a
// pending jump away from the room the user had open (`HARNESS_ROOM_ID`).
const HARNESS_JUMP_TARGET_ROOM_ID = "!harness-jump-target:example.invalid";

async function publishNavigationOpening(page: Page): Promise<void> {
  await page.evaluate((targetRoomId) => {
    const current = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          navigation: {
            ...current.state.ui.navigation,
            active_room_id: targetRoomId,
            event_navigation: { kind: "opening", generation: 1, source: "search" }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, HARNESS_JUMP_TARGET_ROOM_ID);
  await expect(page.locator(".navigation-pending")).toContainText(t("navigation.opening"));
}

// #1848: a slow old-history jump can be abandoned from its pending banner. The
// cancel is the existing close command, and it must not leave a failure behind.
test("the pending navigation banner can be cancelled without a failure", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await publishNavigationOpening(page);

  // Rust restores the displaced room and settles the wait to Idle; the harness
  // mirrors that reply (the restore itself is covered by the Rust tests).
  await page.evaluate((roomId) => {
    window.__harness.setCommandResponse("close_focused_context", () => {
      const next = window.__harness.currentSnapshot();
      return {
        ...next,
        state: {
          ...next.state,
          ui: {
            ...next.state.ui,
            navigation: {
              ...next.state.ui.navigation,
              active_room_id: roomId,
              event_navigation: { kind: "idle" }
            }
          }
        }
      };
    });
  }, HARNESS_ROOM_ID);

  await page.locator(".navigation-pending").getByRole("button", { name: t("action.cancel") }).click();
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("close_focused_context").length))
    .toBe(1);
  await expect(page.getByText(t("navigation.opening"))).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  // The restored room renders again.
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
});
