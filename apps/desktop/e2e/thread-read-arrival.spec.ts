/**
 * Headless spec: a thread's live edge the client scrolled to is not the reader
 * reading (#872).
 *
 * Opening the thread pane snaps it to the newest reply and then snaps again once
 * variable-height rows are measured. Those are client placements: the renderer
 * must report `bottomArrival: "programmatic"` for them, so the read state can
 * refuse to acknowledge the thread's attention count on the strength of the
 * snap alone. A bottom the reader scrolls to reports "user".
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { threadTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const THREAD_ROOT_EVENT_ID = "$seed-event:example.invalid";
const HARNESS_THREAD_KEY = threadTimelineKey(
  HARNESS_ACCOUNT_KEY,
  HARNESS_ROOM_ID,
  THREAD_ROOT_EVENT_ID
);

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

/** Seed a thread tall enough that reaching the live edge requires scrolling. */
async function seedThreadReplies(page: Page, count: number): Promise<void> {
  const items = Array.from({ length: count }, (_, index) => ({
    id: { Event: { event_id: `$thread-reply-${index}:example.invalid` } },
    sender: "@harness-other:example.invalid",
    sender_label: "Harness Other",
    body: `Reply number ${index} with enough text to occupy a row of its own`,
    timestamp_ms: 1_800_000_100_000 + index,
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
  }));
  await expect
    .poll(
      async () =>
        page.evaluate(
          async ({ key, nextItems }) => {
            await window.__harness.pushCoreEvent({
              kind: "Timeline",
              event: {
                InitialItems: { request_id: null, key, generation: 2, items: nextItems }
              }
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any);
            await new Promise((resolve) => setTimeout(resolve, 25));
            return nextItems.every((item) =>
              document.querySelector(
                `[data-event-id="${CSS.escape(
                  (item as { id: { Event: { event_id: string } } }).id.Event.event_id
                )}"]`
              )
            );
          },
          { key: HARNESS_THREAD_KEY, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

/** The `bottomArrival` of the newest viewport observation for the thread. */
async function threadBottomArrival(page: Page): Promise<string | null> {
  return page.evaluate((threadRootEventId) => {
    const invocations = (
      window as unknown as {
        __harness: {
          invocationsOf(command: string): { args?: Record<string, unknown> }[];
        };
      }
    ).__harness.invocationsOf("observe_timeline_viewport");
    const threadObservations = invocations.filter(
      (invocation) => invocation.args?.threadRootEventId === threadRootEventId
    );
    const last = threadObservations.at(-1);
    return typeof last?.args?.bottomArrival === "string"
      ? (last.args.bottomArrival as string)
      : null;
  }, THREAD_ROOT_EVENT_ID);
}

test("a thread the client snapped to reports a programmatic bottom, a reader scroll reports user", async ({
  page
}) => {
  await gotoReadyShell(page);

  // Open the pane first: the thread timeline only renders once it is mounted.
  await page.getByRole("button", { name: /2 replies/ }).click();
  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(panel.getByText(t("panel.thread"), { exact: true })).toBeVisible();
  await seedThreadReplies(page, 40);
  await expect(
    page.locator('[data-event-id="$thread-reply-39:example.invalid"]')
  ).toBeVisible();

  // The open-time snap is the client's own placement.
  await expect.poll(() => threadBottomArrival(page)).toBe("programmatic");

  // The reader scrolling the pane to the live edge is theirs.
  await page.locator('[data-event-id="$thread-reply-20:example.invalid"]').hover();
  await page.mouse.wheel(0, 4000);
  await expect.poll(() => threadBottomArrival(page)).toBe("user");
});
