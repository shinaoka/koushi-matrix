/**
 * Linux WebKitGTK touchpad resampling (see
 * `src/components/timeline/touchpadScrollResampler.ts`, WebKit bug 332612).
 *
 * The jsdom component test fakes the scroll container. Here a real engine
 * rounds scrollTop to whole pixels, fires real scroll events and runs real
 * layout, anchoring and virtualization, so this pins what jsdom cannot: the
 * resampled movement lands at Chromium's distance in whole pixels, and nothing
 * in the timeline moves the content afterwards. The harness reports the
 * `linux` platform, so WebKit resamples and Chromium stays native.
 *
 * A real touchpad cannot be driven headlessly; the swipe is a stream of
 * synthetic fractional wheel deltas. WebKit clamps nested timers, so the
 * stream runs at a uniform 8 ms (a real touchpad sends every ~4 ms).
 */
import { expect, test, type Page } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_SEED_EVENT_ID = "$seed-event:example.invalid";
const TIMELINE_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);
const SPEED = 2.4;
// The user agent Koushi's WebKitGTK webview reports on Linux. Playwright's
// WebKit reports a macOS one on every host, so present the Linux one there.
const WEBKITGTK_USER_AGENT =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15";

test.use({
  userAgent: async ({ browserName }, use) => {
    await use(browserName === "webkit" ? WEBKITGTK_USER_AGENT : undefined);
  }
});

function makeEventItem(index: number) {
  return {
    id: { Event: { event_id: `$touchpad-item-${String(index).padStart(4, "0")}:example.invalid` } },
    sender: HARNESS_ACCOUNT_KEY,
    sender_label: "Harness User",
    body: `touchpad message ${index}\nThis row has stable text height.`,
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
            return Boolean(document.querySelector('[data-item-id^="$touchpad-item-"]'));
          },
          { key: TIMELINE_KEY, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
  // Leave the live edge for the middle of history, as a reader would.
  await page.evaluate(async () => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    for (let step = 0; step < 12; step += 1) {
      container.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -400 }));
      container.scrollTop -= 400;
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }
  });
}

type SwipeResult = {
  prevented: boolean[];
  deltas: number[];
  /** On-screen offset of one row from the container top. */
  rowBefore: number;
  rowSettled: number | null;
  rowLater: number | null;
};

/**
 * A slow upward swipe: 45 fractional deltas every 8 ms, then 600 ms to settle.
 * The region it crosses is scrolled through and settled first, so its rows
 * carry measured heights and the on-screen distance is not mixed with
 * estimated-height corrections from rows mounting mid-swipe.
 */
function swipeUp(page: Page): Promise<SwipeResult> {
  return page.evaluate(async () => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const frames = (count: number) =>
      new Promise<void>((resolve) => {
        const step = (left: number) => (left === 0 ? resolve() : requestAnimationFrame(() => step(left - 1)));
        step(count);
      });
    const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    for (const delta of [-350, -350, 350, 350]) {
      container.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: delta }));
      container.scrollTop += delta;
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
      await frames(3);
    }
    await sleep(500);
    await frames(2);
    // What the reader sees: a row just above the viewport, mounted by overscan
    // now and on screen once the swipe moves the content down ~570 px.
    const containerTop = () => container.getBoundingClientRect().top;
    const row = Array.from(container.querySelectorAll<HTMLElement>("[data-event-id]"))
      .filter((candidate) => candidate.getBoundingClientRect().top < containerTop() - 60)
      .at(-1)!;
    const rowId = row.dataset.eventId!;
    const rowOffset = () => {
      const current = container.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(rowId)}"]`);
      return current ? current.getBoundingClientRect().top - containerTop() : null;
    };
    const rowBefore = rowOffset()!;
    const prevented: boolean[] = [];
    const deltas: number[] = [];
    for (let index = 0; index < 45; index += 1) {
      const deltaY = -(5.3 + 0.6 * Math.sin(index * 1.7));
      const event = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY, deltaMode: 0 });
      container.dispatchEvent(event);
      prevented.push(event.defaultPrevented);
      deltas.push(deltaY);
      await sleep(8);
    }
    await sleep(600);
    const rowSettled = rowOffset();
    await sleep(400);
    return { prevented, deltas, rowBefore, rowSettled, rowLater: rowOffset() };
  });
}

test("a WebKitGTK touchpad swipe moves Chromium's distance as the reader's own scroll", async ({
  page,
  browserName
}) => {
  await openSeededTimeline(page, 400);
  const result = await swipeUp(page);

  if (browserName !== "webkit") {
    // Chromium (Electron, WebView2) delivers frame-aligned input itself.
    expect(result.prevented.some(Boolean)).toBe(false);
    return;
  }

  // The timeline owns the gesture once two fractional deltas arrive at
  // touchpad cadence (a slow timer may delay that by an event or two).
  const owned = result.prevented.filter(Boolean).length;
  expect(owned).toBeGreaterThanOrEqual(40);
  const ownedInput = result.deltas.reduce(
    (sum, delta, index) => sum + (result.prevented[index] ? delta : 0),
    0
  );

  // Chromium's distance on screen (scrolling up moves content down). Rows
  // mounting next to the tracked one settle its offset by a few pixels (3.9 px
  // on ~560 px here, as with native scrolling), so allow 1.5%: enough to catch
  // a missing or doubled scale (hundreds of pixels), not to re-measure rounding,
  // which the jsdom component test pins to under a pixel.
  expect(result.rowSettled).not.toBeNull();
  const moved = result.rowSettled! - result.rowBefore;
  const expected = -ownedInput * SPEED;
  expect(Math.abs(moved - expected)).toBeLessThanOrEqual(Math.max(3, expected * 0.015));

  // Per-frame evenness is pinned by the resampler unit test and the jsdom
  // component test, which control input timing. Headless timers here stretch
  // and spike under load (8-36 ms), which a real touchpad's ~4 ms stream does
  // not, so evenness asserted here would measure the harness.

  // The reader's own scroll: no anchoring, restore or height commit moves the
  // content after it settles.
  expect(result.rowLater).not.toBeNull();
  expect(Math.abs(result.rowLater! - result.rowSettled!)).toBeLessThanOrEqual(1);
});
