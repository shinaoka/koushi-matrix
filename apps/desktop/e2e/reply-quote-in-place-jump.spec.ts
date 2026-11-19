/**
 * Headless spec: activating a reply quote (#1981). Core decides whether the
 * quoted original is already a row of the live room timeline; when it is, the
 * navigation settles in place and the live view scrolls to that row instead of
 * opening a focused timeline that swaps the pane and back (#1982).
 */

import { expect, test, type Page } from "@playwright/test";
import { focusedTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";
import {
  HARNESS_ACCOUNT_KEY,
  HARNESS_ROOM_ID,
  HARNESS_ROOM_KEY,
  gotoReadyShell,
  seedTimelineItems
} from "./support/basicOperations";
import { pushSnapshot } from "./support/stateUpdates";

const QUOTED_EVENT_ID = "$quoted-original:example.invalid";
const QUOTER_EVENT_ID = "$quoter:example.invalid";
const QUOTED_BODY = "The original message a reply quotes";
const LIVE_ROW_COUNT = 40;
const QUOTED_ROW_INDEX = 8;
const FOCUSED_ROW_COUNT = 30;

const jumpToLatest = () =>
  t("shortcut.jumpToLatestMessage");

function item(eventId: string, index: number, overrides: Record<string, unknown> = {}) {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-bob:example.invalid",
    sender_label: "Harness Bob",
    body: `Row ${index}`,
    timestamp_ms: 1_800_000_000_000 + index * 60_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: true,
    is_edited: false,
    can_edit: true,
    ...overrides
  };
}

/** The reply quote the renderer is allowed to activate. */
function openableQuote(): Record<string, unknown> {
  return {
    event_id: QUOTED_EVENT_ID,
    sender: "@harness-bob:example.invalid",
    sender_label: "Harness Bob",
    body_preview: QUOTED_BODY,
    formatted: null,
    state: "ready",
    permalink: `https://matrix.to/#/${HARNESS_ROOM_ID}/${QUOTED_EVENT_ID}`,
    can_open: true
  };
}

function liveRows(): unknown[] {
  return Array.from({ length: LIVE_ROW_COUNT }, (_unused, index) => {
    if (index === QUOTED_ROW_INDEX) {
      return item(QUOTED_EVENT_ID, index, { body: QUOTED_BODY });
    }
    if (index === LIVE_ROW_COUNT - 1) {
      return item(QUOTER_EVENT_ID, index, {
        body: "Replying to the original",
        in_reply_to_event_id: QUOTED_EVENT_ID,
        reply_quote: openableQuote()
      });
    }
    return item(`$row-${index}:example.invalid`, index);
  });
}

/**
 * A focused window long enough that its target has to be placed, not just
 * shown. It is pushed rather than seeded through `seedTimelineItems`, whose
 * predicate also wants the batch's tail mounted: the focused placement centers
 * the target and unmounts that tail.
 */
function focusedRows(): unknown[] {
  return Array.from({ length: FOCUSED_ROW_COUNT }, (_unused, index) =>
    item(
      index === QUOTED_ROW_INDEX
        ? QUOTED_EVENT_ID
        : `$focused-${index}:example.invalid`,
      index,
      index === QUOTED_ROW_INDEX ? { body: QUOTED_BODY } : {}
    )
  );
}

/**
 * Click the quote, then answer `open_reply_quote_event` the way Core does:
 * the navigation arrives as a pushed snapshot and the command settles with that
 * snapshot's generation.
 */
async function activateQuote(page: Page, outcome: "inPlace" | "focused"): Promise<void> {
  await page.evaluate(() => window.__harness.clearInvocations());
  await expect
    .poll(() =>
      page.evaluate(() => {
        const view = document.querySelector('[data-testid="timeline-view"]') as HTMLElement;
        return Math.round(view.scrollTop + view.clientHeight) >= view.scrollHeight;
      })
    )
    .toBe(true);
  await page.evaluate(() => window.__harness.deferCommand("open_reply_quote_event"));
  await page.locator(`[data-item-id="${QUOTER_EVENT_ID}"] .reply-quote`).click();
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("open_reply_quote_event")[0]?.args))
    .toEqual({ roomId: HARNESS_ROOM_ID, eventId: QUOTED_EVENT_ID });

  const base = await page.evaluate(() => window.__harness.currentSnapshot());
  const next = structuredClone(base) as typeof base;
  const generation =
    base.state.ui.navigation.event_navigation.kind === "idle"
      ? 1
      : base.state.ui.navigation.event_navigation.generation + 1;
  next.state.ui.navigation.active_room_id = HARNESS_ROOM_ID;
  if (outcome === "inPlace") {
    next.state.ui.navigation.event_navigation = {
      kind: "inPlace",
      generation,
      source: "replyQuote",
      eventId: QUOTED_EVENT_ID
    };
  } else {
    next.state.ui.navigation.main_timeline_anchor = { event_id: QUOTED_EVENT_ID };
    next.state.ui.navigation.event_navigation = {
      kind: "anchored",
      generation,
      source: "replyQuote"
    };
    next.state.ui.focused_context = {
      kind: "open",
      room_id: HARNESS_ROOM_ID,
      event_id: QUOTED_EVENT_ID,
      is_subscribed: true
    };
  }
  const published = await pushSnapshot(page, next);
  await page.evaluate((publishedGeneration) => {
    window.__harness.resolveDeferredCommand("open_reply_quote_event", 0, {
      protocolVersion: 1,
      publishedGeneration
    });
  }, published);
}

/** The timeline's own geometry, so a "is it on screen" claim is measurable. */
async function timelineGeometry(page: Page, itemId: string) {
  return page.evaluate((id) => {
    const view = document.querySelector('[data-testid="timeline-view"]') as HTMLElement;
    const row = document.querySelector(`[data-item-id="${CSS.escape(id)}"]`) as HTMLElement | null;
    const viewRect = view.getBoundingClientRect();
    const rowRect = row?.getBoundingClientRect() ?? null;
    return {
      scrollTop: Math.round(view.scrollTop),
      scrollBottom: Math.round(view.scrollTop + view.clientHeight),
      scrollHeight: Math.round(view.scrollHeight),
      totalItems: view.dataset.totalItems,
      viewTop: Math.round(viewRect.top),
      viewBottom: Math.round(viewRect.bottom),
      rowTop: rowRect === null ? null : Math.round(rowRect.top),
      rowBottom: rowRect === null ? null : Math.round(rowRect.bottom),
      targetMarked: row?.classList.contains("pinned-target") ?? false
    };
  }, itemId);
}

test("a reply quote already in the live window settles without swapping the pane", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, liveRows());
  const before = await timelineGeometry(page, QUOTED_EVENT_ID);
  // The live view sits at the live edge: the original is outside the mounted
  // window and the timeline has room to move.
  expect(before.totalItems).toBe(String(LIVE_ROW_COUNT));
  expect(before.rowTop).toBeNull();

  await activateQuote(page, "inPlace");

  await expect
    .poll(async () => (await timelineGeometry(page, QUOTED_EVENT_ID)).totalItems)
    .toBe(String(LIVE_ROW_COUNT));
  const after = await timelineGeometry(page, QUOTED_EVENT_ID);
  // The live view moved off the live edge towards the original, and the
  // original's row became part of the mounted window and is marked as the
  // navigation target.
  expect(after.scrollTop).toBeLessThan(before.scrollTop - 200);
  expect(after.rowTop).not.toBeNull();
  expect(after.targetMarked).toBe(true);
  // #1987: the settle exists so the reader arrives at the quoted message, so the
  // row's box has to sit inside the timeline's visible area instead of just
  // above it. The reported geometry keeps a failure readable.
  await expect
    .poll(async () => {
      const geometry = await timelineGeometry(page, QUOTED_EVENT_ID);
      const { rowTop, rowBottom, viewTop, viewBottom } = geometry;
      if (rowTop === null || rowBottom === null) {
        return "the quoted row is not mounted";
      }
      return rowTop >= viewTop && rowBottom <= viewBottom
        ? "visible"
        : `row ${rowTop}..${rowBottom} outside ${viewTop}..${viewBottom}`;
    })
    .toBe("visible");
  // No anchored control: nothing was anchored, so there is nothing to return
  // from, and the right panel is untouched.
  await expect(page.locator(".timeline-navigation-pill", { hasText: jumpToLatest() })).toHaveCount(
    0
  );
});

test("a reply quote outside the live window opens the focused context", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, liveRows());

  await activateQuote(page, "focused");
  // The focused timeline is only on screen once the pane swapped, so its rows
  // arrive afterwards: the target still has to be placed inside them.
  await page.evaluate(
    async ({ key, items }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: { InitialItems: { request_id: null, key, generation: 2, items } }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: focusedTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID, QUOTED_EVENT_ID),
      items: focusedRows()
    }
  );
  await expect
    .poll(async () => (await timelineGeometry(page, QUOTED_EVENT_ID)).totalItems)
    .toBe(String(FOCUSED_ROW_COUNT));

  // The focused path is unchanged: the pane swapped to the focused timeline
  // (a different row set), its anchored control can return to the live view,
  // and the original is centered in what the reader sees.
  const focused = await timelineGeometry(page, QUOTED_EVENT_ID);
  expect(focused.totalItems).toBe(String(FOCUSED_ROW_COUNT));
  await expect(page.locator(".timeline-navigation-pill", { hasText: jumpToLatest() })).toHaveCount(
    1
  );
  await expect
    .poll(async () => {
      const geometry = await timelineGeometry(page, QUOTED_EVENT_ID);
      if (geometry.rowTop === null || geometry.rowBottom === null) {
        return false;
      }
      const rowCenter = (geometry.rowTop + geometry.rowBottom) / 2;
      const viewCenter = (geometry.viewTop + geometry.viewBottom) / 2;
      const viewHeight = geometry.viewBottom - geometry.viewTop;
      return Math.abs(rowCenter - viewCenter) <= viewHeight * 0.35;
    })
    .toBe(true);
});
