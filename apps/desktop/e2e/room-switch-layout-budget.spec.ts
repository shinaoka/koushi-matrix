import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { gotoReadyShell, seedTimelineItems } from "./support/basicOperations";
import {
  installLayoutReadProbe,
  layoutReads,
  layoutBudgetItem,
  resetLayoutReads,
  seedSwitchableRoom,
  switchableRoomKey,
  type LayoutReadCounts
} from "./support/roomSwitch";

/**
 * #1889 gate: a room switch must cost reads bounded by the viewport and the
 * rail size, not by the number of timeline rows.
 *
 * The assertion is a fixed ceiling on reads per switch with roughly 2x headroom
 * over the measured baseline, not a comparison between two item counts: the
 * number of observation passes a switch makes is not perfectly repeatable, so a
 * difference-based budget measured frames rather than scaling. Nothing here
 * asserts milliseconds.
 * `.timeline-item-frame` reads belong to the virtual height model (#1890), so
 * they are counted separately and never asserted.
 */

const SMALL_ROOM_ID = "!budget-small:example.invalid";
const SMALL_ROOM_NAME = "Budget Small";
const LARGE_ROOM_ID = "!budget-large:example.invalid";
const LARGE_ROOM_NAME = "Budget Large";
const SMALL_ITEMS = 30;
const LARGE_ITEMS = 100;
/** Room-switch reads may differ by at most this much between the two rooms. */
const ROW_READ_GROWTH_BUDGET = 8;

function items(prefix: string, count: number): unknown[] {
  return Array.from({ length: count }, (_, index) =>
    layoutBudgetItem(`$${prefix}-${index}:example.invalid`, index)
  );
}

const SMALL_PREFIX = "budget-small";
const LARGE_PREFIX = "budget-large";
const SMALL_FIRST = `$${SMALL_PREFIX}-0:example.invalid`;

/** Waits out the switch, then the two frames the observation that follows it needs. */
async function settleSwitch(page: Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      )
  );
}

async function clickRoom(page: Page, roomName: string): Promise<void> {
  await resetLayoutReads(page);
  await page.locator(`.room-item[aria-label="${roomName}"]`).click();
  await expect(page.locator(`.room-item.is-active[aria-label="${roomName}"]`)).toHaveCount(1);
}

/**
 * Switches into `roomName` and, once it is active, delivers its timeline the
 * way Rust does after a subscription: an `InitialItems` batch for that key.
 */
async function switchIntoSeededRoom(
  page: Page,
  roomName: string,
  roomId: string,
  prefix: string,
  count: number
): Promise<LayoutReadCounts> {
  await clickRoom(page, roomName);
  await seedTimelineItems(page, items(prefix, count), 2, switchableRoomKey(roomId));
  await expect(page.locator(`[data-item-id="$${prefix}-0:example.invalid"]`)).toHaveCount(1);
  await settleSwitch(page);
  return layoutReads(page);
}

async function showHomeSections(page: Page): Promise<void> {
  await page
    .getByRole("navigation", { name: t("workspace.workspaces") })
    .getByRole("button", { name: /^Home/ })
    .click();
  await expect(page.locator(`.room-item[aria-label="${SMALL_ROOM_NAME}"]`)).toHaveCount(1);
}

test.beforeEach(async ({ page }) => {
  await installLayoutReadProbe(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await gotoReadyShell(page);
  await seedSwitchableRoom(page, SMALL_ROOM_ID, SMALL_ROOM_NAME);
  await seedSwitchableRoom(page, LARGE_ROOM_ID, LARGE_ROOM_NAME);
});

test("switching into a busy room reads rows per viewport, not per item", async ({ page }) => {
  await showHomeSections(page);

  const small = await switchIntoSeededRoom(
    page,
    SMALL_ROOM_NAME,
    SMALL_ROOM_ID,
    SMALL_PREFIX,
    SMALL_ITEMS
  );
  const large = await switchIntoSeededRoom(
    page,
    LARGE_ROOM_NAME,
    LARGE_ROOM_ID,
    LARGE_PREFIX,
    LARGE_ITEMS
  );

  expect(large.row).toBeGreaterThan(0);
  expect(large.row - small.row).toBeLessThanOrEqual(ROW_READ_GROWTH_BUDGET);
});

test("switching rooms does not re-measure the Space rail per snapshot", async ({ page }) => {
  await page.evaluate(() => {
    const snapshot = structuredClone(window.__harness.currentSnapshot());
    const template = snapshot.sidebar.space_rail[0];
    if (template) {
      snapshot.sidebar.space_rail.push({
        ...template,
        space_id: "!budget-extra-space:example.invalid",
        display_name: "Budget Space"
      });
    }
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateUpdate();
  });
  await showHomeSections(page);

  const railSize = await page.locator("[data-space-id]").count();
  expect(railSize).toBeGreaterThanOrEqual(2);

  const first = await switchIntoSeededRoom(
    page,
    SMALL_ROOM_NAME,
    SMALL_ROOM_ID,
    SMALL_PREFIX,
    SMALL_ITEMS
  );
  const second = await switchIntoSeededRoom(
    page,
    LARGE_ROOM_NAME,
    LARGE_ROOM_ID,
    LARGE_PREFIX,
    LARGE_ITEMS
  );

  // A switch moves no rail geometry, so at most the active item's
  // scroll-into-view may measure it, once per switch.
  expect(first.rail).toBeLessThanOrEqual(2 * railSize);
  expect(second.rail).toBeLessThanOrEqual(2 * railSize);
});

test("a closed receipt popover does not measure during a room switch", async ({ page }) => {
  await page.evaluate(({ roomId, firstEventId }) => {
    const snapshot = structuredClone(window.__harness.currentSnapshot());
    snapshot.state.domain.live_signals.rooms[roomId] = {
      fully_read_event_id: null,
      typing_user_ids: [],
      typing_users: [],
      receipts_by_event: {
        [firstEventId]: {
          total_count: 10,
          overflow_count: 0,
          readers: Array.from({ length: 10 }, (_, index) => ({
            user_id: `@budget-reader${index}:example.invalid`,
            display_name: "Budget Reader",
            original_display_label: "Budget Reader",
            avatar: null,
            timestamp_ms: 1_800_000_000_000
          }))
        }
      }
    };
    window.__harness.setSnapshot(snapshot);
    window.__harness.pushStateUpdate();
  }, { roomId: SMALL_ROOM_ID, firstEventId: SMALL_FIRST });
  await showHomeSections(page);

  const counts = await switchIntoSeededRoom(
    page,
    SMALL_ROOM_NAME,
    SMALL_ROOM_ID,
    SMALL_PREFIX,
    SMALL_ITEMS
  );
  await expect(page.locator(`[data-item-id="${SMALL_FIRST}"] .message-receipts`)).toHaveCount(1);
  // #1894: a closed popover stays off the measurement path (#1892).
  expect(counts.receipt).toBe(0);
});
