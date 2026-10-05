/**
 * Headless spec: the right panel's INNER layout stays inside the panel (#1119).
 *
 * With two signed-in accounts the shared composer toolbar names the sending
 * account by Matrix ID. A long ID used to widen the panel's implicit grid
 * column to its intrinsic width (~649px inside a 390px panel), so the thread
 * header, messages, and composer ran past the panel and the close button left
 * the window, although the panel's own rectangle still fitted (#452's checks
 * passed). The contract is engineering rules "Right Panel And Composer
 * Containment"; the plan is
 * docs/superpowers/plans/2026-10-05-issue1119-panel-containment.md.
 *
 * Every case opens an actual thread through the user's pill click, measures the
 * panel and descendant rectangles, hit-tests the close and send controls with
 * `elementFromPoint`, and closes the thread with a real pointer click at the
 * measured centre, asserting the typed `close_thread` command. All data is
 * synthetic.
 */

import { expect, test, type Page } from "@playwright/test";

import { threadTimelineKey } from "../src/domain/coreEvents";
import type { AccountTabsSnapshot } from "../src/domain/types";
import { t } from "../src/i18n/messages";
import { HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID, gotoReadyShell, invocationCount } from "./support/basicOperations";
import {
  RIGHT_PANEL_SELECTOR,
  containmentReport,
  dragResizer,
  expectContained,
  type ContainmentProbe
} from "./support/panelGeometry";

const HARNESS_ACCOUNT_TAB_ID = "harness-account-tab";
const SECOND_ACCOUNT_TAB_ID = "second-account-tab";
const LONG_ACCOUNT_ID =
  "@synthetic-long-account-name-for-panel-containment-regression:example.invalid";
const SECOND_ACCOUNT_ID = "@second-synthetic-account:example.invalid";
const THREAD_ROOT_EVENT_ID = "$seed-event:example.invalid";
const THREAD_KEY = threadTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID, THREAD_ROOT_EVENT_ID);
const LONG_REPLY_EVENT_ID = "$containment-long-reply:example.invalid";
const LONG_UNBROKEN_BODY = `synthetic${"unbroken".repeat(48)} https://example.invalid/${"segment-".repeat(30)}end`;
/** The inline grid needs 1200px (#452); below it the panel is an overlay. */
const INLINE_MIN_WIDTH = 1200;

const THREAD_PANEL: ContainmentProbe = {
  root: RIGHT_PANEL_SELECTOR,
  controls: {
    close: ".thread-header button",
    send: ".composer .send-button"
  },
  rows: [".composer-tools", ".composer-footer", ".thread-header"]
};

function accountTabs(signedIn: 1 | 2): AccountTabsSnapshot {
  return {
    selectedTabId: HARNESS_ACCOUNT_TAB_ID,
    tabs: [
      {
        id: HARNESS_ACCOUNT_TAB_ID,
        accountKey: LONG_ACCOUNT_ID,
        homeserver: "https://harness.example.invalid",
        displayName: LONG_ACCOUNT_ID,
        avatarSourceRef: null,
        status: "ready",
        unreadCount: 0
      },
      ...(signedIn === 2
        ? [
            {
              id: SECOND_ACCOUNT_TAB_ID,
              accountKey: SECOND_ACCOUNT_ID,
              homeserver: "https://second.example.invalid",
              displayName: "Second Synthetic",
              avatarSourceRef: null,
              status: "ready" as const,
              unreadCount: 0
            }
          ]
        : [])
    ],
    badgeCount: 0
  };
}

async function setSignedInAccounts(page: Page, signedIn: 1 | 2): Promise<void> {
  await page.evaluate((tabs) => {
    window.__harness.setCommandResponse("list_account_tabs", tabs);
    void window.__harness.pushAccountTabs(tabs);
  }, accountTabs(signedIn));
  // The shared composer names the sender only when more than one account is
  // signed in; the main composer is always mounted, so it is the barrier.
  await expect(page.locator(".main-pane .composer-sending-as")).toHaveCount(signedIn === 2 ? 1 : 0);
}

function longReplyItem() {
  return {
    id: { Event: { event_id: LONG_REPLY_EVENT_ID } },
    sender: "@synthetic-thread-member:example.invalid",
    sender_label: "Synthetic Thread Member With An Expanded Display Name For Width",
    body: LONG_UNBROKEN_BODY,
    timestamp_ms: 1_800_000_100_000,
    in_reply_to_event_id: THREAD_ROOT_EVENT_ID,
    thread_root: THREAD_ROOT_EVENT_ID,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: true,
    is_edited: false,
    can_edit: true
  };
}

/** Opens the seeded thread with a real pill click and seeds a long unbroken reply. */
async function openThread(page: Page): Promise<void> {
  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect.poll(() => invocationCount(page, "open_thread")).toBe(1);
  const panel = page.locator(RIGHT_PANEL_SELECTOR);
  await expect(panel.locator(".thread-header")).toHaveCount(1);
  await expect(panel.locator(".composer .send-button")).toHaveCount(1);
  await expect
    .poll(() =>
      page.evaluate(
        async ({ key, items, eventId }) => {
          await window.__harness.pushCoreEvent({
            kind: "Timeline",
            event: { InitialItems: { request_id: null, key, generation: 2, items } }
            // The fixture delivers the public CoreEvent wire payload.
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
          } as any);
          return Boolean(
            document.querySelector(`.thread-pane [data-event-id="${CSS.escape(eventId)}"]`)
          );
        },
        { key: THREAD_KEY, items: [longReplyItem()], eventId: LONG_REPLY_EVENT_ID }
      )
    )
    .toBe(true);
}

function expectedLayout(width: number): "inline" | "overlay" {
  return width >= INLINE_MIN_WIDTH ? "inline" : "overlay";
}

/** Inner fit of the thread panel plus the truncated, fully named sending identity. */
async function expectThreadContained(page: Page, width: number, signedIn: 1 | 2): Promise<void> {
  const report = await expectContained(
    page,
    THREAD_PANEL,
    `thread panel inner fit at ${width}px with ${signedIn} account(s)`
  );
  expect(report.layout, `layout at ${width}px`).toBe(expectedLayout(width));
  const identity = page.locator(`${RIGHT_PANEL_SELECTOR} .composer-sending-as`);
  if (signedIn === 1) {
    await expect(identity).toHaveCount(0);
    return;
  }
  // Readable but truncated in place: the visible ID has a box, is clipped with
  // an ellipsis rather than widening the toolbar, and the full ID stays
  // available through the title and accessible name.
  const fit = await identity.evaluate((element) => {
    const id = element.querySelector<HTMLElement>(".composer-sending-as-id")!;
    const toolbar = element.closest<HTMLElement>(".composer-tools")!;
    const rect = id.getBoundingClientRect();
    const toolbarRect = toolbar.getBoundingClientRect();
    return {
      width: rect.width,
      truncated: id.scrollWidth > id.clientWidth,
      textOverflow: getComputedStyle(id).textOverflow,
      insideToolbar: rect.left >= toolbarRect.left - 1 && rect.right <= toolbarRect.right + 1,
      title: element.getAttribute("title"),
      label: element.getAttribute("aria-label")
    };
  });
  expect(fit.width).toBeGreaterThan(0);
  expect(fit.truncated).toBe(true);
  expect(fit.textOverflow).toBe("ellipsis");
  expect(fit.insideToolbar).toBe(true);
  expect(fit.title).toContain(LONG_ACCOUNT_ID);
  expect(fit.label).toContain(LONG_ACCOUNT_ID);
}

/** Closes the thread with a pointer click at the measured close-button centre. */
async function closeThreadByPointer(page: Page): Promise<void> {
  const before = await invocationCount(page, "close_thread");
  const report = await containmentReport(page, THREAD_PANEL);
  const close = report.hitTargets.close;
  expect(close).toBeDefined();
  await page.mouse.click(close.centerX, close.centerY);
  await expect.poll(() => invocationCount(page, "close_thread")).toBe(before + 1);
  await expect(page.locator(`${RIGHT_PANEL_SELECTOR} .thread-header`)).toHaveCount(0);
}

async function setViewportWidth(page: Page, width: number): Promise<void> {
  await page.setViewportSize({ width, height: 800 });
}

for (const width of [800, 1100, 1190, 1200, 1400]) {
  test(`a thread with two accounts and a long sending ID stays inside the panel at ${width}px`, async ({
    page
  }) => {
    await setViewportWidth(page, width);
    await gotoReadyShell(page);
    await setSignedInAccounts(page, 2);
    await openThread(page);

    await expectThreadContained(page, width, 2);
    await closeThreadByPointer(page);
  });
}

test("inner containment holds across the compact, overlay, and inline breakpoint boundaries", async ({
  page
}) => {
  await setViewportWidth(page, 1400);
  await gotoReadyShell(page);
  await setSignedInAccounts(page, 2);
  await openThread(page);

  // 760px is the native minimum window width (tauri.conf.json) and the compact
  // overlay breakpoint; 1199/1200 bound the inline grid minimum (#452).
  for (const width of [1400, 1201, 1200, 1199, 1181, 1180, 1100, 900, 800, 761, 760]) {
    await setViewportWidth(page, width);
    await expectThreadContained(page, width, 2);
  }
  await closeThreadByPointer(page);
});

test("narrow, wide, then narrow again keeps the thread panel contained", async ({ page }) => {
  await setViewportWidth(page, 800);
  await gotoReadyShell(page);
  await setSignedInAccounts(page, 2);
  await openThread(page);

  for (const width of [800, 1400, 800]) {
    await setViewportWidth(page, width);
    await expectThreadContained(page, width, 2);
  }
  await closeThreadByPointer(page);
});

for (const width of [1100, 1400]) {
  test(`switching between one and two signed-in accounts keeps the thread contained at ${width}px`, async ({
    page
  }) => {
    await setViewportWidth(page, width);
    await gotoReadyShell(page);
    await setSignedInAccounts(page, 1);
    await openThread(page);
    await expectThreadContained(page, width, 1);

    await setSignedInAccounts(page, 2);
    await expectThreadContained(page, width, 2);

    await setSignedInAccounts(page, 1);
    await expectThreadContained(page, width, 1);
    await closeThreadByPointer(page);
  });
}

test("dragging the right panel wider and narrower keeps the thread contained", async ({ page }) => {
  await setViewportWidth(page, 1600);
  await gotoReadyShell(page);
  await setSignedInAccounts(page, 2);
  await openThread(page);
  await expectThreadContained(page, 1600, 2);

  const panel = page.locator(RIGHT_PANEL_SELECTOR);
  const panelWidth = () => panel.evaluate((element) => element.getBoundingClientRect().width);
  const initialWidth = await panelWidth();
  await dragResizer(page, t("workspace.resizeRightPanel"), -260);
  await expect.poll(panelWidth).toBeGreaterThan(initialWidth);
  await expectThreadContained(page, 1600, 2);

  const widened = await panelWidth();
  await dragResizer(page, t("workspace.resizeRightPanel"), 400);
  await expect.poll(panelWidth).toBeLessThan(widened);
  await expectThreadContained(page, 1600, 2);
  await closeThreadByPointer(page);
});

test("the main composer keeps its send control inside the conversation pane with two accounts", async ({
  page
}) => {
  for (const width of [800, 1190, 1400]) {
    await setViewportWidth(page, width);
    if (width === 800) {
      await gotoReadyShell(page);
      await setSignedInAccounts(page, 2);
    }
    await expectContained(
      page,
      {
        root: ".main-pane .composer",
        controls: { send: ".send-button" },
        rows: [".composer-tools", ".composer-footer"]
      },
      `main composer inner fit at ${width}px`
    );
  }
});

/*
 * Deterministic pairwise matrix for the secondary factors (#1121 Phase B):
 * every density x locale pair appears once, and each density and each locale
 * meets both the inline and the overlay layout. Pseudo-accented labels stand
 * in for expanded localized text; the bidi pseudo locale runs RTL.
 */
const LOCALES = {
  ltr: {
    lang: "en",
    dir: "ltr",
    catalog_locale: "en",
    pseudo_locale: "none"
  },
  accented: {
    lang: "en-XA",
    dir: "ltr",
    catalog_locale: "pseudo",
    pseudo_locale: "accented"
  },
  rtl: {
    lang: "ar-XB",
    dir: "rtl",
    catalog_locale: "pseudo",
    pseudo_locale: "bidi"
  }
} as const;

const PAIRWISE: Array<{
  density: "compact" | "default" | "comfortable";
  locale: keyof typeof LOCALES;
  width: number;
}> = [
  { density: "compact", locale: "ltr", width: 1100 },
  { density: "compact", locale: "accented", width: 1400 },
  { density: "compact", locale: "rtl", width: 800 },
  { density: "default", locale: "ltr", width: 1200 },
  { density: "default", locale: "accented", width: 1190 },
  { density: "default", locale: "rtl", width: 1400 },
  { density: "comfortable", locale: "ltr", width: 800 },
  { density: "comfortable", locale: "accented", width: 1200 },
  { density: "comfortable", locale: "rtl", width: 1100 }
];

for (const { density, locale, width } of PAIRWISE) {
  test(`pairwise: ${density} density, ${locale} locale, ${width}px keeps the thread contained`, async ({
    page
  }) => {
    await setViewportWidth(page, width);
    await gotoReadyShell(page);
    await setSignedInAccounts(page, 2);
    await openThread(page);

    await page.evaluate(
      ({ nextDensity, profile }) => {
        const snapshot = window.__harness.currentSnapshot();
        window.__harness.setSnapshot({
          ...snapshot,
          state: {
            ...snapshot.state,
            domain: {
              ...snapshot.state.domain,
              locale_profile: {
                ...snapshot.state.domain.locale_profile,
                ...profile
              },
              settings: {
                ...snapshot.state.domain.settings,
                values: {
                  ...snapshot.state.domain.settings.values,
                  appearance: {
                    ...snapshot.state.domain.settings.values.appearance,
                    density: nextDensity
                  }
                }
              }
            }
          }
        });
        window.__harness.pushStateUpdate();
      },
      { nextDensity: density, profile: LOCALES[locale] }
    );
    await expect
      .poll(() => page.evaluate(() => document.documentElement.dir))
      .toBe(LOCALES[locale].dir);
    await expect(page.locator(`[data-density="${density}"]`)).toHaveCount(1);

    await expectThreadContained(page, width, 2);
    await closeThreadByPointer(page);
  });
}

/*
 * Shared right-panel modes: the same shell must contain every mode. These open
 * through the conversation header (or search box) and close through the
 * panel's own close button hit-tested at its measured centre.
 */
const PANEL_MODES: Array<{ name: string; open: (page: Page) => Promise<void> }> = [
  {
    name: "room info",
    open: (page) => page.getByRole("button", { name: t("room.roomInfo"), exact: true }).click()
  },
  {
    name: "people",
    open: (page) => page.getByRole("button", { name: t("panel.people"), exact: true }).click()
  },
  {
    name: "threads list",
    open: async (page) => {
      // The harness has no default threads-list response; mirror Rust's
      // open_threads_list projection (activity-files-threads-navigation.spec).
      await page.evaluate((roomId) => {
        window.__harness.setCommandResponse(
          "open_threads_list",
          ({ scope }: { scope: { kind: string; room_id?: string } }) => {
            const current = window.__harness.currentSnapshot();
            const next = {
              ...current,
              state: {
                ...current.state,
                ui: {
                  ...current.state.ui,
                  threads_list: {
                    kind: "open",
                    room_id: scope.room_id ?? roomId,
                    request_id: 1,
                    items: [],
                    is_paginating: false,
                    end_reached: true
                  }
                }
              }
            };
            // The fixture mirrors the Rust snapshot shape.
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            window.__harness.setSnapshot(next as any);
            return next;
          }
        );
        window.__harness.setCommandResponse("close_threads_list", () => {
          const current = window.__harness.currentSnapshot();
          const next = {
            ...current,
            state: { ...current.state, ui: { ...current.state.ui, threads_list: { kind: "closed" } } }
          };
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          window.__harness.setSnapshot(next as any);
          return next;
        });
      }, HARNESS_ROOM_ID);
      await page
        .locator(".channel-actions")
        .getByRole("button", { name: t("workspace.threads"), exact: true })
        .click();
    }
  }
];

for (const mode of PANEL_MODES) {
  for (const width of [1100, 1400]) {
    test(`the ${mode.name} panel stays contained with two accounts at ${width}px`, async ({ page }) => {
      await setViewportWidth(page, width);
      await gotoReadyShell(page);
      await setSignedInAccounts(page, 2);
      await mode.open(page);

      const probe: ContainmentProbe = {
        root: RIGHT_PANEL_SELECTOR,
        controls: { close: ".thread-header button, header button[aria-label]" }
      };
      const report = await expectContained(page, probe, `${mode.name} inner fit at ${width}px`);
      expect(report.layout).toBe(expectedLayout(width));
      const close = report.hitTargets.close;
      await page.mouse.click(close.centerX, close.centerY);
      await expect(page.locator(".app-grid.right-panel-open")).toHaveCount(0);
    });
  }
}
