import { expect, type Page } from "@playwright/test";

import { roomTimelineKey, type TimelineKey } from "../../src/domain/coreEvents";
import { HARNESS_ACCOUNT_KEY } from "./basicOperations";

/**
 * #1889: two switchable rooms plus the layout-budget probe for
 * `e2e/room-switch-layout-budget.spec.ts`.
 *
 * The probe counts `getBoundingClientRect` calls by the element they are made
 * on, because a room switch is dominated by how many *forced layouts* it
 * triggers, and a read loop over every mounted row is what makes that scale
 * with the item count instead of the viewport.
 */

export interface LayoutReadCounts {
  /** Rows the timeline reads by identity (`[data-item-id]` and friends). */
  row: number;
  /** `.timeline-item-frame` reads from the virtual height model (#1890). */
  frame: number;
  /** Space-rail buttons and the rail list. */
  rail: number;
  /** Receipt chrome; closed popovers must not measure (#1892). */
  receipt: number;
  total: number;
}

declare global {
  interface Window {
    __layoutReads: LayoutReadCounts;
    __resetLayoutReads: () => void;
  }
}

/** Installs the counter before any application script runs. */
export async function installLayoutReadProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const counts = { row: 0, frame: 0, rail: 0, receipt: 0, total: 0 };
    window.__layoutReads = counts;
    window.__resetLayoutReads = () => {
      counts.row = 0;
      counts.frame = 0;
      counts.rail = 0;
      counts.receipt = 0;
      counts.total = 0;
    };
    const original = Element.prototype.getBoundingClientRect;
    Element.prototype.getBoundingClientRect = function (this: Element) {
      counts.total += 1;
      if (this.classList.contains("timeline-item-frame")) {
        counts.frame += 1;
      } else if (
        this.matches("[data-item-id], [data-activity-event-id], [data-content-event-id]")
      ) {
        counts.row += 1;
      }
      if (this.closest("[data-space-id]") !== null) {
        counts.rail += 1;
      }
      if (this.closest(".message-receipts, .receipt-tooltip") !== null) {
        counts.receipt += 1;
      }
      return original.call(this);
    };
  });
}

export async function resetLayoutReads(page: Page): Promise<void> {
  await page.evaluate(() => window.__resetLayoutReads());
}

export async function layoutReads(page: Page): Promise<LayoutReadCounts> {
  return page.evaluate(() => ({ ...window.__layoutReads }));
}

/**
 * Adds a room the shell can switch to: a full `domain.rooms` summary (so
 * `select_room` resolves it) plus its sidebar row.
 */
export async function seedSwitchableRoom(
  page: Page,
  roomId: string,
  displayName: string
): Promise<void> {
  await page.evaluate(
    ({ roomId, displayName }) => {
      const snapshot = structuredClone(window.__harness.currentSnapshot());
      const room = snapshot.state.domain.rooms[0];
      if (!room) {
        throw new Error("harness snapshot has no room to clone");
      }
      if (!snapshot.state.domain.rooms.some((candidate) => candidate.room_id === roomId)) {
        snapshot.state.domain.rooms.push({
          ...room,
          room_id: roomId,
          display_name: displayName,
          display_label: displayName,
          original_display_label: displayName,
          parent_space_ids: [],
          latest_event: null,
          unread_count: 0,
          notification_count: 0,
          highlight_count: 0
        });
      }
      const sidebarRow = snapshot.sidebar.sections.rooms[0] ?? snapshot.sidebar.space_rooms[0];
      if (!snapshot.sidebar.sections.rooms.some((row) => row.room_id === roomId)) {
        snapshot.sidebar.sections.rooms.push({
          ...sidebarRow,
          room_id: roomId,
          display_name: displayName,
          unread_count: 0,
          highlight_count: 0,
          notification_count: 0,
          display_count: 0
        });
      }
      window.__harness.setSnapshot(snapshot);
      window.__harness.pushStateUpdate();
    },
    { roomId, displayName }
  );
  await expect(page.locator(`.room-item[aria-label="${displayName}"]`)).toHaveCount(1);
}

export function switchableRoomKey(roomId: string): TimelineKey {
  return roomTimelineKey(HARNESS_ACCOUNT_KEY, roomId);
}

/** One directly rendered timeline item; the lifecycle fields are required. */
export function layoutBudgetItem(eventId: string, index: number): unknown {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@budget-sender:example.invalid",
    sender_label: "Budget Sender",
    body: `Budget row ${index}`,
    timestamp_ms: 1_800_000_000_000 + index,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    media: null,
    reactions: [],
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false
  };
}
