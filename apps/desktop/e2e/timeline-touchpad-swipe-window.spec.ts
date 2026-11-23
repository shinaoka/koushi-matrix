/**
 * White blocks while swiping through history on Linux WebKitGTK.
 *
 * On Linux the timeline resamples touchpad input onto frames and moves the
 * timeline itself, inside an animation frame (`touchpadScrollResampler.ts`,
 * #2020). The timeline mounts the visible rows plus one screen of overscan on
 * each side (#1890); everything beyond that is a `.timeline-virtual-spacer`,
 * which paints as an empty block.
 *
 * Each resampled step marks user input, which advances the viewport epoch and
 * cancels pending viewport frames. The commit frame that the previous step's
 * scroll event scheduled runs after the next resampled step in the same frame,
 * so it was cancelled every frame: during a continuous swipe the mounted window
 * never moved and the reader scrolled into the spacer. Native wheel scrolling
 * never hit this, because the browser dispatches the scroll event before
 * animation-frame callbacks.
 *
 * The second test adds the 536 ms renderer frame a real Linux session
 * recorded mid-swipe. The finger keeps moving during it, so the queued input
 * lands as one large catch-up step; the reader must still never see unmounted
 * space.
 */
import { expect, test, type Page } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_SEED_EVENT_ID = "$seed-event:example.invalid";
const TIMELINE_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);
// The user agent Koushi's WebKitGTK webview reports on Linux, so the harness
// (which reports the `linux` platform) turns touchpad resampling on.
const WEBKITGTK_USER_AGENT =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15";
/** The stall measured in the field (`max frame gap 536 ms`). */
const LONG_FRAME_MS = 536;

test.use({
  userAgent: async ({ browserName }, use) => {
    await use(browserName === "webkit" ? WEBKITGTK_USER_AGENT : undefined);
  }
});

function makeEventItem(index: number) {
  return {
    id: { Event: { event_id: `$swipe-item-${String(index).padStart(4, "0")}:example.invalid` } },
    sender: HARNESS_ACCOUNT_KEY,
    sender_label: "Harness User",
    body: `swipe message ${index}\nThis row has stable text height.`,
    timestamp_ms: 1_800_000_000_000 + index * 1000,
    in_reply_to_event_id: null,
    thread_root: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [],
    send_state: null,
    thread_summary: null
  };
}

async function openSeededTimeline(page: Page, itemCount: number): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.locator(`[data-event-id="${HARNESS_SEED_EVENT_ID}"]`)).toBeVisible();
  const items = Array.from({ length: itemCount }, (_, index) => makeEventItem(index));
  await expect
    .poll(
      () =>
        page.evaluate(
          async ({ key, nextItems }) => {
            await window.__harness.pushCoreEvent({
              kind: "Timeline",
              event: {
                InitialItems: {
                  request_id: null,
                  key,
                  generation: 2,
                  // eslint-disable-next-line @typescript-eslint/no-explicit-any
                  items: nextItems as any[]
                }
              }
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any);
            await new Promise((resolve) => setTimeout(resolve, 25));
            return Boolean(document.querySelector('[data-item-id^="$swipe-item-"]'));
          },
          { key: TIMELINE_KEY, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
  // Leave the live edge for the middle of history and let rows measure, as a
  // reader who has already scrolled up would.
  await page.evaluate(async () => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    for (let step = 0; step < 12; step += 1) {
      container.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -400 }));
      container.scrollTop -= 400;
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  });
}

type SwipeResult = {
  ownedEvents: number;
  clientHeight: number;
  /** Frames that ended with a spacer on screen. */
  spacerFrames: number;
  /** Largest visible spacer height in one frame, px. */
  maxSpacerPx: number;
  /** Largest scrollTop change between two painted frames, px. */
  maxFrameJumpPx: number;
  /** Total distance the swipe moved the timeline, px. */
  travelledPx: number;
  stallMs: number;
};

/**
 * A steady upward touchpad swipe: a fractional delta every 8 ms for about
 * 840 ms. With `stallMs`, the renderer main thread is busy for that long in
 * the middle of the swipe. The finger keeps moving meanwhile: each delta the
 * touchpad produces during the stall is created at its own time (so it carries
 * that `timeStamp`, as WebKitGTK's compositor timestamps do) and delivered in
 * one burst when the main thread frees up, as queued input is. Every painted
 * frame is sampled for visible spacer and for its scroll jump.
 */
function swipeUp(page: Page, stallMs: number): Promise<SwipeResult> {
  return page.evaluate(async (stall) => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const state = {
      spacerFrames: 0,
      maxSpacerPx: 0,
      maxFrameJumpPx: 0,
      lastScrollTop: container.scrollTop,
      running: true,
      pending: false
    };
    const startScrollTop = container.scrollTop;
    // Read each frame as painted: the rAF is requested from the container's
    // own scroll listener, which runs after the app's, so it runs after the
    // app's window commit for that scroll (see the round-trip spacer spec).
    const readFrame = () => {
      state.pending = false;
      const box = container.getBoundingClientRect();
      let sliver = 0;
      for (const spacer of Array.from(container.querySelectorAll<HTMLElement>(".timeline-virtual-spacer"))) {
        const rect = spacer.getBoundingClientRect();
        if (rect.height <= 0) continue;
        const visible = Math.min(rect.bottom, box.bottom) - Math.max(rect.top, box.top);
        if (visible > 0) sliver = Math.max(sliver, visible);
      }
      if (sliver > 0) {
        state.spacerFrames += 1;
        state.maxSpacerPx = Math.max(state.maxSpacerPx, sliver);
      }
      state.maxFrameJumpPx = Math.max(state.maxFrameJumpPx, Math.abs(container.scrollTop - state.lastScrollTop));
      state.lastScrollTop = container.scrollTop;
    };
    const scheduleRead = () => {
      if (!state.running || state.pending) return;
      state.pending = true;
      requestAnimationFrame(readFrame);
    };
    container.addEventListener("scroll", scheduleRead, { passive: true });

    let ownedEvents = 0;
    let index = 0;
    const makeDelta = () =>
      new WheelEvent("wheel", {
        bubbles: true,
        cancelable: true,
        deltaY: -(5.3 + 0.6 * Math.sin((index += 1) * 1.7)),
        deltaMode: 0
      });
    const deliver = (event: WheelEvent) => {
      container.dispatchEvent(event);
      if (event.defaultPrevented) ownedEvents += 1;
    };
    // 300 ms of swipe before the stall.
    for (let step = 0; step < 38; step += 1) {
      deliver(makeDelta());
      await sleep(8);
    }
    if (stall > 0) {
      const queued: WheelEvent[] = [];
      const started = performance.now();
      let nextAt = started;
      while (performance.now() - started < stall) {
        // A long task on the renderer main thread; the touchpad keeps sending.
        if (performance.now() >= nextAt) {
          queued.push(makeDelta());
          nextAt += 8;
        }
      }
      for (const event of queued) deliver(event);
    } else {
      // The same finger movement, delivered on time.
      for (let step = 0; step < Math.round(536 / 8); step += 1) {
        deliver(makeDelta());
        await sleep(8);
      }
    }
    // The rest of the swipe, then lift and settle.
    for (let step = 0; step < 38; step += 1) {
      deliver(makeDelta());
      await sleep(8);
    }
    await sleep(1200);
    state.running = false;
    container.removeEventListener("scroll", scheduleRead);
    return {
      ownedEvents,
      clientHeight: container.clientHeight,
      spacerFrames: state.spacerFrames,
      maxSpacerPx: Math.round(state.maxSpacerPx),
      maxFrameJumpPx: Math.round(state.maxFrameJumpPx),
      travelledPx: Math.round(startScrollTop - container.scrollTop),
      stallMs: stall
    };
  }, stallMs);
}

test.describe("touchpad swipe through history (Linux WebKitGTK)", () => {
  test.skip(({ browserName }) => browserName !== "webkit", "touchpad resampling is WebKitGTK-only");

  test("a swipe without a long frame never shows unmounted space", async ({ page }) => {
    await openSeededTimeline(page, 400);
    const result = await swipeUp(page, 0);
    test.info().annotations.push({ type: "swipe", description: JSON.stringify(result) });
    // The timeline owned the gesture and the swipe moved more than a screen.
    expect(result.ownedEvents).toBeGreaterThanOrEqual(100);
    expect(result.travelledPx).toBeGreaterThan(result.clientHeight);
    expect(result.spacerFrames).toBe(0);
  });

  test("a long frame during a swipe never shows unmounted space", async ({ page }) => {
    await openSeededTimeline(page, 400);
    const result = await swipeUp(page, LONG_FRAME_MS);
    test.info().annotations.push({ type: "swipe", description: JSON.stringify(result) });
    expect(result.ownedEvents).toBeGreaterThanOrEqual(100);
    expect(result.travelledPx).toBeGreaterThan(result.clientHeight);
    expect(
      result.spacerFrames,
      `spacer visible in ${result.spacerFrames} frames (max ${result.maxSpacerPx} px); ` +
        `largest frame jump ${result.maxFrameJumpPx} px on a ${result.clientHeight} px viewport`
    ).toBe(0);
  });
});
