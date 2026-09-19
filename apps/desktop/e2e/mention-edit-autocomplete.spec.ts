/**
 * Headless spec: inline message editing queries its own mention surface (#936).
 *
 * The edit Composer must not reuse the main Composer's candidate target. This
 * drives the full App -> TauriDesktopApi -> IPC mock path and verifies both the
 * command payload and the projected edit candidates.
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";
import {
  gotoReadyShell,
  HARNESS_ROOM_ID,
  seedTimelineItems
} from "./support/basicOperations";

const EDIT_EVENT_ID = "$mention-edit:example.invalid";

function editableItem(): Record<string, unknown> {
  return {
    id: { Event: { event_id: EDIT_EVENT_ID } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: "old body",
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
    actions: {
      can_copy: true,
      can_forward: true,
      can_reply: true,
      can_permalink: true,
      can_view_source: true,
      editable_document: {
        version: 2,
        inlines: [{ kind: "text", text: "old body" }]
      }
    }
  };
}

test("inline edit autocomplete uses the edit candidate target", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [editableItem()]);

  await page.getByRole("button", { name: /edit message/i }).click();
  const editor = page.getByRole("textbox", { name: t("timeline.editBody") });
  await editor.fill("@ed");

  await expect
    .poll(
      async () =>
        page.evaluate(() => window.__harness.invocationsOf("query_mention_candidates").at(-1)?.args),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toMatchObject({ roomId: HARNESS_ROOM_ID, surface: "edit", query: "ed" });

  await expect(page.getByRole("listbox", { name: t("composer.mentionSuggestions") })).toBeVisible();
  await expect(
    page.getByRole("option", { name: "Edit Candidate @edit:example.invalid" })
  ).toBeVisible();
});
