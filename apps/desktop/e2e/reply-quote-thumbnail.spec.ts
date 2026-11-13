import { expect, test } from "@playwright/test";

import {
  HARNESS_ROOM_KEY,
  gotoReadyShell,
  seedTimelineItems
} from "./support/basicOperations";

const QUOTED_EVENT_ID = "$off-window-picture:example.invalid";

const quote = {
  event_id: QUOTED_EVENT_ID,
  sender: "@harness-alice:example.invalid",
  sender_label: "Harness Alice",
  body_preview: null,
  media: {
    kind: "image",
    animated: false,
    width: 640,
    height: 480,
    has_thumbnail_source: false
  },
  permalink: null,
  can_open: true,
  state: "ready"
};

test("a reply to an off-window picture requests one thumbnail and renders it", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$reply-a:example.invalid" } },
      sender: "@harness-bob:example.invalid",
      sender_label: "Harness Bob",
      body: "first reply",
      timestamp_ms: 1_800_000_100_000,
      in_reply_to_event_id: QUOTED_EVENT_ID,
      reply_quote: quote,
      thread_root: null,
      thread_summary: null,
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_react: true,
      can_redact: true,
      is_edited: false,
      can_edit: true,
      reactions: []
    },
    {
      id: { Event: { event_id: "$reply-b:example.invalid" } },
      sender: "@harness-carol:example.invalid",
      sender_label: "Harness Carol",
      body: "second reply",
      timestamp_ms: 1_800_000_100_100,
      in_reply_to_event_id: QUOTED_EVENT_ID,
      reply_quote: quote,
      thread_root: null,
      thread_summary: null,
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_react: true,
      can_redact: true,
      is_edited: false,
      can_edit: true,
      reactions: []
    }
  ]);

  await expect
    .poll(async () =>
      page.evaluate(() =>
        window.__harness
          .invocationsOf("request_reply_quote_thumbnail")
          .map((invocation) => invocation.args)
      )
    )
    .toEqual([
      {
        roomId: "!harness-room:example.invalid",
        quotedEventId: QUOTED_EVENT_ID,
        timelineKey: HARNESS_ROOM_KEY
      }
    ]);

  const thumbnailUrl =
    "data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=";
  await page.evaluate(async ({ quotedEventId, sourceRef }) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          timeline: {
            ...snapshot.state.ui.timeline,
            reply_quote_thumbnails: {
              [quotedEventId]: {
                kind: "ready",
                source_ref: sourceRef,
                width: 480,
                height: 360,
                mime_type: "image/gif"
              }
            },
            media_downloads: {
              ...snapshot.state.ui.timeline.media_downloads,
              [quotedEventId]: {
                kind: "ready",
                source_ref: sourceRef,
                width: 640,
                height: 480,
                mime_type: "image/gif"
              }
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, { quotedEventId: QUOTED_EVENT_ID, sourceRef: thumbnailUrl });

  const slots = page.getByRole("button", { name: "Open image from Harness Alice" });
  await expect(slots).toHaveCount(2);
  await expect(slots.first().locator("img")).toHaveAttribute("src", thumbnailUrl);
  await slots.first().click();
  await expect(page.getByRole("dialog")).toBeVisible();
});

// A portrait stand-in for a quote whose media hints are missing, so the box
// falls back to 4:3 while the image itself is taller than that.
const PORTRAIT_IMAGE =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='120' height='480'%3E%3Crect width='120' height='480' fill='%23b83333'/%3E%3C/svg%3E";

test("an off-ratio thumbnail is centered in the reserved box and the quote keeps its height", async ({
  page
}) => {
  await gotoReadyShell(page);

  const replyId = "$reply-portrait:example.invalid";
  const quotedEventId = "$portrait-picture:example.invalid";
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: replyId } },
      sender: "@harness-bob:example.invalid",
      sender_label: "Harness Bob",
      body: "no hints here",
      timestamp_ms: 1_800_000_200_000,
      in_reply_to_event_id: quotedEventId,
      reply_quote: {
        ...quote,
        event_id: quotedEventId,
        // Hints absent: the box falls back to 4:3 while the image is portrait.
        media: {
          kind: "image",
          animated: false,
          width: null,
          height: null,
          has_thumbnail_source: false
        }
      },
      thread_root: null,
      thread_summary: null,
      media: null,
      is_redacted: false,
      is_hidden: false,
      can_react: true,
      can_redact: true,
      is_edited: false,
      can_edit: true,
      reactions: []
    }
  ]);

  const row = page.locator(`[data-item-id="${replyId}"]`);
  const preview = row.locator(".reply-quote-thumbnail");
  const quoteBox = row.locator(".reply-quote");
  const rectOf = async (locator: typeof preview) => {
    const box = await locator.boundingBox();
    if (!box) {
      throw new Error("element has no box");
    }
    return box;
  };
  const boxHeight = async () => (await quoteBox.boundingBox())?.height ?? Number.NaN;

  const reservedPreview = await rectOf(preview);
  const reservedHeight = await boxHeight();

  await page.evaluate(
    async ({ quotedEventId: id, sourceRef }) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            timeline: {
              ...snapshot.state.ui.timeline,
              reply_quote_thumbnails: {
                [id]: {
                  kind: "ready",
                  source_ref: sourceRef,
                  width: 480,
                  height: 360,
                  mime_type: "image/svg+xml"
                }
              }
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { quotedEventId, sourceRef: PORTRAIT_IMAGE }
  );

  const image = preview.locator("img");
  await expect(image).toHaveAttribute("src", PORTRAIT_IMAGE);
  await expect
    .poll(() =>
      image.evaluate((element) => {
        const img = element as HTMLImageElement;
        return img.complete && img.naturalHeight === 480;
      })
    )
    .toBe(true);

  const readyPreview = await rectOf(preview);
  // (a) The reserved box already had its final size, so nothing jumps.
  expect(await boxHeight()).toBeCloseTo(reservedHeight, 1);
  expect(readyPreview.height).toBeCloseTo(reservedPreview.height, 1);
  expect(readyPreview.width).toBeCloseTo(reservedPreview.width, 1);
  expect(readyPreview.y).toBeCloseTo(reservedPreview.y, 1);

  // (b) The image element fills the box's padding box instead of letting its
  // intrinsic height overflow the box and anchoring the visible part at the top.
  const { contentBox, imageBox } = await preview.evaluate((element) => {
    const img = element.querySelector("img");
    if (!img) {
      throw new Error("no image");
    }
    const outer = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    const rect = img.getBoundingClientRect();
    const borderX = parseFloat(style.borderLeftWidth) + parseFloat(style.borderRightWidth);
    const borderY = parseFloat(style.borderTopWidth) + parseFloat(style.borderBottomWidth);
    return {
      contentBox: {
        top: outer.top + parseFloat(style.borderTopWidth),
        left: outer.left + parseFloat(style.borderLeftWidth),
        width: outer.width - borderX,
        height: outer.height - borderY
      },
      imageBox: {
        top: rect.top,
        left: rect.left,
        width: rect.width,
        height: rect.height
      }
    };
  });
  expect(imageBox.height).toBeCloseTo(contentBox.height, 1);
  expect(imageBox.width).toBeCloseTo(contentBox.width, 1);
  expect(imageBox.top).toBeCloseTo(contentBox.top, 1);
  expect(imageBox.left).toBeCloseTo(contentBox.left, 1);

  // (c) The picture is scaled down and centered in that box, not cropped: a
  // wider-than-3:1 formula or screenshot keeps its edges.
  const fit = await preview.locator("img").evaluate((img) => {
    const style = getComputedStyle(img);
    return { objectFit: style.objectFit, objectPosition: style.objectPosition };
  });
  expect(fit).toEqual({ objectFit: "contain", objectPosition: "50% 50%" });
});
