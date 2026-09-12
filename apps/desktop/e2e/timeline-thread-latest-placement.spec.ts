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
    can_edit: false,
    display_metadata: {
      row_id: `$window-normal-${String(index).padStart(3, "0")}:example.invalid`,
      kind: { kind: "event" },
      content_event_id: `$window-normal-${String(index).padStart(3, "0")}:example.invalid`,
      activity_event_id: `$window-normal-${String(index).padStart(3, "0")}:example.invalid`,
      display_timestamp_ms: 1_800_200_000_000 + index * 1_000
    }
  };
}

function rootItem(
  activityEventId = ROOT_EVENT_ID,
  displayTimestampMs = 1_800_199_000_000
): TimelineItem {
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
    can_edit: true,
    display_metadata: {
      row_id: `thread-root:${ROOT_EVENT_ID}`,
      kind: { kind: "threadRoot" },
      content_event_id: ROOT_EVENT_ID,
      activity_event_id: activityEventId,
      display_timestamp_ms: displayTimestampMs
    }
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

async function pushRootDisplayReset(
  page: Page,
  generation: number,
  items: readonly TimelineItem[]
): Promise<void> {
  await page.evaluate(
    async ({ key, nextGeneration, nextItems }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: nextGeneration,
            batch_id: 1,
            diffs: [{ Reset: { items: nextItems } }]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: ROOM_KEY, nextGeneration: generation, nextItems: items }
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

test("Rust display updates keep an old root whole without room backfill", async ({ page }) => {
  await gotoReadyApp(page);

  await pushInitialItems(page, 101, [
    rootItem(),
    ...Array.from({ length: 3 }, (_, index) => ordinaryItem(index))
  ]);
  const root = page.locator(`[data-row-id="thread-root:${ROOT_EVENT_ID}"]`);
  await expect(root).toHaveCount(1);
  await expect(root).toHaveAttribute("data-content-event-id", ROOT_EVENT_ID);
  await expect(root).toHaveAttribute("data-activity-event-id", ROOT_EVENT_ID);
  await expect(root).toContainText(ROOT_BODY);

  const freshWindow = Array.from(
    { length: INITIAL_WINDOW_NORMAL_MESSAGE_COUNT },
    (_, index) => ordinaryItem(index + 10)
  );
  await pushInitialItems(page, 102, [rootItem(), ...freshWindow]);
  await expect(root).toHaveCount(1);
  await expect(page.locator(`[data-content-event-id="${LATEST_REPLY_EVENT_ID}"]`)).toHaveCount(0);

  await clearInvocations(page);
  await page.getByRole("button", { name: t("workspace.userSettings") }).click();
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
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

  await pushRootDisplayReset(page, 102, [
    ...freshWindow,
    rootItem(LATEST_REPLY_EVENT_ID, 1_800_200_100_000)
  ]);
  await expect(root).toHaveCount(1);
  await expect(root).toHaveAttribute("data-content-event-id", ROOT_EVENT_ID);
  await expect(root).toHaveAttribute("data-activity-event-id", LATEST_REPLY_EVENT_ID);
  await expect(root).toContainText(ROOT_BODY);
  await expect(root.getByRole("button", { name: /^Open thread,/ })).toHaveCount(1);
  expect((await displayRowIds(page)).filter((id) => id === `thread-root:${ROOT_EVENT_ID}`)).toHaveLength(1);
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);

  await page.keyboard.press("Escape");
  await root.getByRole("button", { name: /^Open thread,/ }).click();
  await expect.poll(() => latestInvocationArgs(page, "open_thread")).toEqual({
    roomId: ROOM_ID,
    rootEventId: ROOT_EVENT_ID,
    intent: "existingThread"
  });
});

test("room replay retains the Rust-provided dormant root activity", async ({ page }) => {
  await gotoReadyApp(page);
  const projectedRoot = rootItem(LATEST_REPLY_EVENT_ID, 1_800_200_100_000);
  await pushInitialItems(page, 201, [
    ...Array.from({ length: INITIAL_WINDOW_NORMAL_MESSAGE_COUNT }, (_, index) =>
      ordinaryItem(index + 100)
    ),
    projectedRoot
  ]);

  const root = page.locator(`[data-row-id="thread-root:${ROOT_EVENT_ID}"]`);
  await expect(root).toHaveCount(1);
  await expect(root).toHaveAttribute("data-content-event-id", ROOT_EVENT_ID);
  await expect(root).toHaveAttribute("data-activity-event-id", LATEST_REPLY_EVENT_ID);
  await expect(root).toContainText(ROOT_BODY);
  await waitForTimelineLayout(page);
  await clearInvocations(page);

  await pushInitialItems(page, 202, [
    ...Array.from({ length: INITIAL_WINDOW_NORMAL_MESSAGE_COUNT }, (_, index) =>
      ordinaryItem(index + 200)
    ),
    projectedRoot
  ]);
  await expect(root).toHaveCount(1);
  expect((await displayRowIds(page)).filter((id) => id === `thread-root:${ROOT_EVENT_ID}`)).toHaveLength(1);
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);

  await root.getByRole("button", { name: /^Open thread,/ }).click({ force: true });
  await expect.poll(() => latestInvocationArgs(page, "open_thread")).toEqual({
    roomId: ROOM_ID,
    rootEventId: ROOT_EVENT_ID,
    intent: "existingThread"
  });
});
