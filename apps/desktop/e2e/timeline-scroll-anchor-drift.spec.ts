/**
 * Headless layout spec: proves that restored scroll positions are
 * re-corrected after fonts / layout settle (symptom B "微妙にずれる",
 * issue #123 Phase C).
 *
 * Scenario (TRUE gate — confirmed RED without the fix):
 *  1. Render a real TimelineView (via appHarness.html) taller than the
 *     viewport with 30 synthetic items.
 *  2. Scroll so the anchor row is ~80px from the container top and capture
 *     that offset as originalOffsetPx.
 *  3. Patch document.fonts.ready to a DEFERRED promise so the post-settle
 *     re-correction is held open during the test.
 *  4. Trigger the room-scroll-anchor restore by pushing a snapshot update.
 *     The App's 250ms state-event debounce fires, React state updates, and
 *     the initial restoreRoomScrollAnchor correction runs in the layout
 *     effect — placing the anchor at ~originalOffsetPx.  The post-settle rAF
 *     then starts and suspends waiting on fonts.ready (which is deferred).
 *  5. Wait 350 ms so the debounce fires, the initial restore runs, and the
 *     post-settle rAF is now pending on fonts.ready before the layout shift is
 *     introduced.  Confirm the anchor is already at ~originalOffsetPx.
 *  6. WHILE fonts.ready is still pending, apply a CSS translateY transform to
 *     the ANCHOR ELEMENT itself to simulate a visual drift of DRIFT_HEIGHT_PX
 *     downward.  Using a CSS transform rather than layout height changes is
 *     intentional:
 *     - CSS transforms affect getBoundingClientRect() so the post-settle
 *       block sees a drifted position.
 *     - CSS transforms do NOT change layout flow, scrollTop, or trigger
 *       scroll events, so the user-scroll abort guard in
 *       applyPostSettleCorrection (containerSnapshot.scrollTop !==
 *       scrollTopAfterRestore) does NOT fire.
 *     - CSS transforms bypass the browser's native scroll anchoring (which
 *       only responds to layout/paint-phase changes that affect scrollHeight),
 *       so the drift is NOT silently compensated by the browser.
 *     This makes the drift visible only to applyPostSettleCorrection — the
 *     sole corrective path when the post-settle block is active.
 *  7. Resolve fonts.ready → applyPostSettleCorrection fires, reads the
 *     drifted getBoundingClientRect, and re-applies scrollTop to compensate.
 *     The CSS transform is still applied so we assert position with it.
 *  8. Assert the anchor's computed position (with transform still applied)
 *     is within ±2px of originalOffsetPx after the scrollTop correction.
 *
 * RED-without-fix confirmation: verified by temporarily setting the
 * post-settle block condition to `false &&` and running this spec.  Step 8
 * fails because the anchor stays at the drifted position (~originalOffsetPx
 * + DRIFT_HEIGHT_PX) with no correction path — the scrollTop adjustment
 * that would compensate the transform is never applied.  With the fix active
 * the spec is GREEN.  See the commit message for the explicit confirmation.
 *
 * Private-data-free: all event ids, user ids, and bodies are synthetic.
 */

import { expect, test } from "@playwright/test";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const DUMMY_ROOM_ID = "!dummy-room:example.invalid";
const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const ANCHOR_PIXEL_TOLERANCE = 2;
// Height by which the anchor is visually displaced (via CSS transform) to
// simulate post-restore font/media reflow drift.
const DRIFT_HEIGHT_PX = 30;
// Number of items to seed — enough to overflow a 600px viewport.
const ITEM_COUNT = 30;
// The anchor is the 10th item from the top (non-bottom, clearly visible).
const ANCHOR_INDEX = 9;
// The App uses a 250ms debounce before calling get_snapshot after a state
// event.  Wait this long (plus margin) to ensure the initial restore has
// already fired before the layout shift is introduced.
const DEBOUNCE_SETTLE_MS = 350;

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

/** Install a deferred document.fonts.ready; call window.__qa_resolveFontsReady() to release. */
async function installDeferredFontsReady(page: import("@playwright/test").Page): Promise<void> {
  await page.evaluate(() => {
    const realFontsReady = document.fonts.ready;
    let resolveDeferred: () => void;
    const deferred = new Promise<FontFaceSet>((resolve) => {
      resolveDeferred = () => void realFontsReady.then(resolve);
    });
    Object.defineProperty(document.fonts, "ready", {
      get: () => deferred,
      configurable: true
    });
    (window as unknown as Record<string, unknown>)["__qa_resolveFontsReady"] = () => {
      resolveDeferred();
    };
  });
}

test("post-settle re-correction compensates layout-shift drift after restore", async ({
  page
}) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

  // --- 1. Seed the timeline ---
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

  await expect(page.locator(`[data-event-id="${ANCHOR_EVENT_ID}"]`)).toBeVisible({
    timeout: 5000
  });

  // Scroll so the anchor row is ~80px from container top.
  await page.evaluate(
    ({ eventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return;
      const containerTop = container.getBoundingClientRect().top;
      const anchorTop = anchor.getBoundingClientRect().top;
      container.scrollTop += anchorTop - containerTop - 80;
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
    },
    { eventId: ANCHOR_EVENT_ID }
  );
  await page.waitForTimeout(30);

  // --- 2. Capture anchor position ---
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

  // --- 3. Patch fonts.ready to a deferred promise ---
  // Must be patched BEFORE the restore fires so the post-settle rAF chain
  // suspends on the deferred promise rather than resolving immediately.
  await installDeferredFontsReady(page);

  // --- 4. Trigger restore (queues the 250ms debounce) ---
  // Push a dummy-room state first (to clear any prior anchor), then push the
  // real anchor room in a microtask.  The debounce fires once, roughly 250ms
  // after the last pushStateChanged() call.
  await page.evaluate(
    ({ roomId, dummyRoomId, eventId, offsetPx }) => {
      const snap = window.__harness.currentSnapshot();
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
              timeline: { ...s.state.ui.timeline, room_id: roomId }
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

  // --- 5. Wait for the debounce + initial restore to complete ---
  // The App's 250ms state-event debounce fires and calls get_snapshot.
  // React updates, the layout effect runs restoreRoomScrollAnchor, places the
  // anchor at ~originalOffsetPx, and schedules the post-settle rAF chain.
  // The rAF fires and suspends on document.fonts.ready (which is deferred).
  // DEBOUNCE_SETTLE_MS > 250ms ensures this is fully settled before we
  // introduce the layout shift.
  await page.waitForTimeout(DEBOUNCE_SETTLE_MS);

  // Confirm the initial restore placed the anchor near originalOffsetPx.
  const afterInitialRestore = await page.evaluate(
    ({ eventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return null;
      return anchor.getBoundingClientRect().top - container.getBoundingClientRect().top;
    },
    { eventId: ANCHOR_EVENT_ID }
  );
  expect(afterInitialRestore).not.toBeNull();
  expect(Math.abs(afterInitialRestore! - originalOffsetPx)).toBeLessThanOrEqual(
    ANCHOR_PIXEL_TOLERANCE
  );

  // --- 6. Apply CSS transform drift WHILE fonts.ready is pending ---
  // The post-settle rAF chain is now suspended on fonts.ready.  The initial
  // restore has already run (restoredRoomScrollAnchorSignatureRef is set), so
  // the layout effect guard prevents another correction attempt.
  // Apply a translateY transform to simulate post-restore font/media reflow
  // drift.  CSS transforms:
  //   - affect getBoundingClientRect() so applyPostSettleCorrection sees drift,
  //   - do NOT change scrollTop so the user-scroll abort guard stays dormant,
  //   - bypass native browser scroll anchoring (no layout-flow changes).
  await page.evaluate(
    ({ eventId, driftPx }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      if (!anchor) return;
      anchor.style.transform = `translateY(${driftPx}px)`;
    },
    { eventId: ANCHOR_EVENT_ID, driftPx: DRIFT_HEIGHT_PX }
  );

  // Confirm the anchor's visual position has drifted beyond the ±2px tolerance.
  const afterDrift = await page.evaluate(
    ({ eventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return null;
      return anchor.getBoundingClientRect().top - container.getBoundingClientRect().top;
    },
    { eventId: ANCHOR_EVENT_ID }
  );
  expect(afterDrift).not.toBeNull();
  expect(Math.abs(afterDrift! - originalOffsetPx)).toBeGreaterThan(ANCHOR_PIXEL_TOLERANCE);

  // --- 7. Resolve fonts.ready → post-settle re-correction fires ---
  // applyPostSettleCorrection re-measures the anchor via getBoundingClientRect
  // (which includes the CSS transform, so it sees the drifted position) and
  // adjusts scrollTop by DRIFT_HEIGHT_PX to compensate.  After the correction,
  // the anchor's getBoundingClientRect().top (with transform still applied)
  // returns ~originalOffsetPx — this is what we assert in step 8.
  await page.evaluate(() => {
    const resolver = (window as unknown as Record<string, unknown>)["__qa_resolveFontsReady"];
    if (typeof resolver === "function") resolver();
  });

  // --- 8. Assert the anchor (with transform still applied) lands back within
  //        ±2px of originalOffsetPx after the scrollTop correction ---
  await expect
    .poll(
      async () => {
        const offset = await page.evaluate(
          ({ eventId }) => {
            const anchor = document.querySelector<HTMLElement>(`[data-event-id="${eventId}"]`);
            const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
            if (!anchor || !container) return null;
            return (
              anchor.getBoundingClientRect().top - container.getBoundingClientRect().top
            );
          },
          { eventId: ANCHOR_EVENT_ID }
        );
        if (offset === null) return false;
        return Math.abs(offset - originalOffsetPx) <= ANCHOR_PIXEL_TOLERANCE;
      },
      { timeout: 3000, intervals: [30, 50, 100, 200] }
    )
    .toBe(true);

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
  expect(Math.abs(finalOffset! - originalOffsetPx)).toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
  // Clearly NOT at the drifted position (which was DRIFT_HEIGHT_PX below original).
  expect(Math.abs(finalOffset! - afterDrift!)).toBeGreaterThan(ANCHOR_PIXEL_TOLERANCE);
});
