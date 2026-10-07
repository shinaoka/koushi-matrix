import { expect, test, type Page } from "@playwright/test";
import type { DesktopSnapshot, RoomJoinRule, RoomListItem } from "../src/domain/types";
import { t } from "../src/i18n/messages";
import { HARNESS_ROOM_ID } from "./support/basicOperations";
import { pushSnapshot } from "./support/stateUpdates";

// #1166: the room list and the room header show each object's own access
// condition, with a tooltip on the icon and on each badge, and an accessible
// name that carries the condition.
interface Harness {
  currentSnapshot(): DesktopSnapshot;
  setCommandResponse(command: string, response: unknown): void;
}

function row(room_id: string, display_name: string, rule?: RoomJoinRule): RoomListItem {
  return {
    room_id,
    display_name,
    avatar: null,
    tags: { favourite: null, low_priority: null },
    unread_count: 0,
    notification_count: 0,
    display_count: 0,
    highlight_count: 0,
    has_unread_content: false,
    is_attention_highlighted: false,
    has_unread_mention: false,
    is_muted: false,
    ...(rule ? { access_join_rule: rule } : {})
  };
}

const PLAIN_ROOM = row("!plain:example.invalid", "Plain Room");
const PUBLIC_ROOM = row("!open:example.invalid", "Open Room", "public");
const INVITE_ROOM = row("!invite:example.invalid", "Invite Room", "invite");
const RESTRICTED_ROOM = row("!conditional:example.invalid", "Conditional Room", "restricted");
const KNOCK_ROOM = row("!request:example.invalid", "Request Room", "knock");
const BOTH_ROOM = row("!both:example.invalid", "Both Room", "knockRestricted");
const UNKNOWN_ROOM = row("!unknown:example.invalid", "Unknown Room", "unknown");

const ROOMS = [
  PLAIN_ROOM,
  PUBLIC_ROOM,
  INVITE_ROOM,
  RESTRICTED_ROOM,
  KNOCK_ROOM,
  BOTH_ROOM,
  UNKNOWN_ROOM
];

/** The harness already holds this room in `state.domain.rooms`, so it is the
 * active room the header renders. */
const HARNESS_ROOM = row(HARNESS_ROOM_ID, "Harness Room", "knockRestricted");

async function openHarness(page: Page): Promise<DesktopSnapshot> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("complementary", { name: t("workspace.rooms") })).toBeVisible();
  return page.evaluate(() => {
    const harness = (window as unknown as { __harness: Harness }).__harness;
    harness.setCommandResponse("close_search", { protocolVersion: 1, publishedGeneration: 0 });
    return harness.currentSnapshot();
  });
}

async function pushRoomList(page: Page): Promise<void> {
  const base = await openHarness(page);
  const rows = [...ROOMS, HARNESS_ROOM];
  await pushSnapshot(page, {
    ...base,
    sidebar: {
      ...base.sidebar,
      active_space_id: null,
      account_home: { ...base.sidebar.account_home, is_active: true },
      space_rail: base.sidebar.space_rail.map((space) => ({ ...space, is_active: false })),
      space_rooms: rows,
      global_dms: [],
      sections: {
        favourites: [],
        not_joined: [],
        rooms: rows,
        people: [],
        low_priority: []
      }
    }
  });
}

/** Re-push the room list with a new access rule on the active harness room. */
async function pushRoomRule(page: Page, rule?: RoomJoinRule): Promise<void> {
  const base = await openHarness(page);
  const rows = [...ROOMS, row(HARNESS_ROOM_ID, "Harness Room", rule)];
  await pushSnapshot(page, {
    ...base,
    sidebar: {
      ...base.sidebar,
      space_rooms: rows,
      global_dms: [],
      sections: {
        favourites: [],
        not_joined: [],
        rooms: rows,
        people: [],
        low_priority: []
      }
    }
  });
}

test("room rows render the projected access condition with its tooltip", async ({ page }) => {
  await pushRoomList(page);
  const rooms = page.getByRole("region", { name: t("roomList.categoryRooms"), exact: true });

  // Public and invite-only use their own icon, and no badge.
  await expect(rooms.locator('[data-room-access="globe"]')).toHaveCount(1);
  await expect(rooms.locator('[data-room-access="padlock"]')).toHaveCount(1);
  await expect(
    rooms.locator('[data-room-access="globe"]').locator("xpath=ancestor::*[@data-testid='room-item']")
  ).toContainText("Open Room");
  await expect(rooms.locator('[data-testid="room-item"]', { hasText: "Open Room" }).locator(".room-access-badge")).toHaveCount(0);

  // The other rules use compact badges, in the documented order.
  const bothRow = rooms.getByRole("button", { name: new RegExp("Both Room") });
  await expect(bothRow.locator(".room-access-badge")).toHaveText([
    t("access.conditionsApply"),
    t("access.canRequest")
  ]);

  // A row whose condition has not been projected says so rather than guessing.
  const plainRow = rooms.getByRole("button", { name: new RegExp("Plain Room") });
  await expect(plainRow.locator(".room-access-badge")).toHaveText([t("access.checking")]);
  await expect(plainRow.locator("[data-room-access]")).toHaveCount(0);

  // The condition is the row's description, so the name stays the room label
  // while screen readers still hear the condition.
  const inviteRow = rooms.getByRole("button", { name: "Invite Room", exact: true });
  await expect(inviteRow).toBeVisible();
  const describedBy = await inviteRow.getAttribute("aria-describedby");
  expect(describedBy).toBeTruthy();
  await expect(page.locator(`[id="${describedBy}"]`)).toHaveText(
    t("access.inviteOnlyDescription")
  );

  // A bubble anchored to the first row is clamped inside the viewport instead of
  // being pushed above its top edge.
  await plainRow.locator(".room-access-badge").first().hover();
  const topBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("access.checkingDescription") });
  await expect(topBubble).toHaveCount(1);
  await expect(topBubble).toBeInViewport({ ratio: 1 });
  expect((await topBubble.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(0);

  // Hovering the icon or a badge explains it.
  // The bubble renders in the body-level floating layer, so it is not clipped by
  // the sidebar's scrollport.
  await rooms.locator('[data-room-access="padlock"]').hover();
  const iconBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("access.inviteOnlyDescription") });
  await expect(iconBubble).toHaveCount(1);
  await expect(iconBubble).toBeInViewport({ ratio: 1 });

  await bothRow.locator(".room-access-badge").first().hover();
  const routeBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("access.conditionsRouteDescription") });
  await expect(routeBubble).toHaveCount(1);
  await expect(routeBubble).toBeInViewport({ ratio: 1 });

  // The trailing badge's tooltip — the one the old in-row bubble clipped — is
  // fully inside the viewport too.
  await bothRow.locator(".room-access-badge").last().hover();
  const requestBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("access.requestRouteDescription") });
  await expect(requestBubble).toHaveCount(1);
  await expect(requestBubble).toBeInViewport({ ratio: 1 });
});

test("the room header shows the active room's access condition", async ({ page }) => {
  await pushRoomList(page);
  const header = page.locator(".channel-header");

  await expect(header.locator(".channel-name")).toHaveText("Harness Room");
  await expect(header.locator(".channel-access-badge")).toHaveText([
    t("access.conditionsApply"),
    t("access.canRequest")
  ]);
  await expect(header.locator("[data-room-access]")).toHaveCount(0);

  // The combined explanation is available from the header badge.
  await header.locator(".channel-access-badge").first().hover();
  const headerBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("access.conditionsRouteDescription") });
  await expect(headerBubble).toHaveCount(1);
  await expect(headerBubble).toBeInViewport({ ratio: 1 });
});

test("the header uses the full unknown and checking labels", async ({ page }) => {
  await pushRoomList(page);
  const header = page.locator(".channel-header");
  await expect(header.locator(".channel-access-badge")).toHaveText([
    t("access.conditionsApply"),
    t("access.canRequest")
  ]);

  // An unrecognised rule reads as the full unknown label, not the compact one.
  await pushRoomRule(page, "unknown");
  await expect(header.locator(".channel-access-badge")).toHaveText([
    t("access.unknownFull")
  ]);

  // A rule that has not been projected yet reads as the full checking label.
  await pushRoomRule(page, undefined);
  await expect(header.locator(".channel-access-badge")).toHaveText([
    t("access.checkingFull")
  ]);
  await expect(header.locator(".channel-access-badge")).toBeVisible();
});
