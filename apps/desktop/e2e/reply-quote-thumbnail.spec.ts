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
                width: 96,
                height: 96,
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
