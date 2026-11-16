/**
 * Headless spec: editor-only composers never invite a file they cannot keep
 * (#1904).
 *
 * An inline message edit, a scheduled-send edit and an upload caption reuse the
 * main composer in editor-only mode. They used to draw the "Drop files to
 * attach" overlay and then discard the drop, because the attach callback
 * defaulted to a no-op. This drives the real App -> TauriDesktopApi -> IPC mock
 * path for the two mounts reachable here: the inline editor must not advertise
 * or swallow a drop, and a file dropped on an open staging dialog (including on
 * its caption field) joins the staged list through `stage_upload_bytes`.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import {
  gotoReadyShell,
  invocationCount,
  seedTimelineItems
} from "./support/basicOperations";

const EDIT_EVENT_ID = "$editor-drop:example.invalid";

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

async function dispatchFileDrag(
  page: Page,
  selector: string,
  type: "dragenter" | "dragover" | "drop",
  filename: string
): Promise<boolean> {
  return page.evaluate(
    ({ selector, type, filename }) => {
      const target = document.querySelector(selector);
      if (!target) {
        throw new Error(`${selector} not found`);
      }
      const data = new DataTransfer();
      data.items.add(new File(["fixture bytes"], filename, { type: "application/pdf" }));
      const event = new DragEvent(type, {
        bubbles: true,
        cancelable: true,
        dataTransfer: data
      });
      target.dispatchEvent(event);
      return event.defaultPrevented;
    },
    { selector, type, filename }
  );
}

test("an inline edit neither invites nor swallows a dropped file (#1904)", async ({ page }) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [editableItem()]);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("button", { name: /edit message/i }).click();
  const editor = page.getByRole("textbox", { name: t("timeline.editBody") });
  await expect(editor).toBeVisible();

  const editComposer = page.locator("section.composer.is-editor-only").first();
  await editor.evaluate((element) => {
    const data = new DataTransfer();
    data.items.add(new File(["fixture bytes"], "edit-over.pdf", { type: "application/pdf" }));
    for (const type of ["dragenter", "dragover"]) {
      element.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: data }));
    }
  });

  await expect(editComposer).toHaveAttribute("data-file-drag-over", "false");
  await expect(editComposer.locator(".composer-drop-overlay")).toHaveAttribute("aria-hidden", "true");

  await editor.evaluate((element) => {
    const data = new DataTransfer();
    data.items.add(new File(["fixture bytes"], "edit-dropped.pdf", { type: "application/pdf" }));
    element.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
  });

  await expect.poll(() => invocationCount(page, "stage_upload_bytes")).toBe(0);
  await expect(page.getByRole("dialog", { name: t("upload.dialogTitle") })).toHaveCount(0);
  await expect(editor).toHaveText("old body");
});

test("a file dropped on the open staging dialog joins its staged list (#1904)", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  // The control: a drop on the main composer stages the first file.
  await dispatchFileDrag(page, "section.composer", "drop", "first.pdf");
  await expect.poll(() => invocationCount(page, "stage_upload_bytes")).toBe(1);
  await expect(page.getByRole("dialog", { name: t("upload.dialogTitle") })).toBeVisible();

  // A second file dropped on the caption field is addressed to the dialog, so
  // the caption must not advertise the drop and the dialog must keep the file.
  const caption = page.getByRole("textbox", { name: "Caption for first.pdf" });
  await expect(caption).toBeVisible();
  const captionComposer = page
    .locator("section.composer.is-editor-only")
    .filter({ has: caption });
  await caption.evaluate((element) => {
    const data = new DataTransfer();
    data.items.add(new File(["fixture bytes"], "second-hover.pdf", { type: "application/pdf" }));
    for (const type of ["dragenter", "dragover"]) {
      element.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: data }));
    }
  });
  await expect(captionComposer).toHaveAttribute("data-file-drag-over", "false");

  await dispatchFileDrag(
    page,
    '[aria-label="Caption for first.pdf"]',
    "drop",
    "second.pdf"
  );

  await expect.poll(() => invocationCount(page, "stage_upload_bytes")).toBe(2);
  await expect(page.getByText("second.pdf", { exact: true })).toBeVisible();
  await expect(page.getByText("first.pdf", { exact: true })).toBeVisible();
});
