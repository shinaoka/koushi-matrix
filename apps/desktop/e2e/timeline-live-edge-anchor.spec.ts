/**
 * Headless regression for the DM-return live-edge bug:
 * after an own outgoing event pins the viewport to the live edge, the same
 * bottom-most event must be persisted as the room scroll anchor. Switching to
 * another room and back should restore that event into view without requiring
 * a fresh timeline replay.
 */

import { expect, test } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";

const ACCOUNT_KEY = "@harness-user:example.invalid";
const ROOM_A_ID = "!harness-room:example.invalid";
const ROOM_B_ID = "!live-edge-return-other:example.invalid";
const ROOM_A_KEY = roomTimelineKey(ACCOUNT_KEY, ROOM_A_ID);
const SENT_EVENT_ID = "$live-edge-sent:example.invalid";

function makeEventItem(
  eventId: string,
  index: number,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    id: { Event: { event_id: eventId } },
    sender: index % 2 === 0 ? ACCOUNT_KEY : "@other-user:example.invalid",
    sender_label: index % 2 === 0 ? "Harness User" : "Other User",
    body: `live-edge anchor message ${index}\nThis row has enough text to occupy stable vertical space.`,
    timestamp_ms: 1_800_100_000_000 + index * 1000,
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
    send_state: null,
    ...overrides
  };
}

async function gotoReadyApp(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
}

async function seedRoomATimeline(page: import("@playwright/test").Page): Promise<void> {
  const items = Array.from({ length: 28 }, (_, index) =>
    makeEventItem(`$live-edge-seed-${index}:example.invalid`, index)
  );
  await page.evaluate(
    async ({ key, nextItems }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key,
            generation: 1,
            items: nextItems
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: ROOM_A_KEY, nextItems: items }
  );
  await expect(page.getByText("live-edge anchor message 27")).toBeVisible({
    timeout: 5000
  });
}

async function switchActiveRoom(
  page: import("@playwright/test").Page,
  roomId: string,
  roomName: string
): Promise<void> {
  await page.evaluate(
    ({ nextRoomId, nextRoomName }) => {
      const snapshot = window.__harness.currentSnapshot();
      const rooms = snapshot.state.domain.rooms.some((room) => room.room_id === nextRoomId)
        ? snapshot.state.domain.rooms
        : [
            ...snapshot.state.domain.rooms,
            {
              room_id: nextRoomId,
              display_name: nextRoomName,
              display_label: nextRoomName,
              original_display_label: nextRoomName,
              avatar: null,
              is_dm: true,
              dm_user_ids: ["@other-user:example.invalid"],
              tags: { favourite: null, low_priority: null },
              unread_count: 0,
              notification_count: 0,
              highlight_count: 0,
              parent_space_ids: [],
              dm_space_ids: [],
              is_encrypted: false,
              joined_members: 2
            }
          ];
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            rooms
          },
          ui: {
            ...snapshot.state.ui,
            navigation: {
              ...snapshot.state.ui.navigation,
              active_space_id: null,
              active_room_id: nextRoomId
            },
            timeline: {
              ...snapshot.state.ui.timeline,
              room_id: nextRoomId,
              is_subscribed: true,
              composer: {
                ...snapshot.state.ui.timeline.composer,
                pending_transaction_id: null,
                draft: "",
                mode: "Plain"
              }
            },
            thread: { kind: "closed" },
            threads_list: { kind: "closed" },
            focused_context: { kind: "closed" }
          }
        }
      });
      window.__harness.pushStateChanged();
    },
    { nextRoomId: roomId, nextRoomName: roomName }
  );
  await page.waitForTimeout(350);
}

test("own outgoing live-edge event is restored after switching away and back", async ({
  page
}) => {
  await gotoReadyApp(page);

  await page.evaluate(() => {
    window.__harness.setCommandResponse(
      "update_navigation_scroll_anchor",
      ({ roomId, anchor }) => {
        const snapshot = window.__harness.currentSnapshot();
        const next = {
          ...snapshot,
          state: {
            ...snapshot.state,
            ui: {
              ...snapshot.state.ui,
              navigation: {
                ...snapshot.state.ui.navigation,
                room_scroll_anchors: {
                  ...(snapshot.state.ui.navigation.room_scroll_anchors ?? {}),
                  [roomId]: anchor
                }
              }
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      }
    );
  });

  await seedRoomATimeline(page);

  await page.evaluate(
    async ({ key, eventId }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                PushBack: {
                  item: makeOutgoingEvent(eventId)
                }
              }
            ]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);

      function makeOutgoingEvent(nextEventId: string) {
        return {
          id: { Event: { event_id: nextEventId } },
          sender: "@harness-user:example.invalid",
          sender_label: "Harness User",
          body: "Message sent at the live edge",
          timestamp_ms: 1_800_100_100_000,
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
          send_state: { kind: "sending" }
        };
      }
    },
    { key: ROOM_A_KEY, eventId: SENT_EVENT_ID }
  );

  await expect(page.locator(`[data-event-id="${SENT_EVENT_ID}"]`)).toBeVisible({
    timeout: 5000
  });

  await expect
    .poll(
      async () =>
        page.evaluate((eventId) => {
          const invocations = window.__harness.invocationsOf(
            "update_navigation_scroll_anchor"
          );
          for (let index = invocations.length - 1; index >= 0; index -= 1) {
            const candidate = invocations[index];
            if (candidate.args.anchor?.event_id === eventId) {
              return candidate.args.anchor;
            }
          }
          return null;
        }, SENT_EVENT_ID),
      { timeout: 5000, intervals: [25, 50, 100, 250] }
    )
    .not.toBeNull();

  const persistedAnchor = await page.evaluate((eventId) => {
    const invocations = window.__harness.invocationsOf(
      "update_navigation_scroll_anchor"
    );
    for (let index = invocations.length - 1; index >= 0; index -= 1) {
      const candidate = invocations[index];
      if (candidate.args.anchor?.event_id === eventId) {
        return candidate.args.anchor as { offset_px: number };
      }
    }
    return null;
  }, SENT_EVENT_ID);
  expect(persistedAnchor).not.toBeNull();
  if (!persistedAnchor) {
    throw new Error("sent event anchor was not persisted");
  }

  await switchActiveRoom(page, ROOM_B_ID, "Return Test DM");
  await switchActiveRoom(page, ROOM_A_ID, "Harness Room");

  await expect
    .poll(
      async () =>
        page.evaluate(
          ({ eventId }) => {
            const row = document.querySelector<HTMLElement>(
              `[data-event-id="${eventId}"]`
            );
            const container = row?.closest<HTMLElement>("[data-testid=timeline-view]");
            if (!row || !container) {
              return null;
            }
            const rowRect = row.getBoundingClientRect();
            const containerRect = container.getBoundingClientRect();
            return {
              rowBottomOffset: Math.round(rowRect.bottom - containerRect.bottom),
              rowTop: rowRect.top,
              rowBottom: rowRect.bottom,
              containerTop: containerRect.top,
              containerBottom: containerRect.bottom
            };
          },
          { eventId: SENT_EVENT_ID }
        ),
      { timeout: 5000, intervals: [25, 50, 100, 250] }
    )
    .not.toBeNull();

  const geometry = await page.evaluate(
    ({ eventId }) => {
      const row = document.querySelector<HTMLElement>(
        `[data-event-id="${eventId}"]`
      );
      const container = row?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!row || !container) {
        return null;
      }
      const rowRect = row.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      return {
        rowBottomOffset: Math.round(rowRect.bottom - containerRect.bottom),
        rowTop: rowRect.top,
        rowBottom: rowRect.bottom,
        containerTop: containerRect.top,
        containerBottom: containerRect.bottom
      };
    },
    { eventId: SENT_EVENT_ID }
  );
  expect(geometry).not.toBeNull();
  if (!geometry) {
    throw new Error("sent event row was not restored into the timeline");
  }

  expect(geometry.rowBottom).toBeGreaterThanOrEqual(geometry.containerTop);
  expect(geometry.rowTop).toBeLessThanOrEqual(geometry.containerBottom);
  expect(
    Math.abs(geometry.rowBottomOffset - persistedAnchor.offset_px)
  ).toBeLessThanOrEqual(4);
});
