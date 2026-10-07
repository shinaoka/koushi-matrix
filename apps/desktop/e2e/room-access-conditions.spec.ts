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

  // Hovering the icon or a badge explains it.
  await rooms.locator('[data-room-access="padlock"]').hover();
  await expect(
    rooms.locator(".tooltip-bubble.is-open").filter({ hasText: t("access.inviteOnlyDescription") })
  ).toHaveCount(1);
  await bothRow.locator(".room-access-badge").first().hover();
  await expect(
    rooms
      .locator(".tooltip-bubble.is-open")
      .filter({ hasText: t("access.conditionsRouteDescription") })
  ).toHaveCount(1);
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
  await expect(
    header
      .locator(".tooltip-bubble.is-open")
      .filter({ hasText: t("access.conditionsRouteDescription") })
  ).toHaveCount(1);
});
