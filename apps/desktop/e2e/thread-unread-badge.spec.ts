/**
 * Headless spec: the thread unread count sits on the summary chip as a red
 * badge overlapping its top-end corner, instead of a bare number on its own
 * line above the chip. All content is synthetic.
 */

import { expect, test } from "@playwright/test";
import { gotoReadyShell, seedTimelineItems } from "./support/basicOperations";

const ROOT_ID = "$thread-unread-badge:example.invalid";

test("the unread count overlaps the summary chip's top-end corner", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: ROOT_ID } },
      sender: "@harness-bob:example.invalid",
      body: "Synthetic thread root",
      timestamp_ms: 1_800_000_000_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: {
        reply_count: 2,
        unread_count: 1,
        latest_event_id: "$thread-unread-badge-latest:example.invalid",
        latest_sender: "@harness-ada:example.invalid",
        latest_sender_label: "Member One",
        latest_body_preview: "Synthetic reply",
        latest_timestamp_ms: 1_800_000_060_000
      },
      reactions: [],
      can_react: false,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
    }
  ]);

  const row = page.locator(`[data-item-id="${ROOT_ID}"]`);
  await expect(row.locator(".thread-new-replies-chip")).toHaveCount(0);
  const chip = row.locator(".thread-summary-chip");
  const badge = chip.locator(".thread-summary-unread-badge");
  await expect(badge).toHaveText("1");

  const chipBox = (await chip.boundingBox())!;
  const badgeBox = (await badge.boundingBox())!;
  // Straddles the chip's top and end edges.
  expect(badgeBox.y).toBeLessThan(chipBox.y);
  expect(badgeBox.y + badgeBox.height).toBeGreaterThan(chipBox.y);
  expect(badgeBox.x + badgeBox.width).toBeGreaterThan(chipBox.x + chipBox.width);
  expect(badgeBox.x).toBeLessThan(chipBox.x + chipBox.width);
  // A single digit renders as a circle.
  expect(Math.abs(badgeBox.width - badgeBox.height)).toBeLessThanOrEqual(1);
  // Not clipped by the row.
  await expect(badge).toBeInViewport({ ratio: 1 });

  const paint = await badge.evaluate((el) => {
    const style = getComputedStyle(el);
    return { color: style.color, background: style.backgroundColor };
  });
  expect(paint.color).toBe("rgb(255, 255, 255)");
  expect(paint.background).not.toBe("rgba(0, 0, 0, 0)");

  if (process.env.THREAD_BADGE_SHOT) {
    await row.screenshot({ path: process.env.THREAD_BADGE_SHOT });
  }
});
