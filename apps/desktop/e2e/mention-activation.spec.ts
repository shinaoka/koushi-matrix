/**
 * Headless spec: activating a user mention opens the account in place (#874).
 *
 * A `matrix.to` user permalink is an entity this client owns, so activating one
 * must show the profile panel — the same in-app result as clicking a sender
 * name. Before, the main timeline dropped the click entirely and the thread and
 * search panes handed the URL to the browser.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { roomTimelineKey, threadTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);
const TARGET_USER_ID = "@harness-ada:example.invalid";
const TARGET_LABEL = "Harness Ada";
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

async function seedTimelineItems(page: Page, items: unknown[], key = HARNESS_ROOM_KEY): Promise<void> {
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
          { key, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

/** A message whose formatted body carries a real user-mention anchor. */
function mentionItem(eventId = "$mention-target:example.invalid") {
  const mentionUrl = `https://matrix.to/#/${encodeURIComponent(TARGET_USER_ID)}`;
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: `@${TARGET_LABEL} please take a look`,
    formatted: {
      html: `<a href="${mentionUrl}">@${TARGET_LABEL}</a> please take a look`,
      plain_text: `@${TARGET_LABEL} please take a look`,
      code_blocks: []
    },
    timestamp_ms: 1_800_000_100_000,
    in_reply_to_event_id: null,
    thread_root: null,
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

test("activating a user mention opens the profile in the main timeline", async ({ page }) => {
  const popups: string[] = [];
  page.on("popup", (popup) => popups.push(popup.url()));

  await gotoReadyShell(page);
  await seedTimelineItems(page, [mentionItem()]);

  const mention = page.getByRole("link", { name: `@${TARGET_LABEL}`, exact: true });
  await expect(mention).toBeVisible();
  await mention.click();

  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel).toContainText(TARGET_USER_ID);
  // The mention is in-app navigation, so no browser surface may open.
  expect(popups).toEqual([]);
});

test("activating a user mention opens the profile inside the thread pane", async ({ page }) => {
  const popups: string[] = [];
  page.on("popup", (popup) => popups.push(popup.url()));

  await gotoReadyShell(page);

  // One pill click opens the thread pane; the harness answers `open_thread`
  // with an open thread for the seeded root.
  await page.getByRole("button", { name: /2 replies/ }).click();
  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(panel.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  await seedTimelineItems(page, [mentionItem("$thread-mention:example.invalid")], HARNESS_THREAD_KEY);

  const mention = panel.getByRole("link", { name: `@${TARGET_LABEL}`, exact: true });
  await expect(mention).toBeVisible();
  await mention.click();

  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel).toContainText(TARGET_USER_ID);
  expect(popups).toEqual([]);
});
