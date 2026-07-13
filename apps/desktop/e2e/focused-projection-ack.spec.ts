import { expect, test } from "@playwright/test";

import { focusedTimelineKey, threadTimelineKey, type TimelineItem } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";

const ACCOUNT_KEY = "@harness-user:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const EVENT_ID = "$focused-target:example.invalid";
const REQUEST_ID = { connection_id: 41, sequence: 7 };
const FOCUSED_KEY = focusedTimelineKey(ACCOUNT_KEY, ROOM_ID, EVENT_ID);

function message(eventId: string, body: string): TimelineItem {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@sender:example.invalid",
    sender_label: "Synthetic Sender",
    body,
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
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

test("applies a pre-screen Focused projection before ACK and fences a delayed old actor", async ({
  page
}) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.evaluate(
    async ({ key, requestId, item }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: requestId,
            key,
            actor_generation: 8,
            generation: 0,
            items: [item]
          }
        }
      });
    },
    { key: FOCUSED_KEY, requestId: REQUEST_ID, item: message(EVENT_ID, "applied before switch") }
  );

  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("acknowledge_timeline_projection").length
      )
    )
    .toBe(1);

  await page.evaluate(
    async ({ key, item }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: { connection_id: 40, sequence: 99 },
            key,
            actor_generation: 7,
            generation: 0,
            items: [item]
          }
        }
      });
      const snapshot = window.__harness.currentSnapshot();
      if (!("Focused" in key.kind)) {
        throw new Error("expected focused key");
      }
      snapshot.state.ui.focused_context = {
        kind: "open",
        room_id: key.kind.Focused.room_id,
        event_id: key.kind.Focused.event_id
      };
      snapshot.state.ui.navigation.main_timeline_anchor = {
        event_id: key.kind.Focused.event_id
      };
      window.__harness.setSnapshot(snapshot);
      await window.__harness.pushStateChanged();
    },
    { key: FOCUSED_KEY, item: message("$stale:example.invalid", "must not replace") }
  );

  await expect(page.getByText("applied before switch", { exact: true })).toBeVisible();
  await expect(page.getByText("must not replace", { exact: true })).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("acknowledge_timeline_projection").length
      )
    )
    .toBe(1);
});

test("thread InitialItems uses the same canonical-store acknowledgement contract", async ({
  page
}) => {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  await page.evaluate(() => window.__harness.clearInvocations());
  const key = threadTimelineKey(ACCOUNT_KEY, ROOM_ID, "$thread-root:example.invalid");

  await page.evaluate(
    async ({ threadKey, item }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: { connection_id: 42, sequence: 1 },
            key: threadKey,
            actor_generation: 3,
            generation: 0,
            items: [item]
          }
        }
      });
    },
    { threadKey: key, item: message("$thread-reply:example.invalid", "thread reply") }
  );

  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("acknowledge_timeline_projection").length
      )
    )
    .toBe(1);
});
