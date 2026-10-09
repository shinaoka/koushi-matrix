/**
 * Headless spec: scheduled-messages right-panel lifecycle (#1160).
 *
 * The panel renders a Rust-owned, body-bearing projection
 * (`state.ui.scheduled_sends_list`). These tests drive the mounted App with
 * explicit Rust-shaped transport updates and assert that the renderer:
 *
 *  - retires a projection a fresh renderer did not open (attach reconcile),
 *  - removes the bodies when Rust replaces an `Open` with `Closed`,
 *  - never lets a delayed open outlive a newer panel intent or the
 *    before-account-switch cleanup,
 *  - keeps two accounts' projections apart.
 *
 * The harness only carries the transport value the test publishes; it does not
 * derive the projection or the reservation queue.
 */

import { expect, test, type Page } from "@playwright/test";
import type { AccountTabsSnapshot } from "../src/domain/types";
import { t } from "../src/i18n/messages";
import { gotoReadyShell, HARNESS_ROOM_ID } from "./support/basicOperations";
import { pushDelta } from "./support/stateUpdates";

const HARNESS_SPACE_ID = "!harness-space:example.invalid";
const HARNESS_TAB_ID = "harness-account-tab";
const BOB_TAB_ID = "bob-account-tab";
const ALPHA_BODY = "Synthetic alpha scheduled body";
const BETA_BODY = "Synthetic beta scheduled body";

function accountTabs(selectedTabId: string): AccountTabsSnapshot {
  return {
    selectedTabId,
    tabs: [
      {
        id: HARNESS_TAB_ID,
        accountKey: "@harness-user:example.invalid",
        homeserver: "https://harness.example.invalid",
        displayName: "Harness",
        avatarSourceRef: null,
        status: "ready",
        unreadCount: 0
      },
      {
        id: BOB_TAB_ID,
        accountKey: "@bob:example.invalid",
        homeserver: "https://bob.example.invalid",
        displayName: "Bob",
        avatarSourceRef: null,
        status: "ready",
        unreadCount: 0
      }
    ],
    badgeCount: 0
  };
}

function projectionKind(page: Page): Promise<string> {
  return page.evaluate(
    () => window.__harness.currentSnapshot().state.ui.scheduled_sends_list.kind
  );
}

function queueReservationCount(page: Page): Promise<number> {
  return page.evaluate(
    () => window.__harness.currentSnapshot().state.ui.timeline.scheduled_sends.length
  );
}

async function registerProjectionClose(page: Page): Promise<void> {
  await page.evaluate(() => {
    window.__harness.setCommandResponse("close_scheduled_sends_list", () => {
      const current = window.__harness.currentSnapshot();
      const next = {
        ...current,
        state: {
          ...current.state,
          ui: { ...current.state.ui, scheduled_sends_list: { kind: "closed" as const } }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
  });
}

async function registerOpenProjection(page: Page, body: string, id: string): Promise<void> {
  await page.evaluate(
    ({ body: itemBody, id: itemId, spaceId }) => {
      window.__harness.setCommandResponse("open_scheduled_sends_list", () => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            ui: {
              ...current.state.ui,
              scheduled_sends_list: {
                kind: "open" as const,
                scope: { kind: "space" as const, space_id: spaceId },
                capability: "localFallback" as const,
                items: [
                  {
                    scheduled_id: itemId,
                    room_id: "!harness-room:example.invalid",
                    body: itemBody,
                    send_at_ms: 4_000_000_000_000,
                    handle: { kind: "local" as const }
                  }
                ]
              }
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      });
    },
    { body, id, spaceId: HARNESS_SPACE_ID }
  );
}

/**
 * Publish an explicit Rust-shaped Open projection and settle a deferred
 * `open_scheduled_sends_list` with a receipt at the published generation.
 * `deferCommand` bypasses the harness response normalizer, so the test has to
 * land the value in the store and hand back the matching receipt itself.
 */
async function landDeferredOpenProjection(page: Page, body: string, id: string): Promise<void> {
  await page.evaluate(
    ({ body: itemBody, id: itemId, spaceId }) => {
      const current = window.__harness.currentSnapshot();
      const next = {
        ...current,
        state: {
          ...current.state,
          ui: {
            ...current.state.ui,
            scheduled_sends_list: {
              kind: "open" as const,
              scope: { kind: "space" as const, space_id: spaceId },
              capability: "localFallback" as const,
              items: [
                {
                  scheduled_id: itemId,
                  room_id: "!harness-room:example.invalid",
                  body: itemBody,
                  send_at_ms: 4_000_000_000_000,
                  handle: { kind: "local" as const }
                }
              ]
            }
          }
        }
      };
      window.__harness.setSnapshot(next);
      window.__harness.pushStateUpdate();
      window.__harness.resolveDeferredCommand("open_scheduled_sends_list", 0, {
        protocolVersion: 1,
        publishedGeneration: window.__harness.currentSnapshot().state_generation ?? 0
      });
    },
    { body, id, spaceId: HARNESS_SPACE_ID }
  );
}

test("a fresh renderer retires a stale Open projection and keeps the reservations", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await registerProjectionClose(page);
  await page.evaluate(async (tabs) => {
    window.__harness.setCommandResponse("select_account_tab", ({ tabId }: { tabId: string }) => ({
      ...tabs,
      selectedTabId: tabId
    }));
    await window.__harness.pushAccountTabs(tabs);
  }, accountTabs(HARNESS_TAB_ID));

  // Select Bob so the Harness renderer unmounts, then seed the Harness runtime
  // with a stale Open projection while nothing is watching it.
  await page.getByRole("button", { name: "Bob: Ready", exact: true }).click();
  await expect(page.getByRole("button", { name: "Bob: Ready", exact: true }))
    .toHaveAttribute("aria-current", "page");
  await page.evaluate((body) => {
    const current = window.__harness.currentSnapshot();
    const item = {
      scheduled_id: "harness-stale",
      room_id: "!harness-room:example.invalid",
      body,
      send_at_ms: 4_000_000_000_000,
      handle: { kind: "local" as const }
    };
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          scheduled_sends_list: {
            kind: "open" as const,
            scope: { kind: "space" as const, space_id: "!harness-space:example.invalid" },
            capability: "localFallback" as const,
            items: [item]
          },
          // A reservation the queue still holds while the projection is retired.
          timeline: { ...current.state.ui.timeline, scheduled_sends: [item] }
        }
      }
    });
  }, ALPHA_BODY);
  await page.evaluate(() => window.__harness.clearInvocations());

  // Reattach the Harness renderer: it starts with the panel closed but the
  // runtime still reports Open, so the account-bound close must retire it.
  await page.getByRole("button", { name: "Harness: Ready", exact: true }).click();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("close_scheduled_sends_list").length))
    .toBeGreaterThan(0);
  await expect.poll(() => projectionKind(page)).toBe("closed");
  await expect(
    page.getByRole("button", { name: t("action.close", { title: t("scheduled.title") }) })
  ).toHaveCount(0);
  expect(await queueReservationCount(page)).toBeGreaterThan(0);

  // Switching away and back preserves the reservation but not the projection.
  await page.getByRole("button", { name: "Bob: Ready", exact: true }).click();
  await page.getByRole("button", { name: "Harness: Ready", exact: true }).click();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect.poll(() => projectionKind(page)).toBe("closed");
  expect(await queueReservationCount(page)).toBeGreaterThan(0);
});

test("a Rust Closed replacement retires the projection and removes the bodies", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await registerProjectionClose(page);
  await registerOpenProjection(page, ALPHA_BODY, "synthetic-retire");

  await page.locator('[data-header-action="scheduled"]').click();
  await expect(page.getByText(ALPHA_BODY)).toBeVisible();
  expect(await projectionKind(page)).toBe("open");

  // Rust retires the projection with an explicit delta; the renderer must drop
  // both the projection value and the DOM bodies.
  await pushDelta(page, { state: { ui: { scheduled_sends_list: { kind: "closed" } } } });
  await expect.poll(() => projectionKind(page)).toBe("closed");
  await expect(page.getByText(ALPHA_BODY)).toHaveCount(0);
});

test("a delayed open does not override a newer Info intent and retires its projection", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await registerProjectionClose(page);
  await page.evaluate(() => {
    window.__harness.deferCommand("open_scheduled_sends_list");
    window.__harness.clearInvocations();
  });

  await page.locator('[data-header-action="scheduled"]').click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("open_scheduled_sends_list").length))
    .toBeGreaterThan(0);

  // A newer panel intent lands while the open is still pending.
  await page.locator('[data-header-action="info"]').click();
  await expect(page.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();

  await landDeferredOpenProjection(page, ALPHA_BODY, "synthetic-delayed-info");

  // The stale open neither resurrects the scheduled panel nor leaves the
  // projection open.
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("close_scheduled_sends_list").length))
    .toBeGreaterThan(0);
  await expect.poll(() => projectionKind(page)).toBe("closed");
  await expect(page.getByText(ALPHA_BODY)).toHaveCount(0);
  await expect(page.getByText(t("panel.spaceInfo"), { exact: true })).toBeVisible();
});

test("a delayed open does not survive a navigation transition", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await registerProjectionClose(page);
  await page.evaluate(() => {
    window.__harness.deferCommand("open_scheduled_sends_list");
    window.__harness.clearInvocations();
  });

  await page.locator('[data-header-action="scheduled"]').click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("open_scheduled_sends_list").length))
    .toBeGreaterThan(0);

  // Selecting a Space is a navigation transition that must fence the open.
  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Harness Space/ })
    .click();

  await landDeferredOpenProjection(page, ALPHA_BODY, "synthetic-delayed-nav");

  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("close_scheduled_sends_list").length))
    .toBeGreaterThan(0);
  await expect.poll(() => projectionKind(page)).toBe("closed");
  await expect(page.getByText(ALPHA_BODY)).toHaveCount(0);
});

test("a delayed open settles before the account switch and never reaches the new tab", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await registerProjectionClose(page);
  // Seed the Harness reservation queue so the old account keeps its data.
  await page.evaluate((body) => {
    const current = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          timeline: { ...current.state.ui.timeline, scheduled_sends: [{
            scheduled_id: "harness-reservation",
            room_id: "!harness-room:example.invalid",
            body,
            send_at_ms: 4_000_000_000_000,
            handle: { kind: "local" as const }
          }] }
        }
      }
    });
  }, BETA_BODY);
  await page.evaluate(async (tabs) => {
    window.__harness.setCommandResponse("select_account_tab", ({ tabId }: { tabId: string }) => {
      if (tabId === "bob-account-tab") {
        // Bob's runtime starts with the panel closed and no projection.
        const current2 = window.__harness.currentSnapshot();
        window.__harness.setSnapshot({
          ...current2,
          state: {
            ...current2.state,
            domain: {
              ...current2.state.domain,
              session: {
                kind: "ready" as const,
                homeserver: "https://bob.example.invalid",
                user_id: "@bob:example.invalid",
                device_id: "BOBDEVICE"
              }
            },
            ui: { ...current2.state.ui, scheduled_sends_list: { kind: "closed" as const } }
          }
        });
      }
      return { ...tabs, selectedTabId: tabId };
    });
    await window.__harness.pushAccountTabs(tabs);
  }, accountTabs(HARNESS_TAB_ID));
  await page.evaluate(() => {
    window.__harness.deferCommand("open_scheduled_sends_list");
    window.__harness.clearInvocations();
  });

  await page.locator('[data-header-action="scheduled"]').click();
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("open_scheduled_sends_list").length))
    .toBeGreaterThan(0);

  // Switch while the open is pending: the before-account-switch drain settles
  // the open on the previous account before the next tab is selected.
  await page.getByRole("button", { name: "Bob: Ready", exact: true }).click();
  await landDeferredOpenProjection(page, ALPHA_BODY, "synthetic-delayed-switch");

  await expect(page.getByRole("button", { name: "Bob: Ready", exact: true }))
    .toHaveAttribute("aria-current", "page");
  await expect.poll(() => page.evaluate(() => window.__harness.invocationsOf("close_scheduled_sends_list").length))
    .toBeGreaterThan(0);
  const closeTabIds = await page.evaluate(() =>
    window.__harness
      .invocations()
      .filter((call) => call.command === "close_scheduled_sends_list")
      .map((call) => call.args.accountTabId)
  );
  expect(closeTabIds).toContain(HARNESS_TAB_ID);
  // The delayed previous-account bodies never reach the new tab.
  await expect(page.getByText(ALPHA_BODY)).toHaveCount(0);
  await expect.poll(() => projectionKind(page)).toBe("closed");
});

test("session retirement closes the panel and removes a populated projection", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await gotoReadyShell(page);
  await registerOpenProjection(page, ALPHA_BODY, "synthetic-session");

  await page.locator('[data-header-action="scheduled"]').click();
  await expect(page.getByText(ALPHA_BODY)).toBeVisible();

  // Rust retires the session (auth failure) and emits the Closed replacement
  // in the same snapshot; the renderer must drop the bodies, not keep rendering
  // a projection that no longer belongs to a Ready session.
  await page.evaluate(() => {
    const current = window.__harness.currentSnapshot();
    const next = {
      ...current,
      state: {
        ...current.state,
        domain: {
          ...current.state.domain,
          session: { kind: "signedOut" as const }
        },
        ui: {
          ...current.state.ui,
          scheduled_sends_list: { kind: "closed" as const }
        }
      }
    };
    window.__harness.setSnapshot(next);
    window.__harness.pushStateUpdate();
  });

  await expect(page.getByText(ALPHA_BODY)).toHaveCount(0);
  await expect.poll(() => projectionKind(page)).toBe("closed");
});
