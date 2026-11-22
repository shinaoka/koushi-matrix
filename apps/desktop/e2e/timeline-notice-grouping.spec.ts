/**
 * Headless spec: consecutive state notices share one line plus a count (#2041 P2).
 *
 * Runs on the full-app harness, so the real stylesheet and the real timeline
 * store are in effect. Rust decides which rows form a group; this spec pins what
 * the reader sees — one information line with `· 4`, opening to the four rows
 * with their own times — and that opening is per view, not persisted.
 */

import { expect, test, type Page } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";

const ACCOUNT_KEY = "@harness-user:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const ROOM_KEY = roomTimelineKey(ACCOUNT_KEY, ROOM_ID);
const SENDER = "@harness-user:example.invalid";
const BASE_MS = 1_800_000_100_000;
const GROUP = ["$notice-0:example.invalid", "$notice-1:example.invalid", "$notice-2:example.invalid", "$notice-3:example.invalid"];

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: t("timeline.replyToMessage") }).first()
  ).toBeVisible();
}

/** Four profile-avatar notices, grouped by Core into one row. */
function groupedNotices(): Record<string, unknown>[] {
  return GROUP.map((eventId, position) => ({
    id: { Event: { event_id: eventId } },
    sender: SENDER,
    sender_label: "Harness User",
    body: "changed their profile picture",
    message_kind: "notice",
    notice_i18n: { key: "timeline.notice.profileAvatarChanged" },
    timestamp_ms: BASE_MS + position * 1_000,
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
    display_metadata: {
      row_id: eventId,
      kind: { kind: "event" },
      content_event_id: eventId,
      activity_event_id: eventId,
      display_timestamp_ms: BASE_MS + position * 1_000,
      group_count: GROUP.length,
      group_position: position,
      group_id: `notice-group:${GROUP[0]}`
    }
  }));
}

async function seedTimelineItems(page: Page, items: unknown[]): Promise<void> {
  await page.evaluate(
    async ({ key, nextItems }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 2, items: nextItems }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: ROOM_KEY, nextItems: items }
  );
}

test("a burst of notices is one line that opens to the individual events", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, groupedNotices());

  const head = page.locator(`[data-event-id="${GROUP[0]}"]`);
  await expect(head).toBeVisible();
  // Collapsed: one information line, the count, and no member rows.
  for (const eventId of GROUP.slice(1)) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toHaveCount(0);
  }
  const toggle = head.getByRole("button", { name: t("timeline.noticeGroupToggle", { count: 4 }) });
  await expect(toggle).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect((await head.textContent())?.replace(/\s+/g, " ").trim()).toBe(
    "Harness User changed their profile picture · 4"
  );

  await toggle.click();

  // Expanded: every event, each with its own time, in order.
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  for (const [position, eventId] of GROUP.entries()) {
    const row = page.locator(`[data-event-id="${eventId}"]`);
    await expect(row).toBeVisible();
    await expect(row.locator("time")).toHaveAttribute(
      "datetime",
      new Date(BASE_MS + position * 1_000).toISOString()
    );
  }

  await toggle.click();
  for (const eventId of GROUP.slice(1)) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toHaveCount(0);
  }
});
