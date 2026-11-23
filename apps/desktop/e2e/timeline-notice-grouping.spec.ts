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
import { pushTimelineDiffs } from "./support/basicOperations";

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

/**
 * Rows the switch never hides: room state and moderation. Core gives them no
 * notice category (`membership_notice_category`), so `is_hidden` stays false
 * whatever the switch says; the classifier itself is pinned in
 * `crates/koushi-core/src/timeline/item_projection/notice_category_tests.rs`.
 */
const PROTECTED = [
  { eventId: "$protected-encryption:example.invalid", body: "enabled encryption" },
  { eventId: "$protected-upgrade:example.invalid", body: "upgraded the room" },
  { eventId: "$protected-ban:example.invalid", body: "was banned" },
  { eventId: "$protected-kick:example.invalid", body: "was kicked" }
];

/** The protected notices, laid out after the profile burst. */
function protectedNotices(): Record<string, unknown>[] {
  return PROTECTED.map(({ eventId, body }, position) => ({
    id: { Event: { event_id: eventId } },
    sender: SENDER,
    sender_label: "Harness User",
    // A harness-only string: the real DTO carries a `notice_i18n` key here. The
    // protected rows are identified by their event id, so the text only has to be
    // unique and stable enough to read in a failure report.
    body,
    message_kind: "notice",
    timestamp_ms: BASE_MS + (GROUP.length + position) * 1_000,
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
      display_timestamp_ms: BASE_MS + (GROUP.length + position) * 1_000
    }
  }));
}

/**
 * Core's answer to the switch: the profile rows it classifies as profile notices
 * go `is_hidden`, the protected rows keep their state, and the acknowledgement
 * travels beside the corrected rows (#2041, #1110).
 */
async function pushPreference(
  page: Page,
  showRoomAndMemberChanges: boolean,
  batchId: number
): Promise<void> {
  const profiles = groupedNotices();
  const protectedRows = protectedNotices();
  const hidden = !showRoomAndMemberChanges;
  await pushTimelineDiffs(
    page,
    [
      ...profiles.map((item, index) => ({ Set: { index, item: { ...item, is_hidden: hidden } } })),
      ...protectedRows.map((item, position) => ({
        Set: { index: profiles.length + position, item: { ...item, is_hidden: false } }
      }))
    ],
    2,
    batchId
  );
  // The store deliberately drops this event (#1110: the renderer acknowledges the
  // policy, it never derives visibility from it). Pushed anyway so the spec keeps
  // the same wire shape as Core, exactly as the "hide deleted messages" spec does.
  await page.evaluate(async (nextPolicy) => {
    await window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        DisplayPolicyUpdated: { hide_redacted: true, show_room_and_member_changes: nextPolicy }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, showRoomAndMemberChanges);
}

/**
 * Flips the switch through the real settings panel and checks the typed command
 * and its settlement. The harness owns `update_settings` (`applySettingsPatch`),
 * so the switch flips on its own; the timeline consequence still arrives only as
 * Core's rows, which is what this spec is about.
 */
async function setRoomAndMemberChanges(page: Page, on: boolean): Promise<void> {
  const toggle = page.getByRole("switch", { name: t("settings.showRoomAndMemberChanges") });
  await expect(toggle).toHaveAttribute("aria-checked", on ? "false" : "true");
  await toggle.click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window.__harness.invocationsOf("update_settings").at(-1)?.args as {
              patch: { display: { show_room_and_member_changes: boolean } };
            }
          ).patch.display.show_room_and_member_changes
      )
    )
    .toBe(on);
  await expect(toggle).toHaveAttribute("aria-checked", on ? "true" : "false");
}

test("the room-changes switch hides the notices it owns and keeps the protected rows", async ({
  page
}) => {
  await gotoReadyShell(page);
  // Eight short rows fit the first mount window; this seed does not wait for the
  // window to settle, so a much larger fixture would need the shared helper.
  await seedTimelineItems(page, [...groupedNotices(), ...protectedNotices()]);

  const head = page.locator(`[data-event-id="${GROUP[0]}"]`);
  await expect(head).toBeVisible();
  expect((await head.textContent())?.replace(/\s+/g, " ").trim()).toBe(
    "Harness User changed their profile picture · 4"
  );
  for (const { eventId } of PROTECTED) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toBeVisible();
  }

  // Off. The renderer only records the choice: the rows stay until Core sends
  // the corrected ones, never a local recomputation (#1110).
  await page.getByRole("button", { name: "App Settings", exact: true }).click();
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
  await setRoomAndMemberChanges(page, false);
  await expect(head).toBeVisible();
  for (const { eventId } of PROTECTED) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toBeVisible();
  }

  await pushPreference(page, false, 3);
  for (const eventId of GROUP) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toHaveCount(0);
  }
  for (const { eventId } of PROTECTED) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toBeVisible();
  }

  // On. The burst comes back as one line with its count, and the protected rows
  // never moved.
  await setRoomAndMemberChanges(page, true);
  await pushPreference(page, true, 4);
  await expect(head).toBeVisible();
  expect((await head.textContent())?.replace(/\s+/g, " ").trim()).toBe(
    "Harness User changed their profile picture · 4"
  );
  // Back as the reader left it: one line, not reopened by the hide/show round trip.
  await expect(
    head.getByRole("button", { name: t("timeline.noticeGroupToggle", { count: 4 }) })
  ).toHaveAttribute("aria-expanded", "false");
  for (const eventId of GROUP.slice(1)) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toHaveCount(0);
  }
  for (const { eventId } of PROTECTED) {
    await expect(page.locator(`[data-event-id="${eventId}"]`)).toBeVisible();
  }
});
