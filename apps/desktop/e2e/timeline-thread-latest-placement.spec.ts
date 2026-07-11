/**
 * Headless DOM regression for optional latest-reply thread placement.
 *
 * This uses the full App over the same recording Tauri transport as the
 * other Playwright specs.  The controlled CoreEvent stream models an old root
 * falling outside a fresh Room timeline window; it does not read the
 * TimelineStore or any component-private state.  Assertions use the public
 * settings control, IPC commands, and stable display-row attributes.
 */

import { expect, test, type Page } from "@playwright/test";

import { roomTimelineKey, type TimelineItem } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";

const ACCOUNT_KEY = "@harness-user:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const ROOM_KEY = roomTimelineKey(ACCOUNT_KEY, ROOM_ID);
const ROOT_EVENT_ID = "$old-thread-root:example.invalid";
const LATEST_REPLY_EVENT_ID = "$latest-thread-reply:example.invalid";
const ROOT_BODY = "Old root stays root-owned";
const LATEST_REPLY_BODY = "Latest threaded activity";
const INITIAL_WINDOW_NORMAL_MESSAGE_COUNT = 72;

function ordinaryItem(index: number): TimelineItem {
  return {
    id: { Event: { event_id: `$window-normal-${String(index).padStart(3, "0")}:example.invalid` } },
    sender: "@sender:example.invalid",
    sender_label: "Timeline Sender",
    body: `normal window message ${index}`,
    timestamp_ms: 1_800_200_000_000 + index * 1_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}

function rootItem(): TimelineItem {
  return {
    id: { Event: { event_id: ROOT_EVENT_ID } },
    sender: "@root-sender:example.invalid",
    sender_label: "Root Sender",
    body: ROOT_BODY,
    timestamp_ms: 1_800_199_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: {
      reply_count: 1,
      latest_event_id: LATEST_REPLY_EVENT_ID,
      latest_sender: "@reply-sender:example.invalid",
      latest_sender_label: "Reply Sender",
      latest_body_preview: LATEST_REPLY_BODY,
      latest_timestamp_ms: 1_800_200_100_000
    },
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: true,
    is_edited: false,
    can_edit: true
  };
}

function latestReplyItem(): TimelineItem {
  return {
    id: { Event: { event_id: LATEST_REPLY_EVENT_ID } },
    sender: "@reply-sender:example.invalid",
    sender_label: "Reply Sender",
    body: LATEST_REPLY_BODY,
    timestamp_ms: 1_800_200_100_000,
    in_reply_to_event_id: ROOT_EVENT_ID,
    thread_root: ROOT_EVENT_ID,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}

async function gotoReadyApp(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

async function pushInitialItems(
  page: Page,
  generation: number,
  items: readonly TimelineItem[]
): Promise<void> {
  await page.evaluate(
    async ({ key, nextGeneration, nextItems }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key,
            generation: nextGeneration,
            items: nextItems
          }
        }
        // The test fixture delivers the public CoreEvent wire payload.
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: ROOM_KEY, nextGeneration: generation, nextItems: items }
  );
}

async function pushLatestReplyAndHydratedRoot(page: Page, generation: number): Promise<void> {
  await page.evaluate(
    async ({ key, nextGeneration, rootEventId, latestReplyEventId, root, reply }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: nextGeneration,
            batch_id: 1,
            diffs: [{ PushBack: { item: reply } }]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ThreadRootProjection: {
            key,
            projection: {
              root_event_id: rootEventId,
              activity_event_id: latestReplyEventId,
              activity_timestamp_ms: reply.timestamp_ms,
              state: { kind: "ready", item: root }
            }
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: ROOM_KEY,
      nextGeneration: generation,
      rootEventId: ROOT_EVENT_ID,
      latestReplyEventId: LATEST_REPLY_EVENT_ID,
      root: rootItem(),
      reply: latestReplyItem()
    }
  );
}

/**
 * The app harness has no Rust actor, so this supplies the exact CoreEvent
 * contract that `handle_replay_initial_items` emits after a bounded replay.
 * The focused Rust actor test proves the event is derived from an already-known
 * root without fetch, pagination, or anchor materialization.
 */
async function pushReplayKnownRoot(page: Page): Promise<void> {
  await page.evaluate(
    async ({ key, rootEventId, latestReplyEventId, root }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ThreadRootProjection: {
            key,
            projection: {
              root_event_id: rootEventId,
              activity_event_id: latestReplyEventId,
              activity_timestamp_ms: 1_800_200_100_000,
              retain_without_reply: true,
              source: { kind: "replayKnown", epoch: 1 },
              state: { kind: "ready", item: root }
            }
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: ROOM_KEY,
      rootEventId: ROOT_EVENT_ID,
      latestReplyEventId: LATEST_REPLY_EVENT_ID,
      root: rootItem()
    }
  );
}

async function clearInvocations(page: Page): Promise<void> {
  await page.evaluate(() => window.__harness.clearInvocations());
}

async function invocationCount(page: Page, command: string): Promise<number> {
  return page.evaluate((commandName) => window.__harness.invocationsOf(commandName).length, command);
}

async function latestInvocationArgs(page: Page, command: string): Promise<unknown> {
  return page.evaluate(
    (commandName) => window.__harness.invocationsOf(commandName).at(-1)?.args,
    command
  );
}

async function displayRowIds(page: Page): Promise<string[]> {
  return page.locator("[data-row-id]").evaluateAll((rows) =>
    rows.flatMap((row) => {
      const id = row.getAttribute("data-row-id");
      return id === null ? [] : [id];
    })
  );
}

async function waitForTimelineLayout(page: Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      })
  );
}

test("latest-reply placement keeps an old root whole without room backfill", async ({ page }) => {
  await gotoReadyApp(page);

  // The default remains the root's canonical time.  This gives the regression
  // a direct origin assertion before the fresh live window intentionally
  // excludes the old root below.
  await pushInitialItems(page, 101, [rootItem(), ...Array.from({ length: 3 }, (_, i) => ordinaryItem(i))]);
  const rootAtOrigin = page.locator(`[data-row-id="thread-root:${ROOT_EVENT_ID}"]`);
  await expect(rootAtOrigin).toHaveCount(1);
  await expect(rootAtOrigin).toHaveAttribute("data-content-event-id", ROOT_EVENT_ID);
  await expect(rootAtOrigin).toHaveAttribute("data-activity-event-id", ROOT_EVENT_ID);
  await expect(rootAtOrigin).toContainText(ROOT_BODY);
  const defaultRowIds = await displayRowIds(page);
  expect(defaultRowIds.indexOf(`thread-root:${ROOT_EVENT_ID}`)).toBeLessThan(
    defaultRowIds.indexOf("$window-normal-000:example.invalid")
  );

  // Model the next Room initial window after an old root has fallen out of
  // live coverage.  It contains enough ordinary events to make the absent
  // root meaningful, then receives the latest threaded reply and the bounded
  // root hydration event.  No test reads canonical-store internals.
  const freshWindow = Array.from(
    { length: INITIAL_WINDOW_NORMAL_MESSAGE_COUNT },
    (_, index) => ordinaryItem(index + 10)
  );
  await pushInitialItems(page, 102, freshWindow);
  await expect(rootAtOrigin).toHaveCount(0);
  await clearInvocations(page);
  await pushLatestReplyAndHydratedRoot(page, 102);

  const standaloneReply = page.locator(
    `[data-content-event-id="${LATEST_REPLY_EVENT_ID}"]`
  );
  // The Core stream retains this reply as the canonical projection anchor,
  // but Room presentation suppresses standalone reply rows both before and
  // after opting in. The default root placement therefore remains unchanged.
  await expect(standaloneReply).toHaveCount(0);
  await expect(rootAtOrigin).toHaveCount(0);

  // Enable through the real settings control.  The harness applies the
  // Rust-shaped update_settings response, just like the desktop boundary.
  await page.getByRole("button", { name: t("workspace.userSettings") }).click();
  const placementToggle = page.getByRole("switch", {
    name: t("settings.threadRootLatestReply")
  });
  await expect(placementToggle).toHaveAttribute("aria-checked", "false");
  await placementToggle.click();
  await expect.poll(() => invocationCount(page, "update_settings")).toBe(1);
  await expect.poll(() => latestInvocationArgs(page, "update_settings")).toEqual({
    patch: {
      timeline: {
        auto_load_older_messages: true,
        thread_root_order: { kind: "latestReply" }
      }
    }
  });
  await expect(placementToggle).toHaveAttribute("aria-checked", "true");

  // A single root/summary block takes the exact latest-reply activity slot.
  const movedRoot = page.locator(`[data-row-id="thread-root:${ROOT_EVENT_ID}"]`);
  await expect(movedRoot).toHaveCount(1);
  await expect(movedRoot).toHaveAttribute("data-content-event-id", ROOT_EVENT_ID);
  await expect(movedRoot).toHaveAttribute("data-activity-event-id", LATEST_REPLY_EVENT_ID);
  await expect(movedRoot).toContainText(ROOT_BODY);
  await expect(movedRoot.getByRole("button", { name: /^Open thread,/ })).toHaveCount(1);
  await expect(standaloneReply).toHaveCount(0);
  expect((await displayRowIds(page)).filter((id) => id === `thread-root:${ROOT_EVENT_ID}`)).toHaveLength(1);

  // The projection hydration and setting toggle must not turn into a viewport
  // driven Room backfill.  Opening the summary still carries the content/root
  // identity to the Rust command boundary.
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);
  await movedRoot.getByRole("button", { name: /^Open thread,/ }).click({ force: true });
  await expect.poll(() => invocationCount(page, "open_thread")).toBe(1);
  await expect.poll(() => latestInvocationArgs(page, "open_thread")).toEqual({
    roomId: ROOM_ID,
    rootEventId: ROOT_EVENT_ID
  });
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);
});

test("a replay-known root uses summary activity when the Room stream has no reply row", async ({
  page
}) => {
  await gotoReadyApp(page);
  await pushInitialItems(
    page,
    201,
    Array.from({ length: INITIAL_WINDOW_NORMAL_MESSAGE_COUNT }, (_, index) => ordinaryItem(index + 100))
  );
  await expect(page.locator('[data-content-event-id="$window-normal-100:example.invalid"]')).toHaveCount(1);
  // The harness viewport intentionally starts underfilled, so allow its
  // ordinary initial-window backfill request to settle before measuring the
  // delta caused by replay projection or the setting switch.
  await waitForTimelineLayout(page);
  await clearInvocations(page);
  await pushReplayKnownRoot(page);

  const knownRoot = page.locator(`[data-row-id="thread-root:${ROOT_EVENT_ID}"]`);
  await expect(knownRoot).toHaveCount(0);
  await expect(
    page.locator(`[data-content-event-id="${LATEST_REPLY_EVENT_ID}"]`)
  ).toHaveCount(0);
  await waitForTimelineLayout(page);
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);

  await page.getByRole("button", { name: t("workspace.userSettings") }).click();
  const placementToggle = page.getByRole("switch", {
    name: t("settings.threadRootLatestReply")
  });
  await expect(placementToggle).toHaveAttribute("aria-checked", "false");
  await placementToggle.click();
  await expect.poll(() => invocationCount(page, "update_settings")).toBe(1);

  await expect(knownRoot).toHaveCount(1);
  await expect(knownRoot).toHaveAttribute("data-content-event-id", ROOT_EVENT_ID);
  await expect(knownRoot).toHaveAttribute("data-activity-event-id", LATEST_REPLY_EVENT_ID);
  await expect(knownRoot).toContainText(ROOT_BODY);
  await expect(knownRoot.getByRole("button", { name: /^Open thread,/ })).toHaveCount(1);
  await waitForTimelineLayout(page);
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);

  await knownRoot.getByRole("button", { name: /^Open thread,/ }).click({ force: true });
  await expect.poll(() => latestInvocationArgs(page, "open_thread")).toEqual({
    roomId: ROOM_ID,
    rootEventId: ROOT_EVENT_ID
  });
});
