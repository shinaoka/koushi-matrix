/**
 * Headless spec: the mention pill is a property of the message (#874).
 *
 * A message whose `m.mentions` names a user renders that mention as the
 * composer's pill, taken from the event's own formatted anchor. Text that only
 * looks like a mention — no `m.mentions` — stays plain text for every viewer,
 * whatever profiles the client happens to have loaded.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);
const MENTIONED_USER_ID = "@harness-ada:example.invalid";
const MENTION_LABEL = "Harness Ada";
const MENTION_URL = `https://matrix.to/#/${encodeURIComponent(MENTIONED_USER_ID)}`;

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

async function seedTimelineItems(page: Page, items: unknown[]): Promise<void> {
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
          { key: HARNESS_ROOM_KEY, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

function itemWithBody(
  eventId: string,
  body: string,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body,
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
    can_edit: true,
    ...overrides
  };
}

/** A real mention: `m.mentions` plus the formatted anchor it produced. */
function realMentionItem(eventId: string): Record<string, unknown> {
  const body = `@${MENTION_LABEL} please take a look`;
  return itemWithBody(eventId, body, {
    formatted: {
      html: `<a href="${MENTION_URL}">@${MENTION_LABEL}</a> please take a look`,
      plain_text: body,
      code_blocks: []
    },
    mentioned_user_ids: [MENTIONED_USER_ID]
  });
}

test("a message that mentions a user renders that mention as a pill", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    realMentionItem("$mentioned:example.invalid"),
    // Same visible text, no `m.mentions`: it notifies nobody and is not a pill.
    itemWithBody("$unmentioned:example.invalid", `@${MENTION_LABEL} please take a look`)
  ]);

  const mentionedRow = page.locator('[data-event-id="$mentioned:example.invalid"]');
  const pill = mentionedRow.locator("a.message-mention-pill");
  await expect(pill).toHaveCount(1);
  await expect(pill).toHaveAttribute("data-mention-user-id", MENTIONED_USER_ID);

  const unmentionedRow = page.locator('[data-event-id="$unmentioned:example.invalid"]');
  await expect(unmentionedRow).toBeVisible();
  await expect(unmentionedRow.locator(".message-mention-pill")).toHaveCount(0);
  await expect(unmentionedRow).toContainText(`@${MENTION_LABEL} please take a look`);

  // The pill keeps the in-app activation of a mention.
  await pill.click();
  const panel = page.getByRole("complementary", { name: t("panel.context") });
  await expect(panel.getByRole("heading", { name: t("panel.profile") })).toBeVisible();
  await expect(panel).toContainText(MENTIONED_USER_ID);
});
