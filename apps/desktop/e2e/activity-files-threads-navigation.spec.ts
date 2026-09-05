import { expect, test } from "@playwright/test";

import { focusedTimelineKey, threadTimelineKey } from "../src/domain/coreEvents";
import { t } from "../src/i18n/messages";
import {
  HARNESS_ROOM_ID,
  HARNESS_ROOM_KEY,
  gotoReadyShell,
  invocationCount
} from "./support/basicOperations";

function makeThreadItem(index: number, rootEventId = "$seed-event:example.invalid") {
  return {
    id: { Event: { event_id: `$thread-page-${String(index).padStart(2, "0")}:example.invalid` } },
    sender: "@thread-user:example.invalid",
    body: `Thread overflow message ${index}`,
    timestamp_ms: 1_800_000_001_000 + index,
    in_reply_to_event_id: rootEventId,
    thread_root: rootEventId,
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

test("Activity renders Rust-owned streams and waits for mark-read snapshots", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const recentRows = [
      {
        kind: "event",
        room_id: "!room-beta:example.invalid",
        event_id: "$activity-beta-newest:example.invalid",
        thread_root_event_id: null,
        room_label: "Project Beta",
        context_label: "Room · Project Beta",
        sender_label: "Beta Sender",
        sender_avatar: null,
        preview: "Newest recent update",
        timestamp_ms: 1_800_000_010_000,
        unread: false,
        highlight: false
      },
      {
        kind: "event",
        room_id: "!room-alpha:example.invalid",
        event_id: "$activity-alpha-middle:example.invalid",
        thread_root_event_id: null,
        room_label: "Project Alpha",
        context_label: "Room · Project Alpha",
        sender_label: "Alpha Sender",
        sender_avatar: null,
        preview: "Middle recent update",
        timestamp_ms: 1_800_000_009_000,
        unread: true,
        highlight: true
      },
      {
        kind: "event",
        room_id: "!room-gamma:example.invalid",
        event_id: "$activity-gamma-oldest:example.invalid",
        thread_root_event_id: null,
        room_label: "Project Gamma",
        context_label: "Room · Project Gamma",
        sender_label: null,
        sender_avatar: null,
        preview: "Oldest recent update",
        timestamp_ms: 1_800_000_008_000,
        unread: false,
        highlight: false
      }
    ];
    const unreadRows = [
      {
        kind: "event",
        room_id: "!room-alpha:example.invalid",
        event_id: "$activity-alpha-unread:example.invalid",
        thread_root_event_id: null,
        room_label: "Project Alpha",
        context_label: "Room · Project Alpha",
        sender_label: "Alpha Sender",
        sender_avatar: null,
        preview: "Stale unread update",
        timestamp_ms: 1_800_000_001_000,
        unread: true,
        highlight: true
      },
      {
        kind: "event",
        room_id: "!room-beta:example.invalid",
        event_id: "$activity-beta-unread:example.invalid",
        thread_root_event_id: null,
        room_label: "Project Beta",
        context_label: "Room · Project Beta",
        sender_label: "Beta Sender",
        sender_avatar: null,
        preview: "Fresh unread update",
        timestamp_ms: 1_800_000_011_000,
        unread: true,
        highlight: false
      }
    ];
    const activitySnapshot = (activeTab: "recent" | "unread", nextUnreadRows = unreadRows) => {
      const snapshot = window.__harness.currentSnapshot();
      return {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            activity: {
              kind: "open",
              active_tab: activeTab,
              recent: { rows: recentRows, next_batch: "activity-page-2", resolution: { kind: "idle" } },
              unread: { rows: nextUnreadRows, next_batch: null, resolution: { kind: "idle" } },
              mark_read: { kind: "idle" }
            }
          }
        }
      };
    };

    window.__harness.setCommandResponse("open_activity", () => {
      const next = activitySnapshot("recent");
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("set_activity_tab", ({ tab }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            activity:
              snapshot.state.domain.activity.kind === "open"
                ? { ...snapshot.state.domain.activity, active_tab: tab }
                : snapshot.state.domain.activity
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("paginate_activity", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("mark_activity_read", () =>
      window.__harness.currentSnapshot()
    );
    window.__harness.setCommandResponse("open_activity_event", ({ roomId, eventId }) => {
      const snapshot = window.__harness.currentSnapshot();
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            navigation: {
              ...snapshot.state.ui.navigation,
              active_room_id: String(roomId),
              main_timeline_anchor: { event_id: String(eventId) },
              event_navigation: {
                kind: "anchored",
                generation:
                  (snapshot.state.ui.navigation.event_navigation.kind === "idle"
                    ? 0
                    : snapshot.state.ui.navigation.event_navigation.generation) + 1,
                source: "activity"
              }
            },
            timeline: {
              ...snapshot.state.ui.timeline,
              room_id: String(roomId),
              is_subscribed: true
            },
            thread: { kind: "closed" },
            focused_context: {
              kind: "opening",
              room_id: String(roomId),
              event_id: String(eventId)
            }
          },
          domain: {
            ...snapshot.state.domain,
            thread_attention: { kind: "closed" }
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.clearInvocations();
  });

  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();

  await expect.poll(() => invocationCount(page, "open_activity")).toBeGreaterThanOrEqual(1);
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
  const recentRows = page.locator(".activity-row");
  await expect(recentRows).toHaveCount(3);
  await expect(recentRows.nth(0)).toContainText("Newest recent update");
  await expect(recentRows.nth(1)).toContainText("Middle recent update");
  await expect(recentRows.nth(2)).toContainText("Oldest recent update");

  await page.getByRole("button", { name: "Load more activity" }).click();
  await expect.poll(() => invocationCount(page, "paginate_activity")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("paginate_activity")[0]?.args)
    )
    .toEqual({
      tab: "recent",
      cursor: "activity-page-2"
    });

  await page.getByRole("button", { name: "Open activity item Project Beta" }).click();
  await expect.poll(() => invocationCount(page, "open_activity_event")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("open_activity_event")[0]?.args)
    )
    .toEqual({
      roomId: "!room-beta:example.invalid",
      eventId: "$activity-beta-newest:example.invalid"
    });
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

  await page.getByRole("button", { name: "Activity" }).click();
  await page.getByLabel("Activity views").getByRole("tab", { name: "Unread" }).click();

  await expect.poll(() => invocationCount(page, "set_activity_tab")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("set_activity_tab")[0]?.args))
    .toEqual({ tab: "unread" });
  expect(await invocationCount(page, "mark_activity_read")).toBe(0);

  const alphaUnreadRow = page.locator(".activity-row").filter({
    hasText: "Stale unread update"
  });
  await expect(page.locator('[data-kind="roomUnread"]')).toHaveCount(0);
  await expect(alphaUnreadRow).toBeVisible();
  await alphaUnreadRow.getByRole("button", { name: "Mark room read" }).click();

  await expect.poll(() => invocationCount(page, "mark_activity_read")).toBe(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("mark_activity_read")[0]?.args)
    )
    .toEqual({
      target: {
        kind: "room",
        room_id: "!room-alpha:example.invalid",
        up_to_event_id: "$activity-alpha-unread:example.invalid"
      }
    });
  await expect(alphaUnreadRow).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    if (snapshot.state.domain.activity.kind !== "open") {
      throw new Error("expected open Activity snapshot");
    }
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          activity: {
            ...snapshot.state.domain.activity,
            unread: {
              ...snapshot.state.domain.activity.unread,
              rows: snapshot.state.domain.activity.unread.rows.filter(
                (row) => row.room_id !== "!room-alpha:example.invalid"
              )
            }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(alphaUnreadRow).toHaveCount(0);
  await page.getByRole("button", { name: "Mark all read" }).click();
  await expect.poll(() => invocationCount(page, "mark_activity_read")).toBe(2);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("mark_activity_read")[1]?.args)
    )
    .toEqual({ target: { kind: "all" } });
  await expect(page.locator(".activity-row").filter({ hasText: "Fresh unread update" })).toBeVisible();

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    if (snapshot.state.domain.activity.kind !== "open") {
      throw new Error("expected open Activity snapshot");
    }
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          activity: {
            ...snapshot.state.domain.activity,
            unread: { rows: [], next_batch: null, resolution: { kind: "idle" } }
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(page.getByText("No unread activity")).toBeVisible();
});

test("Activity Unread replaces unresolved room placeholders with retryable status", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    const placeholder = {
      kind: "roomUnread",
      room_id: "!unresolved:example.invalid",
      event_id: null,
      thread_root_event_id: null,
      sender_id: null,
      room_label: "Unresolved room",
      sender_label: null,
      sender_avatar: null,
      preview: null,
      timestamp_ms: 10,
      unread: true,
      highlight: false,
      context_label: "Room"
    };
    const withResolution = (kind: "failed" | "resolving") => {
      const snapshot = window.__harness.currentSnapshot();
      return {
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            activity: {
              kind: "open",
              active_tab: "unread",
              recent: { rows: [], next_batch: null, resolution: { kind: "idle" } },
              unread: {
                rows: [placeholder],
                next_batch: null,
                resolution: kind === "failed"
                  ? { kind, generation: 1, unresolved_room_count: 1, failure_kind: "network" }
                  : { kind, generation: 2, unresolved_room_count: 1 }
              },
              mark_read: { kind: "idle" }
            }
          }
        }
      };
    };
    window.__harness.setCommandResponse("open_activity", () => {
      const next = withResolution("failed");
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.setCommandResponse("retry_activity_resolution", () => {
      const next = withResolution("resolving");
      window.__harness.setSnapshot(next);
      return next;
    });
    window.__harness.clearInvocations();
  });

  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect(page.getByRole("alert")).toContainText("Unread messages could not be loaded");
  await expect(page.locator('[data-kind="roomUnread"]')).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Mark all read" })).toBeVisible();
  await page.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => invocationCount(page, "retry_activity_resolution")).toBe(1);
  await expect(page.getByRole("region", { name: "Unread" }).getByRole("status"))
    .toContainText("Resolving unread messages");
  await expect(page.locator('[data-kind="roomUnread"]')).toHaveCount(0);
});

test("selecting a search result opens the anchored main timeline from Rust-owned state", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  const searchInput = page.getByRole("textbox", { name: "Search" });
  await searchInput.fill("Alpha");
  await searchInput.press("Enter");

  const resultButton = page
    .getByRole("button", { name: /Alpha keyword update from demo coordinator\./ })
    .first();
  await expect(resultButton).toBeVisible();
  await resultButton.click();

  await expect.poll(() => invocationCount(page, "select_search_result")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("select_search_result")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      eventId: "$seed-event:example.invalid"
    });

  const focusedEventId = "$focused-event:example.invalid";
  const focusedKey = focusedTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );

  await page.evaluate(({ key, focusedEventId }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items: [
            {
              id: { Event: { event_id: focusedEventId } },
              sender: "@harness-user:example.invalid",
              body: "Focused context message",
              timestamp_ms: 1_800_000_000_100,
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
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: focusedKey,
    focusedEventId
  });

  await expect(
    page
      .getByRole("main", { name: t("timeline.conversation") })
      .locator('[data-event-id="$focused-event:example.invalid"]')
  ).toBeVisible();

  await page.evaluate(() => window.__harness.clearInvocations());
  await page
    .getByRole("navigation", { name: t("timeline.navigation") })
    .getByRole("button", { name: t("timeline.latest"), exact: true })
    .click();
  await expect.poll(() => invocationCount(page, "close_focused_context")).toBeGreaterThanOrEqual(1);
  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await expect(page.getByText(t("panel.roomInfo"), { exact: true })).toBeVisible();
});

test("thread summary chip opens a thread timeline from keyed CoreEvents", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await expect(page.getByRole("button", { name: /2 replies/ })).toBeVisible();
  await page.getByRole("button", { name: /2 replies/ }).click();

  await expect.poll(() => invocationCount(page, "open_thread")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("open_thread")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid",
      intent: "existingThread"
    });

  const threadEventId = "$thread-reply:example.invalid";
  const threadKey = threadTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );

  await page.evaluate(({ key, threadEventId }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items: [
            {
              id: { Event: { event_id: threadEventId } },
              sender: "@thread-user:example.invalid",
              body: "Thread panel reply from keyed event stream",
              timestamp_ms: 1_800_000_000_200,
              in_reply_to_event_id: "$seed-event:example.invalid",
              thread_root: "$seed-event:example.invalid",
              thread_summary: null,
              reactions: [],
              can_react: true,
              is_redacted: false,
              is_hidden: false,
              can_redact: false,
              is_edited: false,
              can_edit: false
            }
          ]
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: threadKey,
    threadEventId
  });

  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-event-id="$thread-reply:example.invalid"]')
  ).toBeVisible();
  await expect(
    page
      .locator('aside[aria-label="Context panel"]')
      .getByText("Thread panel reply from keyed event stream", { exact: true })
  ).toBeVisible();
});

test("empty thread timeline initial generation zero triggers thread backfill", async ({
  page
}) => {
  await gotoReadyShell(page);

  await expect(page.getByRole("button", { name: /2 replies/ })).toBeVisible();
  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  const threadKey = threadTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 0,
          items: []
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: threadKey
  });

  await expect
    .poll(() => invocationCount(page, "paginate_thread_timeline_backwards"))
    .toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("paginate_thread_timeline_backwards")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid"
    });
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);
});

test("thread panel scrollback invokes thread pagination command only", async ({
  page
}) => {
  await gotoReadyShell(page);

  await expect(page.getByRole("button", { name: /2 replies/ })).toBeVisible();
  await page.getByRole("button", { name: /2 replies/ }).click();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toBeVisible();

  const threadKey = threadTimelineKey(
    "@harness-user:example.invalid",
    "!harness-room:example.invalid",
    "$seed-event:example.invalid"
  );
  await page.evaluate(({ key, items }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key,
          generation: 1,
          items
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: threadKey,
    items: Array.from({ length: 48 }, (_, index) => makeThreadItem(index))
  });

  const threadTimeline = page.locator('aside[aria-label="Context panel"] [data-testid="timeline-view"]');
  await expect(threadTimeline.locator("[data-item-id]")).toHaveCount(48);
  await expect(
    page
      .locator('aside[aria-label="Context panel"]')
      .getByText("Thread overflow message 47", { exact: true })
  ).toBeVisible();
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      })
  );
  await page.evaluate(() => window.__harness.clearInvocations());
  await threadTimeline.hover();
  await page.mouse.wheel(0, -5000);

  await expect
    .poll(() => invocationCount(page, "paginate_thread_timeline_backwards"))
    .toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("paginate_thread_timeline_backwards")[0]?.args)
    )
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$seed-event:example.invalid"
    });
  expect(await invocationCount(page, "paginate_timeline_backwards")).toBe(0);

  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key,
          direction: "Backward",
          state: "Paginating"
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key: threadKey });
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-testid="timeline-spinner"]')
  ).toBeVisible();

  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key,
          direction: "Backward",
          state: "EndReached"
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, { key: threadKey });
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-testid="timeline-start"]')
  ).toHaveCount(0);
  await expect(
    page.locator('aside[aria-label="Context panel"] [data-testid="timeline-spinner"]')
  ).toHaveCount(0);
});

test("room info Files entry opens the file browser with room scope", async ({ page }) => {
  await gotoReadyShell(page);
  await page.evaluate(() => window.__harness.clearInvocations());

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setCommandResponse("open_files_view", () => {
      const next = {
        ...snapshot,
        state: {
          ...snapshot.state,
          ui: {
            ...snapshot.state.ui,
            files_view: {
              kind: "open",
              request_id: 1,
            scope: { kind: "room", room_id: "!harness-room:example.invalid" },
            filter: { kinds: ["image", "video", "audio", "file"], filename_query: null },
            sort: "newestFirst",
            items: [
              {
                room_id: "!harness-room:example.invalid",
                event_id: "$file-event:example.invalid",
                sender: "@file-sender:example.invalid",
                timestamp_ms: 1_800_000_000_000,
                kind: "file",
                filename: "quarterly_report.pdf",
                mimetype: "application/pdf",
                size: 12_345,
                source_mxc: "mxc://example.invalid/source",
                thumbnail_mxc: null,
                thread_root: null,
                encrypted: false,
                encryption_version: null,
                width: null,
                height: null,
                is_edited: false
              }
            ],
            selected_event_id: null
          }
        }
      }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
  });

  await page.getByRole("button", { name: t("room.roomInfo") }).click();
  await page.getByRole("button", { name: t("room.files") }).click();

  await expect.poll(() => invocationCount(page, "open_files_view")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("open_files_view")[0]?.args))
    .toEqual({
      scope: { kind: "room", room_id: "!harness-room:example.invalid" },
      filter: { kinds: ["image", "video", "audio", "file"], filename_query: null },
      sort: "newestFirst"
    });

  await expect(page.getByText(t("files.title"), { exact: true })).toBeVisible();
  await expect(page.getByText("quarterly_report.pdf")).toBeVisible();
});

test("timeline header Threads button opens the threads list and row opens a thread", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate((roomId) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          thread_attention: {
            kind: "tracking",
            room_id: roomId,
            notification_count: 1,
            highlight_count: 0,
            live_event_marker_count: 0
          }
        }
      }
    });
    window.__harness.setCommandResponse(
      "open_threads_list",
      ({ scope }: { scope: { kind: string; room_id?: string } }) => {
        const current = window.__harness.currentSnapshot();
        const next = {
          ...current,
          state: {
            ...current.state,
            ui: {
              ...current.state.ui,
              threads_list: {
                kind: "open",
                room_id: scope.room_id ?? HARNESS_ROOM_ID,
                request_id: 1,
                items: [],
                is_paginating: false,
                end_reached: true
              }
            }
          }
        };
        window.__harness.setSnapshot(next);
        return next;
      }
    );
    window.__harness.pushStateUpdate();
    window.__harness.clearInvocations();
  }, HARNESS_ROOM_ID);

  await expect(
    page.locator(".channel-actions").getByRole("button", { name: t("threads.title") })
  ).toBeVisible();

  await page
    .locator(".channel-actions")
    .getByRole("button", { name: t("threads.title") })
    .click();

  await expect.poll(() => invocationCount(page, "open_threads_list")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("open_threads_list")[0]?.args)
    )
    .toEqual({
      scope: { kind: "room", room_id: "!harness-room:example.invalid" }
    });

  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          threads_list: {
          kind: "open",
          room_id: "!harness-room:example.invalid",
          request_id: 1,
          items: [
            {
              room_id: "!harness-room:example.invalid",
              root_event_id: "$thread-root:example.invalid",
              root_sender: "@thread-root-sender:example.invalid",
              root_sender_label: null,
              root_body_preview: "Thread root preview",
              root_timestamp_ms: 1_800_000_000_000,
              latest_event_id: "$thread-latest:example.invalid",
              latest_sender: "@thread-latest-sender:example.invalid",
              latest_sender_label: null,
              latest_body_preview: "Latest reply preview",
              latest_timestamp_ms: 1_800_000_000_100,
              reply_count: 3
            }
          ],
          is_paginating: false,
          end_reached: true
        }
      }
    }
    });
    window.__harness.pushStateUpdate();
  });

  const contextPanel = page.locator('aside[aria-label="Context panel"]');
  await expect(contextPanel.getByText(t("threads.title"), { exact: true })).toBeVisible();
  await expect(contextPanel.getByText("Thread root preview")).toBeVisible();

  await page.getByRole("button", { name: /3 replies/ }).click({ force: true });

  await expect.poll(() => invocationCount(page, "open_thread")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("open_thread")[0]?.args))
    .toEqual({
      roomId: "!harness-room:example.invalid",
      rootEventId: "$thread-root:example.invalid",
      intent: "existingThread"
    });
});

test("thread attention renders one Rust count in the root and header and clears on acknowledgement snapshot", async ({
  page
}) => {
  await gotoReadyShell(page);
  const rootEventId = "$attention-root:example.invalid";
  const rootItem = {
      id: { Event: { event_id: rootEventId } },
      sender: "@root-sender:example.invalid",
      body: "Thread root",
      timestamp_ms: 1_800_000_003_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: {
        reply_count: 4,
        latest_event_id: "$latest-reply:example.invalid",
        latest_sender: "@reply-sender:example.invalid",
        latest_sender_label: "Reply sender",
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_003_100
      },
      reactions: [],
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false
  };
  await page.evaluate(
    async ({ key, item }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key,
            generation: 2,
            items: [item]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: HARNESS_ROOM_KEY, item: rootItem }
  );
  await expect(page.getByText("Thread root", { exact: true })).toBeVisible();

  await page.evaluate(
    ({ roomId, rootEventId }) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            thread_attention: {
              kind: "tracking",
              room_id: roomId,
              root_event_id: rootEventId,
              notification_count: 2,
              highlight_count: 0,
              live_event_marker_count: 2
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    },
    { roomId: HARNESS_ROOM_ID, rootEventId }
  );

  const threadsButton = page
    .locator(".channel-actions")
    .getByRole("button", { name: t("threads.title") });
  await expect(threadsButton).toHaveAttribute("data-count", "2");
  await expect(page.getByRole("button", { name: /Thread notifications · 2/ })).toBeVisible();

  // A successful threaded read receipt is projected by Rust as the next
  // snapshot. React must not keep or repair either count locally.
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    if (snapshot.state.domain.thread_attention.kind !== "tracking") {
      throw new Error("thread attention fixture is not tracking");
    }
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          thread_attention: {
            ...snapshot.state.domain.thread_attention,
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0
          }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });

  await expect(page.getByRole("button", { name: /View new replies/ })).toHaveCount(0);
  // #330: the header button is the only entry point to this room's threads, so
  // acknowledgement clears its badge rather than removing the button.
  await expect(threadsButton).toBeVisible();
  await expect(threadsButton).not.toHaveAttribute("data-count", /.+/);
});

test("rail Home and sidebar Threads navigation buttons dispatch Rust-owned commands", async ({
  page
}) => {
  await gotoReadyShell(page);
  await page.evaluate(() => {
    window.__harness.setCommandResponse(
      "open_threads_list",
      () => window.__harness.currentSnapshot()
    );
    window.__harness.clearInvocations();
  });

  const sidebar = page.getByRole("complementary", { name: t("workspace.rooms") });
  await expect(sidebar.getByRole("button", { name: t("workspace.threads") })).toBeVisible();
  await sidebar.getByRole("button", { name: t("workspace.threads") }).click();
  await expect.poll(() => invocationCount(page, "open_threads_list")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("open_threads_list")[0]?.args)
    )
    .toEqual({ scope: { kind: "home" } });

  await page.evaluate(() => window.__harness.clearInvocations());
  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect.poll(() => invocationCount(page, "select_space")).toBeGreaterThanOrEqual(1);
  await expect
    .poll(async () => page.evaluate(() => window.__harness.invocationsOf("select_space")[0]?.args))
    .toEqual({ spaceId: null });
  await expect(page.getByRole("main", { name: t("workspace.activity") })).toBeVisible();

  await sidebar.getByRole("button", { name: t("workspace.threads") }).click();
  await expect
    .poll(async () =>
      page.evaluate(() => window.__harness.invocationsOf("open_threads_list").at(-1)?.args)
    )
    .toEqual({ scope: { kind: "home" } });

  // Explore is account-global, so it becomes reachable only at Home.
  await sidebar.getByRole("button", { name: t("workspace.explore"), exact: true }).click();
  await expect(page.getByRole("main", { name: t("workspace.explore") })).toBeVisible();
});
