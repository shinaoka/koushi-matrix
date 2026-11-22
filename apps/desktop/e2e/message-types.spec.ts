/**
 * Headless-Chromium DOM tier for message type rendering. The Rust core owns
 * msgtype/spoiler projection; this harness injects the resulting DTO and
 * verifies that TimelineView only renders it.
 */

import { expect, test, type Page } from "@playwright/test";

const ROOM_ID = "!harness-room:example.invalid";
const ACCOUNT_KEY = "@harness-user:example.invalid";

function timelineKey() {
  return { account_key: ACCOUNT_KEY, kind: { Room: { room_id: ROOM_ID } } };
}

function makeItem(id: string, body: string, overrides = {}) {
  return {
    id: { Event: { event_id: id } },
    sender: "@alice:example.invalid",
    sender_label: "Alice Alias",
    body,
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [],
    ...overrides
  };
}

async function pushInitialTimelineItems(page: Page) {
  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: [
        makeItem("$emote:example.invalid", "waves", { message_kind: "emote" }),
        makeItem("$notice:example.invalid", "bot notice", {
          message_kind: "notice",
          // #2041: the row's own rule hides Reply on a notice, not the DTO.
          can_react: true,
          actions: {
            can_copy: false,
            can_forward: false,
            can_reply: true,
            can_pin: true,
            can_permalink: false,
            can_view_source: false
          }
        }),
        makeItem("$spoiler:example.invalid", "keep secret hidden", {
          message_kind: "text",
          spoiler_spans: [{ start_utf16: 5, end_utf16: 11 }]
        }),
        makeItem("$formatted-spoiler:example.invalid", "plain fallback", {
          message_kind: "text",
          formatted: {
            html: 'keep <span data-mx-spoiler="reason">secret</span> hidden',
            plain_text: "keep secret hidden",
            code_blocks: []
          },
          spoiler_spans: [{ start_utf16: 5, end_utf16: 11, reason: "reason" }]
        })
      ]
    }
  );
}

test("message kinds and spoilers render from Rust-owned timeline DTOs", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");

  await pushInitialTimelineItems(page);

  const emote = page.locator('[data-message-kind="emote"]');
  await expect(emote.locator(".message-emote-prefix")).toContainText("Alice Alias");
  await expect(emote.locator(".message-body")).toContainText("waves");

  const notice = page.locator('[data-message-kind="notice"]');
  await expect(notice.locator(".message-body")).toHaveClass(/message-notice/);
  await expect(notice).toContainText("bot notice");

  // #2041: a notice is one muted line, not a message row. No avatar slot, no
  // message action bar, and the body shares the sender's line.
  await expect(notice).toHaveClass(/message-notice-row/);
  await expect(notice.locator(".avatar")).toHaveCount(0);
  await expect(notice.getByRole("button", { name: "Reply to message" })).toHaveCount(0);
  await expect(notice.getByRole("button", { name: "Edit message" })).toHaveCount(0);
  // The kept affordances: react and pin.
  await expect(notice.getByRole("button", { name: "Add reaction" })).toHaveCount(1);
  // This page loads no app stylesheet (its own geometry is fixed at 48px per
  // row), so the one-line layout is measured by e2e/notice-row-layout.spec.ts
  // against the full app harness; here only the DOM contract is asserted.

  const plainSpoiler = page.locator('[data-item-id="$spoiler:example.invalid"]');
  await expect(plainSpoiler).toContainText("keep");
  await expect(plainSpoiler).toContainText("hidden");
  await expect(plainSpoiler).not.toContainText("secret");
  await plainSpoiler.locator(".message-spoiler").click();
  await expect(plainSpoiler).toContainText("secret");

  const formattedSpoiler = page.locator(
    '[data-item-id="$formatted-spoiler:example.invalid"]'
  );
  await expect(formattedSpoiler).not.toContainText("secret");
  await formattedSpoiler.locator('.message-spoiler[data-spoiler-reason="reason"]').click();
  await expect(formattedSpoiler).toContainText("secret");
});
