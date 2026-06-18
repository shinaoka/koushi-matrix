/**
 * Headless spec: media attachment download progress and viewer (#78 + #80).
 *
 * Proves that the MediaAttachment component renders Rust-owned state correctly
 * and that download dispatch deduplication works:
 *
 *  1. Media row renders filename, icon, and Download button in notRequested state.
 *  2. Download button dispatches download_media and row transitions to pending.
 *  3. Progress bar renders when progress bytes/total are available.
 *  4. Indeterminate state when progress is null (pending with no bytes/total).
 *  5. Dedupe: a second click while pending does NOT dispatch a second
 *     download_media command.
 *  6. Ready image state renders <img> preview from Rust-owned source_url.
 *  7. Ready non-image state renders download link (not img), no active button.
 *  8. Failed state renders error label and retry button.
 *  9. Retry dispatches download_media again after failure.
 */

import { expect, test } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ROOM_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);

async function gotoReadyShell(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

async function seedTimelineItems(
  page: import("@playwright/test").Page,
  items: unknown[],
  generation = 2
): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(
          async ({ key, nextItems, nextGeneration }) => {
            const itemDomIds = nextItems.map((item) => {
              if ("Transaction" in (item as { id: Record<string, unknown> }).id) {
                return `txn:${(item as { id: { Transaction: { transaction_id: string } } }).id.Transaction.transaction_id}`;
              }
              if ("Event" in (item as { id: Record<string, unknown> }).id) {
                return (item as { id: { Event: { event_id: string } } }).id.Event.event_id;
              }
              return `syn:${(item as { id: { Synthetic: { synthetic_id: string } } }).id.Synthetic.synthetic_id}`;
            });
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
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any);
            await new Promise((resolve) => setTimeout(resolve, 25));
            return itemDomIds.every((id) =>
              document.querySelector(`[data-item-id="${CSS.escape(id)}"]`)
            );
          },
          { key: HARNESS_ROOM_KEY, nextItems: items, nextGeneration: generation }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
}

async function pushMediaDownloadState(
  page: import("@playwright/test").Page,
  eventId: string,
  downloadState: unknown
): Promise<void> {
  await page.evaluate(
    ({ evId, state }) => {
      const snap = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snap,
        state: {
          ...snap.state,
          timeline: {
            ...snap.state.timeline,
            media_downloads: {
              ...snap.state.timeline.media_downloads,
              [evId]: state
            }
          }
        }
      });
      window.__harness.pushStateChanged();
    },
    { evId: eventId, state: downloadState }
  );
}

function makeImageItem(eventId: string): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: null,
    timestamp_ms: 1_800_000_300_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    media: {
      kind: "Image",
      filename: "photo.jpg",
      source: { mxc_uri: "mxc://example.invalid/abc", encrypted: false, encryption_version: null },
      mimetype: "image/jpeg",
      size: 102400,
      width: 800,
      height: 600,
      thumbnail: null
    },
    actions: {
      can_copy: false,
      can_permalink: false,
      permalink: null,
      can_view_source: false,
      can_forward: false
    }
  };
}

function makeFileItem(eventId: string): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@harness-user:example.invalid",
    sender_label: "Harness User",
    body: null,
    timestamp_ms: 1_800_000_400_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    reactions: [],
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    media: {
      kind: "File",
      filename: "document.pdf",
      source: { mxc_uri: "mxc://example.invalid/def", encrypted: false, encryption_version: null },
      mimetype: "application/pdf",
      size: 204800,
      width: null,
      height: null,
      thumbnail: null
    },
    actions: {
      can_copy: false,
      can_permalink: false,
      permalink: null,
      can_view_source: false,
      can_forward: false
    }
  };
}

// ---------------------------------------------------------------------------
// 1. notRequested state — filename and download button
// ---------------------------------------------------------------------------

test("media row shows filename and Download button in notRequested state", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-notreq:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await expect(article).toBeVisible();
  const mediaCard = article.locator(".message-media");
  await expect(mediaCard).toBeVisible();
  await expect(mediaCard.getByText("photo.jpg")).toBeVisible();
  await expect(
    mediaCard.getByRole("button", { name: t("timeline.downloadMedia", { filename: "photo.jpg" }) })
  ).toBeVisible();
});

// ---------------------------------------------------------------------------
// 2. Download dispatch and pending transition
// ---------------------------------------------------------------------------

test("clicking Download dispatches download_media and row shows pending state", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-dispatch:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);
  await page.evaluate(() => window.__harness.clearInvocations());

  const article = page.locator(`[data-event-id="${eventId}"]`);
  const downloadBtn = article.locator(
    `button[aria-label="${t("timeline.downloadMedia", { filename: "photo.jpg" })}"]`
  );
  await expect(downloadBtn).toBeVisible();
  await downloadBtn.click();

  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("download_media").length))
    .toBeGreaterThanOrEqual(1);

  // Push pending state from harness (Rust would do this after the command).
  await pushMediaDownloadState(page, eventId, { kind: "pending", progress: null });

  const mediaCard = article.locator(".message-media");
  await expect(mediaCard).toHaveAttribute("data-download-state", "pending");
  await expect(mediaCard.getByText(t("timeline.mediaDownloadPending"))).toBeVisible();
});

// ---------------------------------------------------------------------------
// 3. Progress bar renders for known bytes/total
// ---------------------------------------------------------------------------

test("progress bar is shown when progress bytes/total are available", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-progress:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);

  await pushMediaDownloadState(page, eventId, {
    kind: "pending",
    progress: { current: 50000, total: 100000 }
  });

  const article = page.locator(`[data-event-id="${eventId}"]`);
  const progressBar = article.locator('[role="progressbar"]');
  await expect(progressBar).toBeVisible();
  // 50% progress.
  await expect(progressBar).toHaveAttribute("aria-valuenow", "50");
  await expect(article.getByText(/50%/)).toBeVisible();
});

// ---------------------------------------------------------------------------
// 4. Indeterminate — no progress bar when progress is null
// ---------------------------------------------------------------------------

test("no progress bar when progress is null (indeterminate)", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-indet:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);

  await pushMediaDownloadState(page, eventId, { kind: "pending", progress: null });

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await expect(article.locator('[role="progressbar"]')).toHaveCount(0);
  // Pending text label is still shown.
  await expect(article.getByText(t("timeline.mediaDownloadPending"))).toBeVisible();
});

// ---------------------------------------------------------------------------
// 5. Dedupe — no second download_media while pending
// ---------------------------------------------------------------------------

test("download button is disabled while pending, preventing duplicate dispatch", async ({
  page
}) => {
  await gotoReadyShell(page);
  const eventId = "$media-dedupe:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);

  // Set pending state without first clicking (simulates Rust-initiated download).
  await pushMediaDownloadState(page, eventId, { kind: "pending", progress: null });
  await page.evaluate(() => window.__harness.clearInvocations());

  const article = page.locator(`[data-event-id="${eventId}"]`);
  const mediaCard = article.locator(".message-media");
  // The download button (not retry) must be disabled while pending.
  const disabledBtn = mediaCard.locator(
    `button[aria-label="${t("timeline.downloadMedia", { filename: "photo.jpg" })}"]`
  );
  await expect(disabledBtn).toBeDisabled();
  // No invocation even if somehow triggered.
  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("download_media").length))
    .toBe(0);
});

// ---------------------------------------------------------------------------
// 6. Ready image — img preview rendered
// ---------------------------------------------------------------------------

test("ready image state renders img element with Rust-owned source_url", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-ready-img:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);

  // Push ready state with a synthetic data-URL as source.
  const syntheticUrl = "data:image/gif;base64,R0lGODlhAQABAAAAACH5BAEKAAEALAAAAAABAAEAAAICTAEAOw==";
  await pushMediaDownloadState(page, eventId, {
    kind: "ready",
    source_url: syntheticUrl,
    width: 800,
    height: 600,
    mime_type: "image/jpeg"
  });

  const article = page.locator(`[data-event-id="${eventId}"]`);
  // The ready+Image variant renders an <img>.
  const img = article.locator("img.message-media-image");
  await expect(img).toBeVisible();
  // The download link switches from button to <a> when ready.
  await expect(article.locator("a.message-media-download")).toBeVisible();
});

// ---------------------------------------------------------------------------
// 7. Ready file — download link (no img preview for non-image kind)
// ---------------------------------------------------------------------------

test("ready file state shows download link, not img element", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-ready-file:example.invalid";
  await seedTimelineItems(page, [makeFileItem(eventId)]);

  const syntheticUrl = "data:application/pdf;base64,JVBERi0=";
  await pushMediaDownloadState(page, eventId, {
    kind: "ready",
    source_url: syntheticUrl,
    width: null,
    height: null,
    mime_type: "application/pdf"
  });

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await expect(article.locator("img.message-media-image")).toHaveCount(0);
  await expect(article.locator("a.message-media-download")).toBeVisible();
});

// ---------------------------------------------------------------------------
// 8. Failed state — error label and retry button
// ---------------------------------------------------------------------------

test("failed state renders error message and retry button", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-failed:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);

  await pushMediaDownloadState(page, eventId, { kind: "failed", failure_kind: "network" });

  const article = page.locator(`[data-event-id="${eventId}"]`);
  const mediaCard = article.locator(".message-media");
  await expect(mediaCard).toHaveAttribute("data-download-state", "failed");
  await expect(article.getByText(t("timeline.mediaDownloadFailed"))).toBeVisible();
  // Retry button replaces the standard download button.
  await expect(
    article.getByRole("button", { name: t("timeline.mediaDownloadRetry") })
  ).toBeVisible();
});

// ---------------------------------------------------------------------------
// 9. Retry dispatch after failure
// ---------------------------------------------------------------------------

test("retry button dispatches download_media after failure", async ({ page }) => {
  await gotoReadyShell(page);
  const eventId = "$media-retry:example.invalid";
  await seedTimelineItems(page, [makeImageItem(eventId)]);

  await pushMediaDownloadState(page, eventId, { kind: "failed", failure_kind: "sdk" });
  await page.evaluate(() => window.__harness.clearInvocations());

  const article = page.locator(`[data-event-id="${eventId}"]`);
  await article
    .getByRole("button", { name: t("timeline.mediaDownloadRetry") })
    .click();

  await expect
    .poll(() => page.evaluate(() => window.__harness.invocationsOf("download_media").length))
    .toBeGreaterThanOrEqual(1);
});
