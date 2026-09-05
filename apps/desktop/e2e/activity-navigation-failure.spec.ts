import { expect, test, type Page } from "@playwright/test";

import { t } from "../src/i18n/messages";
import { gotoReadyShell, invocationCount } from "./support/basicOperations";

const ROOM_A = "!activity-a:example.invalid";
const ROOM_B = "!activity-b:example.invalid";
const EVENT_A = "$activity-a:example.invalid";
const EVENT_B = "$activity-b:example.invalid";
const THREAD_ROOT_B = "$thread-root-b:example.invalid";

type ActivityTab = "recent" | "unread";
type EventNavigationKind = "idle" | "opening" | "anchored" | "liveFallback" | "failed";

type NavigationFixture = {
  roomId: string;
  eventId: string;
  source: "activity" | "search" | "pinned";
  kind: EventNavigationKind;
  failureKind?: "targetMissing" | "roomUnavailable" | "sessionUnavailable" | "timeline";
};

function activityRow(
  roomId: string,
  eventId: string,
  roomLabel: string,
  threadRootEventId: string | null = null
) {
  return {
    kind: "event",
    room_id: roomId,
    event_id: eventId,
    thread_root_event_id: threadRootEventId,
    room_label: roomLabel,
    context_label: `Room · ${roomLabel}`,
    sender_label: `${roomLabel} sender`,
    sender_avatar: null,
    preview: `${roomLabel} activity`,
    timestamp_ms: 1_800_000_000_000,
    unread: false,
    highlight: false
  };
}

function activitySnapshot(snapshot: any, activeTab: ActivityTab, rows: any[]) {
  const baseRoom = snapshot.state.domain.rooms[0];
  const rooms = [
    ...snapshot.state.domain.rooms.filter(
      (room: { room_id: string }) => room.room_id !== ROOM_A && room.room_id !== ROOM_B
    ),
    {
      ...baseRoom,
      room_id: ROOM_A,
      display_name: "Activity A",
      display_label: "Activity A",
      original_display_label: "Activity A",
      parent_space_ids: [],
      latest_event: { ...baseRoom.latest_event, event_id: EVENT_A }
    },
    {
      ...baseRoom,
      room_id: ROOM_B,
      display_name: "Activity B",
      display_label: "Activity B",
      original_display_label: "Activity B",
      parent_space_ids: [],
      latest_event: { ...baseRoom.latest_event, event_id: EVENT_B }
    }
  ];
  const baseSidebarRoom = snapshot.sidebar.space_rooms[0];
  const sidebarRooms = [
    ...(baseSidebarRoom ? [baseSidebarRoom] : []),
    { ...baseSidebarRoom, room_id: ROOM_A, display_name: "Activity A" },
    { ...baseSidebarRoom, room_id: ROOM_B, display_name: "Activity B" }
  ].filter(Boolean);
  return {
    ...snapshot,
    state: {
      ...snapshot.state,
      domain: {
        ...snapshot.state.domain,
        rooms,
        activity: {
          kind: "open",
          active_tab: activeTab,
          recent: {
            rows,
            next_batch: null,
            resolution: { kind: "idle" }
          },
          unread: {
            rows,
            next_batch: null,
            resolution: { kind: "idle" }
          },
          mark_read: { kind: "idle" }
        }
      }
    },
    sidebar: {
      ...snapshot.sidebar,
      space_rooms: sidebarRooms,
      sections: {
        ...snapshot.sidebar.sections,
        rooms: sidebarRooms
      }
    }
  };
}

async function openActivityWithRows(
  page: Page,
  tab: ActivityTab,
  rows: any[]
): Promise<void> {
  await gotoReadyShell(page);
  const snapshot = await page.evaluate(() => window.__harness.currentSnapshot());
  const next = activitySnapshot(snapshot, tab, rows);
  await page.evaluate((activity) => {
    window.__harness.setCommandResponse("open_activity", () => {
      window.__harness.setSnapshot(activity);
      return activity;
    });
    window.__harness.clearInvocations();
  }, next);

  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
  await expect(page.getByRole("button", { name: /Open activity item/ }).first()).toBeVisible();
}

async function publishNavigation(page: Page, fixture: NavigationFixture) {
  await page.evaluate((nextNavigation) => {
    const current = window.__harness.currentSnapshot();
    const eventNavigation =
      nextNavigation.kind === "failed"
        ? {
            kind: "failed" as const,
            generation: 2,
            source: nextNavigation.source,
            failureKind: nextNavigation.failureKind ?? "timeline"
          }
        : nextNavigation.kind === "idle"
          ? { kind: "idle" as const }
          : {
              kind: nextNavigation.kind,
              generation: nextNavigation.kind === "opening" ? 1 : 2,
              source: nextNavigation.source
            };
    window.__harness.setSnapshot({
      ...current,
      state: {
        ...current.state,
        ui: {
          ...current.state.ui,
          navigation: {
            ...current.state.ui.navigation,
            active_room_id: nextNavigation.roomId,
            main_timeline_anchor:
              nextNavigation.kind === "anchored" ? { event_id: nextNavigation.eventId } : null,
            event_navigation: eventNavigation
          },
          timeline: {
            ...current.state.ui.timeline,
            room_id: nextNavigation.roomId,
            is_subscribed: true
          },
          thread: { kind: "closed" },
          focused_context: { kind: "closed" }
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, fixture);
}

async function settleDeferred(
  page: Page,
  index: number,
  outcome: "resolve" | "reject"
): Promise<void> {
  await page.evaluate(
    ({ index: deferredOutcome }) => {
      if (deferredOutcome.outcome === "resolve") {
        const generation = window.__harness.currentSnapshot().state_generation ?? 0;
        window.__harness.resolveDeferredCommand("open_activity_event", deferredOutcome.index, {
          protocolVersion: 1,
          publishedGeneration: generation
        });
      } else {
        window.__harness.rejectDeferredCommand("open_activity_event", deferredOutcome.index);
      }
    },
    { index, outcome }
  );
}

async function assertTarget(page: Page, fixture: NavigationFixture) {
  await expect
    .poll(() =>
      page.evaluate(({ roomId, eventId, source, kind }) => {
        const navigation = window.__harness.currentSnapshot().state.ui.navigation;
        return {
          roomId: navigation.active_room_id,
          timelineRoomId: window.__harness.currentSnapshot().state.ui.timeline.room_id,
          eventId: navigation.main_timeline_anchor?.event_id ?? null,
          source: navigation.event_navigation.source,
          kind: navigation.event_navigation.kind,
          expected: { roomId, eventId, source, kind }
        };
      }, fixture)
    )
    .toMatchObject({
      roomId: fixture.roomId,
      timelineRoomId: fixture.roomId,
      eventId: fixture.kind === "anchored" ? fixture.eventId : null,
      source: fixture.source,
      kind: fixture.kind
    });
}

for (const [tab, terminalKind] of [
  ["recent", "anchored"],
  ["unread", "liveFallback"]
] as const) {
  test(`Rust event navigation wins a rapid ${tab} A→B race`, async ({ page }) => {
    const rows = [
      activityRow(ROOM_A, EVENT_A, "Activity A"),
      activityRow(ROOM_B, EVENT_B, "Activity B")
    ];
    await openActivityWithRows(
      page,
      tab,
      tab === "unread" ? rows.map((row) => ({ ...row, unread: true })) : rows
    );
    await page.evaluate(() => window.__harness.deferCommand("open_activity_event"));

    await page.getByRole("button", { name: "Open activity item Activity A" }).click();
    await page.getByRole("button", { name: "Open activity item Activity B" }).click();
    await expect.poll(() => invocationCount(page, "open_activity_event")).toBe(2);

    const bNavigation: NavigationFixture = {
      roomId: ROOM_B,
      eventId: EVENT_B,
      source: "activity",
      kind: terminalKind
    };
    await publishNavigation(page, {
      roomId: ROOM_A,
      eventId: EVENT_A,
      source: "activity",
      kind: "opening"
    });
    await publishNavigation(page, bNavigation);
    await assertTarget(page, bNavigation);
    await settleDeferred(page, 1, "resolve");
    await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

    // A completes only after B has settled. Its Promise result must not repair
    // the target, the pane, or the current Rust-owned failure projection.
    await settleDeferred(page, 0, "reject");
    await expect(page.getByRole("alert")).toHaveCount(0);
    await assertTarget(page, bNavigation);
    await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  });
}

test("current Activity navigation failure is rendered from the Rust DTO", async ({ page }) => {
  await openActivityWithRows(page, "recent", [activityRow(ROOM_A, EVENT_A, "Activity A")]);
  await page.evaluate(() => window.__harness.deferCommand("open_activity_event"));
  await page.getByRole("button", { name: "Open activity item Activity A" }).click();
  await expect.poll(() => invocationCount(page, "open_activity_event")).toBe(1);

  await publishNavigation(page, {
    roomId: ROOM_A,
    eventId: EVENT_A,
    source: "activity",
    kind: "opening"
  });
  const failure: NavigationFixture = {
    roomId: ROOM_A,
    eventId: EVENT_A,
    source: "activity",
    kind: "failed",
    failureKind: "timeline"
  };
  await publishNavigation(page, failure);
  await expect(page.getByRole("alert")).toContainText(t("navigation.failed"));
  await settleDeferred(page, 0, "reject");
  await expect(page.getByRole("alert")).toContainText(t("navigation.failed"));
  await assertTarget(page, failure);
  await expect(page.getByRole("main", { name: "Activity" })).toBeVisible();
});

test("a late Activity event completion cannot overwrite an ordinary room selection", async ({
  page
}) => {
  await openActivityWithRows(page, "recent", [activityRow(ROOM_A, EVENT_A, "Activity A")]);
  await page.evaluate(() => window.__harness.deferCommand("open_activity_event"));
  await page.getByRole("button", { name: "Open activity item Activity A" }).click();
  await expect.poll(() => invocationCount(page, "open_activity_event")).toBe(1);

  await publishNavigation(page, {
    roomId: ROOM_A,
    eventId: EVENT_A,
    source: "activity",
    kind: "opening"
  });
  await page.getByRole("button", { name: "Activity B", exact: true }).click();
  await expect.poll(() => invocationCount(page, "select_room")).toBeGreaterThanOrEqual(1);
  await publishNavigation(page, {
    roomId: ROOM_B,
    eventId: EVENT_B,
    source: "activity",
    kind: "idle"
  });
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => window.__harness.currentSnapshot().state.ui.navigation.active_room_id))
    .toBe(ROOM_B);

  await settleDeferred(page, 0, "reject");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => window.__harness.currentSnapshot().state.ui.navigation.active_room_id))
    .toBe(ROOM_B);
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
});

test("a rejected thread navigation does not become an event-navigation failure", async ({ page }) => {
  await openActivityWithRows(page, "recent", [
    activityRow(ROOM_A, EVENT_A, "Activity A"),
    activityRow(ROOM_B, EVENT_B, "Activity B thread", THREAD_ROOT_B)
  ]);
  await page.evaluate(() => {
    window.__harness.deferCommand("open_activity_event");
    window.__harness.setCommandResponse("open_thread", () => {
      throw new Error("synthetic thread rejection");
    });
  });

  await page.getByRole("button", { name: "Open activity item Activity A" }).click();
  await expect.poll(() => invocationCount(page, "open_activity_event")).toBe(1);
  await publishNavigation(page, {
    roomId: ROOM_A,
    eventId: EVENT_A,
    source: "activity",
    kind: "opening"
  });
  await page.getByRole("button", { name: "Open activity item Activity B thread" }).click();
  await expect.poll(() => invocationCount(page, "select_room")).toBeGreaterThanOrEqual(1);
  await publishNavigation(page, {
    roomId: ROOM_B,
    eventId: EVENT_B,
    source: "activity",
    kind: "idle"
  });
  await expect.poll(() => invocationCount(page, "open_thread")).toBe(1);
  await settleDeferred(page, 0, "reject");

  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByText(t("panel.thread"), { exact: true })).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => window.__harness.currentSnapshot().state.ui.navigation.active_room_id))
    .toBe(ROOM_B);
});

test("Search source controls the panel after a delayed Activity navigation", async ({ page }) => {
  await openActivityWithRows(page, "recent", [activityRow(ROOM_A, EVENT_A, "Activity A")]);
  await page.evaluate(() => {
    window.__harness.deferCommand("open_activity_event");
    window.__harness.setCommandResponse("submit_search", ({ query }: { query: string }) => {
      const current = window.__harness.currentSnapshot();
      const next = {
        ...current,
        state: {
          ...current.state,
          domain: {
            ...current.state.domain,
            search: {
              kind: "results",
              request_id: 1,
              query,
              scope: "currentRoom",
              results: [
                {
                  room_id: "!activity-b:example.invalid",
                  event_id: "$activity-b:example.invalid",
                  sender: "@search-sender:example.invalid",
                  timestamp_ms: 1_800_000_000_100,
                  score_millis: 999,
                  snippet: "Search B result",
                  match_field: "messageBody",
                  highlights: [],
                  match_kind: "exact"
                }
              ]
            }
          }
        }
      };
      window.__harness.setSnapshot(next);
      return next;
    });
  });

  await page.getByRole("button", { name: "Open activity item Activity A" }).click();
  await expect.poll(() => invocationCount(page, "open_activity_event")).toBe(1);

  const searchInput = page.getByRole("textbox", { name: "Search" });
  await searchInput.fill("Search B");
  await searchInput.press("Enter");
  await page.getByRole("button", { name: /Search B result/ }).click();

  await expect(page.getByText(t("panel.search"), { exact: true })).toBeVisible();
  const searchNavigation: NavigationFixture = {
    roomId: ROOM_B,
    eventId: EVENT_B,
    source: "search",
    kind: "anchored"
  };
  await assertTarget(page, searchNavigation);

  await settleDeferred(page, 0, "reject");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText(t("panel.search"), { exact: true })).toBeVisible();
  await assertTarget(page, searchNavigation);
});
