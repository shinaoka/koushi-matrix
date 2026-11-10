import { expect, test } from "@playwright/test";

import { threadTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";
import {
  HARNESS_ACCOUNT_KEY,
  HARNESS_ROOM_ID,
  gotoReadyShell,
  invocationCount,
  seedTimelineItems
} from "./support/basicOperations";

const ROOT = "$thread-root-1854:example.invalid";
const TARGET = "$thread-reply-1854:example.invalid";

const quote = {
  event_id: TARGET,
  sender: "@harness-alice:example.invalid",
  sender_label: "Harness Alice",
  body_preview: "Synthetic thread reply",
  media: null,
  permalink: `https://matrix.to/#/${HARNESS_ROOM_ID}/${TARGET}`,
  can_open: true,
  state: "ready"
};

function roomMessageWithQuote() {
  return {
    id: { Event: { event_id: "$room-message-1854:example.invalid" } },
    sender: "@harness-bob:example.invalid",
    sender_label: "Harness Bob",
    body: "Synthetic room message quoting a thread reply",
    timestamp_ms: 1_800_000_100_000,
    in_reply_to_event_id: TARGET,
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
  };
}

function threadReply() {
  return {
    id: { Event: { event_id: TARGET } },
    sender: "@harness-alice:example.invalid",
    sender_label: "Harness Alice",
    body: "Synthetic thread reply the quote names",
    timestamp_ms: 1_800_000_000_500,
    in_reply_to_event_id: ROOT,
    thread_root: ROOT,
    thread_summary: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}

/// Publish the Rust state a thread-routed navigation produces: the thread pane
/// holds the target as its pinned reply and the event navigation has settled as
/// `idle` (it never reaches `anchored`/`liveFallback`).
async function publishThreadPane(
  page: import("@playwright/test").Page,
  kind: "opening" | "open"
) {
  await page.evaluate(
    ({ nextKind, roomId, rootEventId, targetEventId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const thread =
        nextKind === "open"
          ? {
              kind: "open" as const,
              room_id: roomId,
              root_event_id: rootEventId,
              intent: { pinnedReply: { event_id: targetEventId } },
              is_subscribed: true,
              staged_uploads: [],
              composer: { ...snapshot.state.ui.timeline.composer }
            }
          : {
              kind: "opening" as const,
              room_id: roomId,
              root_event_id: rootEventId,
              intent: { pinnedReply: { event_id: targetEventId } }
            };
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            navigation: {
              ...snapshot.state.ui.navigation,
              active_room_id: roomId,
              main_timeline_anchor: null,
              event_navigation: { kind: "idle" as const }
            },
            timeline: { ...snapshot.state.ui.timeline, room_id: roomId, is_subscribed: true },
            thread,
            focused_context: { kind: "closed" as const }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { nextKind: kind, roomId: HARNESS_ROOM_ID, rootEventId: ROOT, targetEventId: TARGET }
  );
}

test("a reply quote to a reply inside a thread reveals the thread pane Rust opened (#1854)", async ({
  page
}) => {
  await gotoReadyShell(page);
  await seedTimelineItems(page, [roomMessageWithQuote()]);
  // The reader has no right panel open.
  await expect(page.getByText(t("panel.thread"), { exact: true })).toHaveCount(0);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.getByRole("link", { name: t("timeline.openOriginalMessage") }).first().click();
  await expect.poll(() => invocationCount(page, "open_reply_quote_event")).toBe(1);

  // Core routes the quoted target into the thread pane. The renderer must reveal
  // that pane from Rust state alone, for any navigation source.
  await publishThreadPane(page, "opening");
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  const threadKey = threadTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID, ROOT);
  await page.evaluate(
    ({ key, item }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key,
            generation: 1,
            items: [item]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: threadKey, item: threadReply() }
  );
  await publishThreadPane(page, "open");

  // The quoted target is visible in the thread pane, not hidden behind a closed
  // panel and not in the main pane.
  await expect(page.locator(`[data-item-id="${TARGET}"]`)).toBeVisible();
});
