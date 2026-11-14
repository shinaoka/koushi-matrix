/**
 * Fixtures for the count-based renderer budget spec (#1895).
 *
 * The rooms here exist to give the counters something to measure with the
 * shapes that used to cost: enough rows to sit on both sides of the timeline
 * virtualization threshold, receipt chips on most rows (the per-row popover
 * hook #1892 gated), reactions, media rows, a code block and a link.
 *
 * Everything is synthetic: `example.invalid` ids, generated bodies, no account
 * data. The item shape is the one the anchor-drift spec established; it is
 * copied rather than imported so a change to that spec cannot silently move
 * these budgets.
 */

import { expect, type Page } from "@playwright/test";

import { roomTimelineKey } from "../../src/domain/coreEvents";

const HARNESS_USER_ID = "@harness-user:example.invalid";
const HARNESS_SPACE_ID = "!harness-space:example.invalid";
const HARNESS_SEED_EVENT_ID = "$seed-event:example.invalid";

export type BudgetRoom = {
  readonly roomId: string;
  readonly name: string;
  readonly rowCount: number;
  readonly isDm: boolean;
};

/** Two rooms around the 120-row threshold, plus a small DM for switch cycles. */
export const BUDGET_ROOMS = {
  busy: {
    roomId: "!budget-busy-room:example.invalid",
    name: "Budget Busy Room",
    rowCount: 120,
    isDm: false
  },
  long: {
    roomId: "!budget-busy-long-room:example.invalid",
    name: "Budget Long Room",
    rowCount: 200,
    isDm: false
  },
  quiet: {
    roomId: "!budget-quiet-dm:example.invalid",
    name: "Budget Quiet DM",
    rowCount: 30,
    isDm: true
  }
} as const satisfies Record<string, BudgetRoom>;

/** A key no room ever displays: the burst scenario drops every event on it. */
export const UNOPENED_BURST_ROOM_ID = "!budget-unopened-burst:example.invalid";

type BudgetItem = {
  id: { Event: { event_id: string } };
  sender: string;
  sender_label: string;
  body: string;
  timestamp_ms: number;
  in_reply_to_event_id: null;
  thread_root: null;
  thread_summary: null;
  can_react: boolean;
  is_redacted: boolean;
  is_hidden: boolean;
  can_redact: boolean;
  is_edited: boolean;
  can_edit: boolean;
  reactions: Array<{
    key: string;
    count: number;
    reacted_by_me: boolean;
    my_reaction_event_id: null;
    sender_preview: Array<{ user_id: string; display_label: string }>;
  }>;
  send_state: null;
  media?: unknown;
  formatted?: unknown;
};

function eventId(room: BudgetRoom, index: number): string {
  return `$budget-${room.roomId.slice(1, 8)}-${String(index).padStart(4, "0")}:example.invalid`;
}

const IMAGE_ROWS = new Set([7, 43, 91]);
const CODE_ROW = 60;
const LINK_ROW = 88;

function makeBudgetItem(room: BudgetRoom, index: number): BudgetItem {
  const base: BudgetItem = {
    id: { Event: { event_id: eventId(room, index) } },
    sender: HARNESS_USER_ID,
    sender_label: "Harness User",
    body: `budget row ${index}\nSecond line so the row has a stable two-line height.`,
    timestamp_ms: 1_800_000_000_000 + index * 1000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: true,
    is_edited: false,
    can_edit: true,
    reactions: [],
    send_state: null
  };

  if (index % 5 === 2) {
    base.reactions = [
      {
        key: "👍",
        count: 2,
        reacted_by_me: false,
        my_reaction_event_id: null,
        sender_preview: [{ user_id: "@reactor-a:example.invalid", display_label: "Reactor A" }]
      },
      {
        key: "🎉",
        count: 1,
        reacted_by_me: false,
        my_reaction_event_id: null,
        sender_preview: [{ user_id: "@reactor-b:example.invalid", display_label: "Reactor B" }]
      }
    ];
  }

  if (IMAGE_ROWS.has(index)) {
    base.body = null as unknown as string;
    base.media = {
      kind: "Image",
      filename: `budget-image-${index}.jpg`,
      source: {
        mxc_uri: `mxc://example.invalid/budget-image-${index}`,
        encrypted: false,
        encryption_version: null
      },
      mimetype: "image/jpeg",
      size: 102400,
      width: 800,
      height: 600,
      thumbnail: null
    };
  }

  if (index === CODE_ROW) {
    base.formatted = {
      html: "<pre><code>fn budget() -&gt; usize { 1895 }</code></pre>",
      plain_text: "fn budget() -> usize { 1895 }",
      code_blocks: [{ language: "rust", body: "fn budget() -> usize { 1895 }" }]
    };
  }

  if (index === LINK_ROW) {
    base.body =
      "budget link row https://example.invalid/budget/issue/1895 with surrounding words so the row wraps once.";
  }

  return base;
}

function budgetRoomItems(room: BudgetRoom): BudgetItem[] {
  return Array.from({ length: room.rowCount }, (_, index) => makeBudgetItem(room, index));
}

/**
 * Receipts on every third row, three to five readers each: the chip is rendered
 * from the snapshot, so the closed popover must stay free (#1892).
 */
function budgetRoomReceipts(room: BudgetRoom): Record<string, unknown> {
  const receipts: Record<string, unknown> = {};
  for (let index = 0; index < room.rowCount; index += 3) {
    const readerCount = 3 + (index % 3);
    receipts[eventId(room, index)] = {
      readers: Array.from({ length: readerCount }, (_, reader) => ({
        user_id: `@budget-reader-${reader}:example.invalid`,
        display_name: `Budget Reader ${reader}`,
        original_display_label: `Budget Reader ${reader}`,
        avatar: null,
        timestamp_ms: 1_800_000_000_500 + reader
      })),
      total_count: readerCount,
      overflow_count: 0
    };
  }
  return receipts;
}

/** Room metadata the harness's own `select_room` handler expects. */
function budgetRoomSummary(room: BudgetRoom) {
  return {
    room_id: room.roomId,
    display_name: room.name,
    display_label: room.name,
    original_display_label: room.name,
    avatar: null,
    is_dm: room.isDm,
    dm_user_ids: room.isDm ? ["@budget-dm-peer:example.invalid"] : [],
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    notification_count: 0,
    highlight_count: 0,
    parent_space_ids: room.isDm ? [] : [HARNESS_SPACE_ID],
    dm_space_ids: [],
    is_encrypted: false,
    latest_event: null
  };
}

function budgetSidebarRoom(room: BudgetRoom) {
  return {
    room_id: room.roomId,
    display_name: room.name,
    avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    highlight_count: 0,
    notification_count: 0,
    display_count: 0,
    has_unread_content: false,
    is_attention_highlighted: false,
    has_unread_mention: false,
    is_muted: false
  };
}

/** Open the harness and wait for the boot seed row, which the boot loop owns. */
export async function gotoBudgetHarness(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  // The harness re-emits its boot timeline until a spec owns the stream, so the
  // fixture has to land after that seed is on screen.
  await expect(page.locator(`[data-event-id="${HARNESS_SEED_EVENT_ID}"]`)).toBeVisible();
}

/**
 * Add the budget rooms to the harness snapshot: room summaries, sidebar rows,
 * receipts, and a room list that names them. Nothing is displayed yet — the
 * caller opens a room with {@link openBudgetRoom}.
 */
export async function seedBudgetRooms(page: Page): Promise<void> {
  const rooms = Object.values(BUDGET_ROOMS);
  await page.evaluate(
    ({ summaries, sidebarRows, receiptsByRoom, dmRoomIds }) => {
      const harness = window.__harness;
      const base = harness.currentSnapshot();
      const roomDom = base.state.domain;
      const sidebar = base.sidebar;
      const fixtureRoomIds = new Set<string>(summaries.map((room: { room_id: string }) => room.room_id));
      const withoutFixtures = <T extends { room_id: string }>(rows: T[]): T[] =>
        rows.filter((row) => !fixtureRoomIds.has(row.room_id));
      const nextRooms = [...withoutFixtures(roomDom.rooms), ...summaries];
      const nextSidebarRooms = [
        ...withoutFixtures(sidebar.sections.rooms),
        ...sidebarRows.filter((row: { room_id: string }) => !dmRoomIds.includes(row.room_id))
      ];
      const nextPeople = [
        ...withoutFixtures(sidebar.sections.people),
        ...sidebarRows.filter((row: { room_id: string }) => dmRoomIds.includes(row.room_id))
      ];
      const roomListItems = [
        ...withoutFixtures(base.state.ui.room_list.items),
        ...summaries.map((room: { room_id: string }) => ({
          room_id: room.room_id,
          kind: "room" as const
        }))
      ];
      const liveSignalsRooms: Record<string, unknown> = {
        ...base.state.domain.live_signals.rooms
      };
      for (const [roomId, receipts] of Object.entries(receiptsByRoom)) {
        liveSignalsRooms[roomId] = {
          receipts_by_event: receipts,
          focused_receipts_by_event: {},
          thread_receipts_by_event: {},
          fully_read_event_id: null,
          typing_user_ids: [],
          typing_users: []
        };
      }

      harness.setSnapshot({
        ...base,
        state: {
          ...base.state,
          domain: {
            ...roomDom,
            rooms: nextRooms,
            live_signals: { ...base.state.domain.live_signals, rooms: liveSignalsRooms }
          },
          ui: {
            ...base.state.ui,
            room_list: { ...base.state.ui.room_list, items: roomListItems }
          }
        },
        sidebar: {
          ...sidebar,
          space_rooms: [
            ...withoutFixtures(sidebar.space_rooms),
            ...sidebarRows.filter((row: { room_id: string }) => !dmRoomIds.includes(row.room_id))
          ],
          global_dms: [...withoutFixtures(sidebar.global_dms), ...nextPeople],
          sections: {
            ...sidebar.sections,
            rooms: nextSidebarRooms,
            people: nextPeople
          }
        }
      });
      harness.pushStateUpdate();
    },
    {
      summaries: rooms.map(budgetRoomSummary),
      sidebarRows: rooms.map(budgetSidebarRoom),
      receiptsByRoom: Object.fromEntries(
        rooms.map((room) => [room.roomId, budgetRoomReceipts(room)])
      ),
      dmRoomIds: rooms.filter((room) => room.isDm).map((room) => room.roomId)
    }
  );
  // The sidebar renders the new rows once the pushed snapshot reaches the store.
  for (const room of rooms) {
    await expect(page.getByRole("button", { name: room.name }).first()).toBeVisible();
  }
}

const pushedGenerations = new WeakMap<Page, Map<string, number>>();

/**
 * Push one `InitialItems` batch for a room, the way Core resubscribes after a
 * room is selected. Each key's generation increases so the store applies it.
 */
async function pushBudgetItems(
  page: Page,
  room: BudgetRoom,
  items: readonly unknown[] = budgetRoomItems(room)
): Promise<void> {
  const generations = pushedGenerations.get(page) ?? new Map<string, number>();
  pushedGenerations.set(page, generations);
  const generation = (generations.get(room.roomId) ?? 1) + 1;
  generations.set(room.roomId, generation);
  await page.evaluate(
    async ({ key, nextItems, nextGeneration }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key,
            generation: nextGeneration,
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            items: nextItems as any[]
          }
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
        } as any
      } as any);
    },
    { key: roomTimelineKey(HARNESS_USER_ID, room.roomId), nextItems: items, nextGeneration: generation }
  );
}

/**
 * Select a room from the sidebar and wait for its rows.
 *
 * The click goes through `el.click()`, never Playwright's actionability
 * checks: those move the pointer and re-measure, which would land inside the
 * measured bracket.
 */
export async function openBudgetRoom(page: Page, room: BudgetRoom): Promise<void> {
  await page.getByRole("button", { name: room.name, exact: true }).first().evaluate(
    (element: HTMLElement) => element.click()
  );
  await pushBudgetItems(page, room);
  // The row count is what proves this room's batch landed; which rows are
  // mounted is exactly what the virtualization budget is about.
  await expect(page.locator("[data-testid=timeline-view]")).toHaveAttribute(
    "data-total-items",
    String(room.rowCount)
  );
}
