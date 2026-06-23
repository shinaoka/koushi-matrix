/**
 * Headless layout spec: proves that restored scroll positions are
 * re-corrected after fonts / layout settle (symptom B "微妙にずれる",
 * issue #123 Phase C).
 *
 * Scenario:
 *  1. Render a real TimelineView (via appHarness.html) taller than the
 *     viewport with 30 synthetic items.
 *  2. Scroll to a non-bottom position and note the anchor event's
 *     offsetTop from the container top.
 *  3. Inject a layout-shift spacer ABOVE the anchor row (simulating the
 *     font/media reflow that happens after a cross-session restore).  At
 *     this point a naive single restoreRoomScrollAnchor would place the
 *     row at (original offsetTop - spacerHeight) — i.e. would drift.
 *  4. Trigger the room-scroll-anchor restore path by setting
 *     room_scroll_anchors in the snapshot and simulating a room
 *     switch-and-back so the restore refs are cleared.
 *  5. Wait for the post-settle re-correction (rAF + fonts.ready) to fire.
 *  6. Assert the anchor's offsetTop from the container top is within ±2px
 *     of the ORIGINAL captured value — the re-correction compensated the
 *     spacer drift.
 *  7. Assert the naive-drift value (original - spacerHeight) is NOT the
 *     final value (proves the re-correction is needed and did fire).
 *
 * Private-data-free: all event ids, user ids, and bodies are synthetic.
 */

import { expect, test } from "@playwright/test";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const DUMMY_ROOM_ID = "!dummy-room:example.invalid";
const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const ANCHOR_PIXEL_TOLERANCE = 2;
// Height of the spacer injected to force the layout shift (px).
const SPACER_HEIGHT_PX = 40;
// Number of items to seed — enough to overflow a 600px viewport.
const ITEM_COUNT = 30;
// The anchor is the 10th item from the top (below the top edge, non-bottom).
const ANCHOR_INDEX = 9;

function makeEventItem(index: number) {
  const eventId = `$anchor-drift-item-${String(index).padStart(2, "0")}:example.invalid`;
  return {
    id: { Event: { event_id: eventId } },
    sender: "@sender:example.invalid",
    body: `drift-test message ${index}`,
    formatted: null,
    timestamp_ms: 1_800_000_000_000 + index * 1000,
    in_reply_to_event_id: null,
    reply_quote: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [],
    send_state: null,
    actions: null,
    thread_summary: null,
    media: null,
    sender_label: `Sender ${index}`,
    original_display_label: ""
  };
}

const ALL_ITEMS = Array.from({ length: ITEM_COUNT }, (_, i) => makeEventItem(i));
const ANCHOR_EVENT_ID = ALL_ITEMS[ANCHOR_INDEX].id.Event.event_id;
const TIMELINE_KEY = {
  account_key: HARNESS_ACCOUNT_KEY,
  kind: { Room: { room_id: HARNESS_ROOM_ID } }
};

test("post-settle re-correction compensates layout-shift drift after restore", async ({
  page
}) => {
  await page.goto("/appHarness.html");
  // Wait for the shell to be ready (the conversation timeline landmark is mounted).
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

  // --- 1. Seed the timeline with enough items to overflow the viewport ---
  await page.evaluate(
    ({ key, items }) => {
      void window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key,
            generation: 1,
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            items: items as any[]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: TIMELINE_KEY, items: ALL_ITEMS }
  );

  // Wait for the anchor event to be present.
  const anchorLocator = page.locator(`[data-event-id="${ANCHOR_EVENT_ID}"]`);
  await expect(anchorLocator).toBeVisible({ timeout: 5000 });

  // Scroll so the anchor row is near the top (not at the bottom).
  await page.evaluate(
    ({ eventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return;
      // Scroll so the anchor row is 80px from the container top.
      const containerTop = container.getBoundingClientRect().top;
      const anchorTop = anchor.getBoundingClientRect().top;
      container.scrollTop += anchorTop - containerTop - 80;
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
    },
    { eventId: ANCHOR_EVENT_ID }
  );

  // Let the scroll stabilize.
  await page.waitForTimeout(50);

  // --- 2. Capture the anchor's current offsetTop from the container top ---
  const capturedOffset = await page.evaluate(
    ({ eventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return null;
      return anchor.getBoundingClientRect().top - container.getBoundingClientRect().top;
    },
    { eventId: ANCHOR_EVENT_ID }
  );
  expect(capturedOffset).not.toBeNull();
  const originalOffsetPx = Math.round(capturedOffset!);

  // --- 3. Inject a spacer above the anchor to simulate a layout shift ---
  // This spacer makes a naive single-correction land SPACER_HEIGHT_PX too low.
  await page.evaluate(
    ({ eventId, spacerPx }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      if (!anchor) return;
      const spacer = document.createElement("div");
      spacer.id = "qa-drift-test-spacer";
      spacer.style.height = `${spacerPx}px`;
      spacer.style.background = "transparent";
      anchor.parentElement?.insertBefore(spacer, anchor);
    },
    { eventId: ANCHOR_EVENT_ID, spacerPx: SPACER_HEIGHT_PX }
  );

  // Confirm the spacer shifted the anchor down by SPACER_HEIGHT_PX.
  const shiftedOffset = await page.evaluate(
    ({ eventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return null;
      return anchor.getBoundingClientRect().top - container.getBoundingClientRect().top;
    },
    { eventId: ANCHOR_EVENT_ID }
  );
  // After spacer injection the row has shifted down by ~SPACER_HEIGHT_PX.
  expect(shiftedOffset).not.toBeNull();
  expect(Math.abs(shiftedOffset! - (originalOffsetPx + SPACER_HEIGHT_PX))).toBeLessThanOrEqual(2);

  // --- 4. Trigger room-scroll-anchor restore ---
  // Set room_scroll_anchors in the snapshot so RestoreRoomScrollAnchor fires.
  // First switch to a dummy room (clears restoredRoomScrollAnchorSignatureRef),
  // then switch back with room_scroll_anchors set.
  await page.evaluate(
    ({ roomId, dummyRoomId, eventId, offsetPx }) => {
      const snap = window.__harness.currentSnapshot();
      // Switch to dummy room to reset restore refs.
      window.__harness.setSnapshot({
        ...snap,
        state: {
          ...snap.state,
          ui: {
            ...snap.state.ui,
            navigation: {
              ...snap.state.ui.navigation,
              active_room_id: dummyRoomId,
              room_scroll_anchors: {}
            }
          }
        }
      });
      window.__harness.pushStateChanged();
      // After one microtask, switch back with the anchor set.
      Promise.resolve().then(() => {
        const s = window.__harness.currentSnapshot();
        window.__harness.setSnapshot({
          ...s,
          state: {
            ...s.state,
            ui: {
              ...s.state.ui,
              navigation: {
                ...s.state.ui.navigation,
                active_room_id: roomId,
                room_scroll_anchors: {
                  [roomId]: {
                    event_id: eventId,
                    offset_px: offsetPx,
                    updated_at_ms: Date.now()
                  }
                }
              },
              // Keep timeline pointing at the right room.
              timeline: {
                ...s.state.ui.timeline,
                room_id: roomId
              }
            }
          }
        });
        window.__harness.pushStateChanged();
      });
    },
    {
      roomId: HARNESS_ROOM_ID,
      dummyRoomId: DUMMY_ROOM_ID,
      eventId: ANCHOR_EVENT_ID,
      offsetPx: originalOffsetPx
    }
  );

  // --- 5. Wait for the post-settle re-correction to complete ---
  // The re-correction fires async (rAF + fonts.ready + ~250ms media timeout).
  // Poll until the anchor's offsetTop stabilises within ±2px of originalOffsetPx.
  // Poll until Math.abs(offset - originalOffsetPx) <= ANCHOR_PIXEL_TOLERANCE.
  // We encode the check as a boolean so .toBe(true) works in older Playwright.
  await expect
    .poll(
      async () => {
        const offset = await page.evaluate(
          ({ eventId }) => {
            const anchor = document.querySelector<HTMLElement>(
              `[data-event-id="${eventId}"]`
            );
            const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
            if (!anchor || !container) return null;
            return anchor.getBoundingClientRect().top - container.getBoundingClientRect().top;
          },
          { eventId: ANCHOR_EVENT_ID }
        );
        if (offset === null) return false;
        return Math.abs(offset - originalOffsetPx) <= ANCHOR_PIXEL_TOLERANCE;
      },
      { timeout: 3000, intervals: [50, 100, 200] }
    )
    .toBe(true);

  // --- 6. Confirm the final offset is NOT the naive-drift value ---
  // A naive single correction (without post-settle) would land at
  // (originalOffsetPx - SPACER_HEIGHT_PX) because the spacer was already
  // present when the initial restoreRoomScrollAnchor ran.  The post-settle
  // re-correction must produce a value close to originalOffsetPx instead.
  const finalOffset = await page.evaluate(
    ({ eventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return null;
      return anchor.getBoundingClientRect().top - container.getBoundingClientRect().top;
    },
    { eventId: ANCHOR_EVENT_ID }
  );

  expect(finalOffset).not.toBeNull();
  // Within ±ANCHOR_PIXEL_TOLERANCE of the original pre-spacer position.
  expect(Math.abs(finalOffset! - originalOffsetPx)).toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
  // Clearly NOT at the naive-drift value.
  const naiveDriftOffset = originalOffsetPx - SPACER_HEIGHT_PX;
  expect(Math.abs(finalOffset! - naiveDriftOffset)).toBeGreaterThan(ANCHOR_PIXEL_TOLERANCE);
});
