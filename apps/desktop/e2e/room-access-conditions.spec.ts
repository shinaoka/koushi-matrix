import { expect, test, type Page } from "@playwright/test";
import type {
  DesktopSnapshot,
  RoomJoinRule,
  RoomListItem,
  SpaceSummary
} from "../src/domain/types";
import { roomAccessTooltipLabel } from "../src/app/uiShared";
import { catalogs, pseudoLocalize, t, type MessageId } from "../src/i18n/messages";
import { HARNESS_ROOM_ID } from "./support/basicOperations";
import { pushSnapshot } from "./support/stateUpdates";

// #1327: the room list, the conversation header and Room Info agree on what
// encryption and participation mean. The padlock is the encryption indicator
// and nothing else, every access state has one icon, one label and one
// explanation, and no surface invents a condition Rust has not projected.
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
const RESTRICTED_ROOM = {
  ...row("!conditional:example.invalid", "Conditional Room", "restricted"),
  access_restricted_conditions: "membershipOnly" as const,
  // Rust resolved a named route but no verified single-Space route, so the
  // generic sentence applies and the tooltip names the route it has.
  access_allowed_room_names: ["Allowed Room"]
};
const SPACE_ROUTE_ROOM = {
  ...row("!space-route:example.invalid", "Space Route Room", "restricted"),
  access_restricted_conditions: "membershipOnly" as const,
  access_allowed_room_names: ["Allowed Space"],
  access_space_members_route: "Allowed Space"
};
const SPACE_ROUTE_BOTH_ROOM = {
  ...row("!space-route-both:example.invalid", "Space Route Both Room", "knockRestricted"),
  access_restricted_conditions: "membershipOnly" as const,
  access_allowed_room_names: ["Allowed Space"],
  access_space_members_route: "Allowed Space"
};
const NO_USABLE_ROOM = {
  ...row("!no-usable:example.invalid", "No Usable Room", "restricted"),
  access_restricted_conditions: "confirmedEmpty" as const
};
const NO_USABLE_REQUEST_ROOM = {
  ...row("!no-usable-request:example.invalid", "No Usable Request Room", "knockRestricted"),
  access_restricted_conditions: "confirmedEmpty" as const
};
const UNKNOWN_ALLOW_ROOM = {
  ...row("!unknown-allow:example.invalid", "Unknown Allow Room", "restricted"),
  access_restricted_conditions: "unsupportedOnly" as const
};
const KNOCK_ROOM = row("!request:example.invalid", "Request Room", "knock");
const BOTH_ROOM = row("!both:example.invalid", "Both Room", "knockRestricted");
const UNKNOWN_ROOM = row("!unknown:example.invalid", "Unknown Room", "unknown");
const PRIVATE_ROOM = row("!private:example.invalid", "Private Room", "private");
const DM_ROW = { ...row("!synthetic-dm:example.invalid", "Synthetic Person", "invite"), is_dm: true };

const ROOMS = [
  PLAIN_ROOM,
  PUBLIC_ROOM,
  INVITE_ROOM,
  RESTRICTED_ROOM,
  SPACE_ROUTE_ROOM,
  SPACE_ROUTE_BOTH_ROOM,
  NO_USABLE_ROOM,
  NO_USABLE_REQUEST_ROOM,
  UNKNOWN_ALLOW_ROOM,
  KNOCK_ROOM,
  BOTH_ROOM,
  UNKNOWN_ROOM,
  PRIVATE_ROOM
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

/**
 * The harness's own snapshot, read in-page. Every helper that runs *after* the
 * first push must build on the state already pushed: `openHarness` navigates,
 * which throws the pushed room list away and leaves the header as `checking`.
 */
async function currentSnapshot(page: Page): Promise<DesktopSnapshot> {
  return page.evaluate(() =>
    (window as unknown as { __harness: Harness }).__harness.currentSnapshot()
  );
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
  const base = await currentSnapshot(page);
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

/** Push a snapshot whose active conversation is the given room summary. */
async function pushActiveRoom(
  page: Page,
  overrides: Partial<DesktopSnapshot["state"]["domain"]["rooms"][number]>
): Promise<void> {
  const base = await currentSnapshot(page);
  await pushSnapshot(page, {
    ...base,
    state: {
      ...base.state,
      domain: {
        ...base.state.domain,
        rooms: base.state.domain.rooms.map((candidate) =>
          candidate.room_id === HARNESS_ROOM_ID ? { ...candidate, ...overrides } : candidate
        )
      }
    }
  });
}

/** Switch the Rust-owned catalog locale without leaving the page. */
async function pushLocale(page: Page, catalog: "ja" | "pseudo"): Promise<void> {
  const base = await currentSnapshot(page);
  await pushSnapshot(page, {
    ...base,
    state: {
      ...base.state,
      domain: {
        ...base.state.domain,
        locale_profile: {
          ...base.state.domain.locale_profile,
          lang: catalog === "ja" ? "ja" : "ar-XB",
          dir: catalog === "ja" ? "ltr" : "rtl",
          catalog_locale: catalog,
          pseudo_locale: catalog === "pseudo" ? "bidi" : "none"
        }
      }
    }
  });
}

test("a room row shows exactly one access icon that is never a padlock (#1327)", async ({
  page
}) => {
  await pushRoomList(page);
  const rooms = page.getByRole("region", { name: t("roomList.categoryRooms"), exact: true });

  // No row anywhere in the list treats a padlock as a participation fact.
  await expect(rooms.locator('[data-room-access="padlock"]')).toHaveCount(0);

  // One icon per row, in the row's single tab stop, carrying the label as its
  // accessible name and the explanation as its description.
  const cases: Array<[string, string, string, string, string[]?]> = [
    ["Plain Room", "checking", "access.checking", "access.checkingDescription"],
    ["Open Room", "globe", "access.public", "access.publicDescription"],
    ["Invite Room", "userRoundPlus", "access.inviteOnly", "access.inviteOnlyDescription"],
    [
      "Conditional Room",
      "usersRound",
      "access.conditionsApply",
      "access.conditionsDescription",
      ["Allowed Room"]
    ],
    ["Request Room", "hand", "access.canRequest", "access.requestDescription"],
    ["Both Room", "hand", "access.knockRestrictedLabel", "access.knockRestrictedDescription"],
    ["Unknown Room", "circleHelp", "access.unknown", "access.unknownDescription"],
    ["Private Room", "circleHelp", "access.privateReserved", "access.privateReservedDescription"],
    [
      "No Usable Room",
      "usersRound",
      "access.conditionsApply",
      "access.restrictedNoUsableConditionsDescription"
    ],
    [
      "No Usable Request Room",
      "hand",
      "access.knockRestrictedLabel",
      "access.restrictedNoUsableConditionsCanRequestDescription"
    ],
    [
      "Unknown Allow Room",
      "usersRound",
      "access.conditionsApply",
      "access.conditionsDescription"
    ]
  ];

  for (const [name, glyph, labelId, descriptionId, allowedNames] of cases) {
    const item = rooms.getByRole("button", { name, exact: true });
    const icons = item.locator("[data-room-access]");
    await expect(icons, name).toHaveCount(1);
    await expect(icons, name).toHaveAttribute("data-room-access", glyph);
    // The glyph is a named image, not a second tab stop and not a nested button.
    await expect(item.getByRole("img", { name: t(labelId as never) }), name).toHaveCount(1);
    await expect(item.locator("button"), name).toHaveCount(0);
    await expect(icons, name).not.toHaveAttribute("tabindex", /.+/);

    const describedBy = await item.getAttribute("aria-describedby");
    expect(describedBy, name).toBeTruthy();
    await expect(page.locator(`[id="${describedBy}"]`), name).toHaveText(
      // The row description is the explanation plus any routes Rust resolved.
      roomAccessTooltipLabel(descriptionId as MessageId, allowedNames)
    );
  }
});

test("a room row reveals its explanation on hover without clipping (#1327)", async ({ page }) => {
  await pushRoomList(page);
  const rooms = page.getByRole("region", { name: t("roomList.categoryRooms"), exact: true });

  // Hovering the first row's icon bubbles to the row's one tooltip host; the
  // bubble renders in the body-level floating layer, so the sidebar's
  // scrollport cannot clip it.
  const plainRow = rooms.getByRole("button", { name: "Plain Room", exact: true });
  await plainRow.locator("[data-room-access]").hover();
  const topBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("access.checkingDescription") });
  await expect(topBubble).toHaveCount(1);
  await expect(topBubble).toBeInViewport({ ratio: 1 });
  expect((await topBubble.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(0);

  // The named Space route replaces the generic facts for the verified row only.
  const routeRow = rooms.getByRole("button", { name: "Space Route Room", exact: true });
  await routeRow.locator("[data-room-access]").hover();
  await expect(
    page.locator("body > .tooltip-bubble.is-open").filter({
      hasText: t("access.spaceMembersCanJoinDescription", { space: "Allowed Space" })
    })
  ).toHaveCount(1);
  await expect(
    page.locator("body > .tooltip-bubble.is-open").filter({
      hasText: t("access.conditionsDescription")
    })
  ).toHaveCount(0);

  // A single-Space knock-restricted row keeps both of its routes in one pill.
  const bothRoute = rooms.getByRole("button", { name: "Space Route Both Room", exact: true });
  await expect(bothRoute.locator("[data-room-access]")).toHaveAttribute(
    "data-room-access",
    "hand"
  );
  await bothRoute.locator("[data-room-access]").hover();
  await expect(
    page.locator("body > .tooltip-bubble.is-open").filter({
      hasText: t("access.knockRestrictedSpaceDescription", { space: "Allowed Space" })
    })
  ).toHaveCount(1);

  // Keyboard focus reaches the same explanation through the row's description.
  const inviteRow = rooms.getByRole("button", { name: "Invite Room", exact: true });
  await inviteRow.focus();
  const describedBy = await inviteRow.getAttribute("aria-describedby");
  expect(describedBy).toBeTruthy();
  await expect(page.locator(`[id="${describedBy}"]`)).toHaveText(
    t("access.inviteOnlyDescription")
  );
});

test("status popups stay inside the viewport with Room Info open and closed (#1327)", async ({
  page
}) => {
  const expectBubbleInViewport = async (text: string) => {
    const bubble = page.locator("body > .tooltip-bubble.is-open").filter({ hasText: text });
    await expect(bubble).toHaveCount(1);
    await expect(bubble).toBeInViewport({ ratio: 1 });
    const box = (await bubble.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize()!.width);
    expect(box.y + box.height).toBeLessThanOrEqual(page.viewportSize()!.height);
  };

  await pushRoomList(page);
  const rooms = page.getByRole("region", { name: t("roomList.categoryRooms"), exact: true });
  const lastRow = rooms.getByRole("button", { name: "Private Room", exact: true });

  // Room Info closed: the last row sits at the bottom of the sidebar, where an
  // in-row bubble would be pushed out of the scrollport.
  await lastRow.locator("[data-room-access]").hover();
  await expectBubbleInViewport(t("access.privateReservedDescription"));

  // Room Info open: the panel narrows the conversation pane, so the header pill
  // and its bubble have less room than before. The header stays on the active
  // room (the harness room), whose knock-restricted rule explains both routes.
  await page.locator(".channel-header [data-room-access]").click();
  await expect(page.locator(".room-status-badges")).toBeVisible();

  await page.locator(".channel-header [data-room-access]").hover();
  await expectBubbleInViewport(t("access.knockRestrictedDescription"));

  await page.locator(".channel-header [data-room-encryption]").hover();
  await expectBubbleInViewport(t("room.notEncryptedDescription"));

  // The bubble renders in the body-level layer, never clipped by the panel.
  await expect(page.locator(".thread-pane .tooltip-bubble")).toHaveCount(0);
});

test("a DM row shows neither an access nor an encryption indicator (#1327)", async ({ page }) => {
  const base = await openHarness(page);
  await pushSnapshot(page, {
    ...base,
    sidebar: {
      ...base.sidebar,
      active_space_id: null,
      account_home: { ...base.sidebar.account_home, is_active: true },
      space_rail: base.sidebar.space_rail.map((space) => ({ ...space, is_active: false })),
      space_rooms: [],
      global_dms: [DM_ROW],
      sections: {
        favourites: [],
        not_joined: [],
        rooms: [],
        people: [DM_ROW],
        low_priority: []
      }
    }
  });

  const item = page.getByRole("button", { name: "Synthetic Person", exact: true });
  await expect(item).toBeVisible();
  await expect(item.locator("[data-room-access]")).toHaveCount(0);
  await expect(item.locator("[data-room-encryption]")).toHaveCount(0);
  await expect(item).not.toHaveAttribute("aria-describedby", /.+/);
});

/** Push the active room encrypted, with the room settings Room info renders. */
async function pushManagedRoom(
  page: Page,
  overrides: Partial<DesktopSnapshot["state"]["domain"]["rooms"][number]>,
  joinRule: RoomJoinRule = "knockRestricted",
  historyVisibility: "joined" | "invited" | "shared" | "worldReadable" = "shared"
): Promise<void> {
  const base = await currentSnapshot(page);
  await pushSnapshot(page, {
    ...base,
    state: {
      ...base.state,
      domain: {
        ...base.state.domain,
        rooms: base.state.domain.rooms.map((candidate) =>
          candidate.room_id === HARNESS_ROOM_ID ? { ...candidate, ...overrides } : candidate
        ),
        room_management: {
          selected_room_id: HARNESS_ROOM_ID,
          settings: {
            room_id: HARNESS_ROOM_ID,
            name: "Harness Room",
            topic: null,
            avatar_url: null,
            join_rule: joinRule,
            history_visibility: historyVisibility,
            permissions: {
              can_edit_settings: true,
              can_change_join_rule: true,
              can_edit_roles: true,
              can_invite: true,
              can_kick: true,
              can_ban: true,
              can_unban: true
            },
            members: []
          },
          operation: { kind: "idle" }
        }
      }
    }
  });
}

test("the conversation header shows independent encryption and access pills (#1327)", async ({
  page
}) => {
  await pushRoomList(page);
  const header = page.locator(".channel-header");

  // The harness room is unencrypted and knock-restricted: both facts show, and
  // neither is derived from the other.
  await expect(header.locator(".channel-name")).toHaveText("Harness Room");
  const encryption = header.locator("[data-room-encryption]");
  const access = header.locator("[data-room-access]");
  await expect(encryption).toHaveAttribute("data-room-encryption", "plaintext");
  await expect(encryption).toContainText(t("room.statusNotEncrypted"));
  await expect(access).toHaveAttribute("data-room-access", "hand");
  await expect(access).toHaveText(t("access.knockRestrictedLabel"));
  await expect(header.locator("[data-room-access='padlock']")).toHaveCount(0);

  // Order: avatar, name, encryption, participation — all on the one header row.
  const name = header.locator(".channel-name");
  const nameBox = (await name.boundingBox())!;
  const encryptionBox = (await encryption.boundingBox())!;
  const accessBox = (await access.boundingBox())!;
  expect(encryptionBox.x).toBeGreaterThanOrEqual(nameBox.x + nameBox.width - 1);
  expect(accessBox.x).toBeGreaterThanOrEqual(encryptionBox.x + encryptionBox.width - 1);
  await expect(header.locator(".channel-title")).toHaveCount(1);
  await expect(header.locator(".channel-actions [data-room-encryption]")).toHaveCount(0);
  await expect(header.locator(".channel-actions [data-room-access]")).toHaveCount(0);

  // Each pill explains itself, in the viewport.
  await encryption.hover();
  const encryptionBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("room.notEncryptedDescription") });
  await expect(encryptionBubble).toHaveCount(1);
  await expect(encryptionBubble).toBeInViewport({ ratio: 1 });

  await access.hover();
  const accessBubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: t("access.knockRestrictedDescription") });
  await expect(accessBubble).toHaveCount(1);
  await expect(accessBubble).toBeInViewport({ ratio: 1 });

  // An encrypted room says so, and the rule still comes from the projection.
  await pushActiveRoom(page, { is_encrypted: true });
  await expect(encryption).toHaveAttribute("data-room-encryption", "encrypted");
  await expect(encryption).toContainText(t("room.statusEncrypted"));
  await encryption.hover();
  await expect(
    page.locator("body > .tooltip-bubble.is-open").filter({ hasText: t("room.encryptedDescription") })
  ).toHaveCount(1);
});

test("the header access pill opens Room Info at the condition it summarises (#1327)", async ({
  page
}) => {
  await pushRoomList(page);
  const access = page.locator(".channel-header [data-room-access]");

  // It is a button — one tab stop, no nested control — whose accessible name
  // says what activation does.
  await expect(access).toHaveAttribute(
    "aria-label",
    t("room.statusShowSetting", { status: t("access.knockRestrictedLabel") })
  );
  await expect(access).toHaveJSProperty("tagName", "BUTTON");
  await expect(access.locator("button")).toHaveCount(0);

  await access.click();

  const joinRule = page.locator('[data-setting-property="join-rule"]');
  await expect(joinRule).toBeVisible();
  const heading = joinRule.getByRole("heading", { name: t("room.joinRule") });
  await expect(heading).toBeFocused();
});

test("Room Info carries the same three facts, and its summaries reveal them (#1327)", async ({
  page
}) => {
  // The harness answers the room-settings request as soon as Room Info opens, so
  // the pre-settings statement is covered by the panel's own unit test rather
  // than asserted here against a state the harness cannot hold.
  await pushRoomList(page);
  await pushManagedRoom(page, { is_encrypted: true });

  await page.locator(".channel-header [data-room-access]").click();
  const summary = page.locator(".room-status-badges");
  await expect(summary).toBeVisible();

  // Encryption is a fact, not a link: there is no setting to open for it.
  const encryptionBadge = summary
    .locator(".room-status-badge")
    .filter({ hasText: t("room.statusEncrypted") });
  await expect(encryptionBadge).toHaveCount(1);
  await expect(encryptionBadge).toHaveJSProperty("tagName", "SPAN");

  // The join condition and the history are links to the property they summarise.
  const joinBadge = summary.getByRole("button", {
    name: t("room.statusShowSetting", { status: t("access.knockRestrictedLabel") })
  });
  await expect(joinBadge).toHaveCount(1);
  await joinBadge.hover();
  await expect(
    page.locator("body > .tooltip-bubble.is-open").filter({
      hasText: t("access.knockRestrictedDescription")
    })
  ).toHaveCount(1);

  const historyBadge = summary.getByRole("button", {
    name: t("room.statusShowSetting", { status: t("room.statusHistoryShared") })
  });
  await expect(historyBadge).toHaveCount(1);
  await historyBadge.hover();
  await expect(
    page.locator("body > .tooltip-bubble.is-open").filter({
      hasText: t("room.statusHistorySharedDescription")
    })
  ).toHaveCount(1);

  await joinBadge.click();
  await expect(
    page.locator('[data-setting-property="join-rule"]').getByRole("heading", {
      name: t("room.joinRule")
    })
  ).toBeFocused();
});

test("the header keeps its one pinned row at 62px and 56px, above timeline and composer (#1327)", async ({
  page
}) => {
  await pushRoomList(page);
  await pushActiveRoom(page, { is_encrypted: true });

  const measure = () =>
    page.evaluate(() => {
      const header = document.querySelector(".channel-header") as HTMLElement;
      const name = header.querySelector(".channel-name") as HTMLElement;
      const main = document.querySelector(".main-pane") as HTMLElement;
      const timeline = main.querySelector(".timeline-scroll") as HTMLElement;
      const composer = main.querySelector(".composer") as HTMLElement;
      return {
        headerHeight: header.getBoundingClientRect().height,
        headerTop: header.getBoundingClientRect().top,
        headerBottom: header.getBoundingClientRect().bottom,
        timelineTop: timeline.getBoundingClientRect().top,
        composerTop: composer.getBoundingClientRect().top,
        nameLines: name.getClientRects().length,
        pillsRight:
          header.querySelector("[data-room-access]")?.getBoundingClientRect().right ?? -1,
        actionsLeft:
          (main.querySelector(".channel-actions") as HTMLElement)?.getBoundingClientRect()
            .left ?? -1,
        pillVisible: ["[data-room-encryption]", "[data-room-access]"].every((selector) => {
          const pill = header.querySelector(selector);
          if (!pill) return false;
          const box = pill.getBoundingClientRect();
          return box.width > 0 && box.height > 0;
        })
      };
    });

  const desktop = await measure();
  // #1327: two pills add no second row and do not grow the header.
  expect(desktop.headerHeight).toBeLessThanOrEqual(62.5);
  expect(desktop.nameLines).toBe(1);
  expect(desktop.pillVisible).toBe(true);
  // The pinned row stays above the timeline, which stays above the composer.
  expect(desktop.timelineTop).toBeGreaterThanOrEqual(desktop.headerBottom - 1);
  expect(desktop.composerTop).toBeGreaterThan(desktop.timelineTop);

  await page.setViewportSize({ width: 720, height: 700 });
  const compact = await measure();
  expect(compact.headerHeight).toBeLessThanOrEqual(56.5);
  expect(compact.nameLines).toBe(1);
  // At the smallest width only the pill text gives way: both icons and their
  // accessible descriptions stay.
  expect(compact.pillVisible).toBe(true);
  // The name gives way; the pills never collide with the existing actions.
  expect(compact.pillsRight).toBeLessThanOrEqual(compact.actionsLeft + 1);
  await expect(page.locator(".channel-header [data-room-encryption]")).toHaveAttribute(
    "aria-label",
    t("room.statusEncrypted")
  );
  await page.locator(".channel-header [data-room-access]").focus();
  await expect(
    page.locator("body > .tooltip-bubble.is-open").filter({
      hasText: t("access.knockRestrictedDescription")
    })
  ).toHaveCount(1);
});

test("a long name truncates while both pills stay visible and on one row (#1327)", async ({
  page
}) => {
  const base = await openHarness(page);
  const longName =
    "非常に長い日本語のルーム名がここに表示されます非常に長い日本語のルーム名がここに表示されます";
  const longRoom = { ...row(HARNESS_ROOM_ID, longName, "restricted"), access_restricted_conditions: "membershipOnly" as const };
  const rows = [...ROOMS, longRoom];
  await pushSnapshot(page, {
    ...base,
    sidebar: {
      ...base.sidebar,
      active_space_id: null,
      account_home: { ...base.sidebar.account_home, is_active: true },
      space_rooms: rows,
      global_dms: [],
      sections: { favourites: [], not_joined: [], rooms: rows, people: [], low_priority: [] }
    },
    state: {
      ...base.state,
      domain: {
        ...base.state.domain,
        rooms: base.state.domain.rooms.map((room) =>
          room.room_id === HARNESS_ROOM_ID ? { ...room, display_name: longName, display_label: longName, original_display_label: longName, is_encrypted: true } : room
        )
      }
    }
  });

  const geometry = await page.evaluate(() => {
    const header = document.querySelector(".channel-header") as HTMLElement;
    const name = header.querySelector(".channel-name") as HTMLElement;
    const encryption = header.querySelector("[data-room-encryption]") as HTMLElement;
    const access = header.querySelector("[data-room-access]") as HTMLElement;
    const boxes = [encryption, access].map((pill) => {
      const box = pill.getBoundingClientRect();
      return { width: box.width, height: box.height, top: box.top };
    });
    return {
      headerHeight: header.getBoundingClientRect().height,
      nameScrollWidth: name.scrollWidth,
      nameClientWidth: name.clientWidth,
      nameRight: name.getBoundingClientRect().right,
      nameLines: name.getClientRects().length,
      pillCount: boxes.length,
      pillsVisible: boxes.every((box) => box.width > 0 && box.height > 0),
      sameRow: boxes.every(
        (box) => Math.abs(box.top - name.getBoundingClientRect().top) < name.getBoundingClientRect().height
      ),
      firstPillLeft: boxes[0]?.width ? encryption.getBoundingClientRect().left : -1
    };
  });

  // The name, not the status pills, gives way.
  expect(geometry.nameScrollWidth).toBeGreaterThan(geometry.nameClientWidth);
  expect(geometry.nameLines).toBe(1);
  expect(geometry.pillCount).toBe(2);
  expect(geometry.pillsVisible).toBe(true);
  expect(geometry.sameRow).toBe(true);
  expect(geometry.firstPillLeft).toBeGreaterThanOrEqual(geometry.nameRight - 1);
  expect(geometry.headerHeight).toBeLessThanOrEqual(62.5);
});

test("the access explanation is localized, including the pseudo locale (#1327)", async ({
  page
}) => {
  await pushRoomList(page);

  await pushLocale(page, "ja");
  const jaRooms = page.getByRole("region", {
    name: catalogs.ja["roomList.categoryRooms"],
    exact: true
  });
  // The row glyph is a named image in the active catalog too.
  await expect(
    jaRooms.getByRole("button", { name: "Invite Room", exact: true }).getByRole("img", {
      name: catalogs.ja["access.inviteOnly"]
    })
  ).toHaveCount(1);
  await expect(page.locator(".channel-header [data-room-access]")).toHaveAttribute(
    "aria-label",
    catalogs.ja["room.statusShowSetting"].replace("{status}", catalogs.ja["access.knockRestrictedLabel"])
  );

  await pushLocale(page, "pseudo");
  // `t()` applies the pseudo transform at render time with the active mode.
  await expect(page.locator(".channel-header [data-room-encryption]")).toHaveAttribute(
    "aria-label",
    pseudoLocalize(catalogs.en["room.statusNotEncrypted"], "bidi")
  );
  await expect(page.locator(".channel-header [data-room-access]")).toHaveAttribute(
    "aria-label",
    pseudoLocalize(catalogs.en["room.statusShowSetting"], "bidi").replace(
      "{status}",
      pseudoLocalize(catalogs.en["access.knockRestrictedLabel"], "bidi")
    )
  );
});

function space(space_id: string, display_name: string, rule: RoomJoinRule | null): SpaceSummary {
  return {
    space_id,
    raw_name: null,
    display_name,
    avatar: null,
    join_rule: rule,
    child_room_ids: [],
    parent_side_child_room_ids: []
  };
}

const SPACES: SpaceSummary[] = [
  space("!open-space:example.invalid", "Open Space", "public"),
  space("!invite-space:example.invalid", "Invite Space", "invite"),
  space("!conditional-space:example.invalid", "Conditional Space", "restricted"),
  space("!request-space:example.invalid", "Request Space", "knock"),
  space("!unknown-space:example.invalid", "Unknown Space", "unknown"),
  space("!quiet-space:example.invalid", "Quiet Space", null)
];

function spaceRailItem(
  item: SpaceSummary,
  is_active: boolean,
  unread_count = 0,
  restricted?: "notInspected" | "confirmedEmpty" | "membershipOnly" | "membershipPlusUnsupported" | "unsupportedOnly",
  allowedRoomNames?: string[],
  spaceMembersRoute?: string
) {
  return {
    space_id: item.space_id,
    display_name: item.display_name,
    local_icon: null,
    avatar: null,
    unread_count,
    highlight_count: 0,
    is_active,
    access_join_rule: item.join_rule,
    ...(restricted ? { access_restricted_conditions: restricted } : {}),
    ...(allowedRoomNames ? { access_allowed_room_names: allowedRoomNames } : {}),
    ...(spaceMembersRoute ? { access_space_members_route: spaceMembersRoute } : {}),
    leave_candidates: []
  };
}

async function pushSpaces(page: Page, activeSpaceId: string | null): Promise<void> {
  const base = await openHarness(page);
  await pushSnapshot(page, {
    ...base,
    sidebar: {
      ...base.sidebar,
      space_rail: SPACES.map((item) =>
        spaceRailItem(
          item,
          item.space_id === activeSpaceId,
          item.space_id === "!invite-space:example.invalid" ? 3 : 0,
          item.space_id === "!conditional-space:example.invalid" ? "confirmedEmpty" : undefined
        )
      ),
      account_home: { ...base.sidebar.account_home, is_active: activeSpaceId === null }
    },
    state: {
      ...base.state,
      domain: { ...base.state.domain, spaces: SPACES },
      ui: {
        ...base.state.ui,
        navigation: {
          ...base.state.ui.navigation,
          active_space_id: activeSpaceId
        }
      }
    }
  });
}

test("the Space rail summarises each access condition and keeps the unread badge (#1327)", async ({
  page
}) => {
  await pushSpaces(page, null);
  const rail = page.getByRole("navigation", { name: t("workspace.workspaces") });

  // A padlock never stands for participation, in the rail or anywhere else.
  await expect(rail.locator('[data-space-access="padlock"]')).toHaveCount(0);
  await expect(rail.locator('[data-space-access="globe"]')).toHaveCount(1);
  await expect(rail.locator('[data-space-access="userRoundPlus"]')).toHaveCount(1);
  await expect(rail.locator('[data-space-access="usersRound"]')).toHaveCount(1);
  await expect(rail.locator('[data-space-access="hand"]')).toHaveCount(1);
  await expect(rail.locator('[data-space-access="circleHelp"]')).toHaveCount(1);
  await expect(rail.locator('[data-space-access="checking"]')).toHaveCount(1);

  // Each glyph belongs to its own Space, not merely to the rail as a whole.
  await expect(
    rail.getByRole("button", { name: "Open Space", exact: true }).locator('[data-space-access="globe"]')
  ).toHaveCount(1);
  await expect(
    rail
      .getByRole("button", { name: "Conditional Space", exact: true })
      .locator('[data-space-access="usersRound"]')
  ).toHaveCount(1);
  await expect(
    rail.getByRole("button", { name: "Quiet Space", exact: true }).locator('[data-space-access="checking"]')
  ).toHaveCount(1);

  // The rail item keeps the Space name and explains access, and the bubble is
  // not clipped by the rail.
  const inviteItem = rail.getByRole("button", { name: "Invite Space", exact: true });
  await inviteItem.hover();
  const bubble = page
    .locator("body > .tooltip-bubble.is-open")
    .filter({ hasText: `Invite Space${t("access.conditionSummarySeparator")}${t("access.inviteOnlyDescription")}` });
  await expect(bubble).toHaveCount(1);
  await expect(bubble).toBeInViewport({ ratio: 1 });

  // Keyboard focus on the rail item reaches the same explanation, through the
  // button's own description.
  await inviteItem.focus();
  const describedBy = await inviteItem.getAttribute("aria-describedby");
  expect(describedBy).toBeTruthy();
  await expect(page.locator(`[id="${describedBy}"]`)).toHaveText(
    `Invite Space${t("access.conditionSummarySeparator")}${t("access.inviteOnlyDescription")}`
  );

  // The unread badge keeps the lower trailing corner; the access overlay stays
  // in the upper half so it cannot obscure it.
  await expect(inviteItem).toHaveAttribute("data-count", "3");
  const itemBox = await inviteItem.boundingBox();
  const overlayBox = await inviteItem.locator("[data-space-access]").boundingBox();
  expect(itemBox).not.toBeNull();
  expect(overlayBox).not.toBeNull();
  expect(overlayBox!.y + overlayBox!.height).toBeLessThanOrEqual(
    itemBox!.y + itemBox!.height / 2
  );
});

test("the Space header uses the same participation vocabulary as the room list (#1327)", async ({
  page
}) => {
  await pushSpaces(page, "!open-space:example.invalid");
  const header = page.locator(".workspace-header");

  await expect(header.locator(".workspace-name")).toHaveText("Open Space");
  await expect(header.locator('[data-space-access="globe"]')).toHaveCount(1);
  await expect(header.locator(".workspace-access-badge")).toHaveText([t("access.public")]);

  // An invite-only Space never shows a padlock: a padlock means encryption.
  await pushSpaces(page, "!invite-space:example.invalid");
  await expect(header.locator(".workspace-name")).toHaveText("Invite Space");
  await expect(header.locator('[data-space-access="userRoundPlus"]')).toHaveCount(1);
  await expect(header.locator('[data-space-access="padlock"]')).toHaveCount(0);
  await expect(header.locator(".workspace-access-badge")).toHaveText([t("access.inviteOnly")]);

  // An unrecognised rule reads as the unknown label.
  await pushSpaces(page, "!unknown-space:example.invalid");
  await expect(header.locator(".workspace-name")).toHaveText("Unknown Space");
  await expect(header.locator('[data-space-access="circleHelp"]')).toHaveCount(1);
  await expect(header.locator(".workspace-access-badge")).toHaveText([t("access.unknown")]);

  // A Space whose rule has not been projected yet reads as checking.
  await pushSpaces(page, "!quiet-space:example.invalid");
  await expect(header.locator('[data-space-access="checking"]')).toHaveCount(1);
  await expect(header.locator(".workspace-access-badge")).toHaveText([t("access.checking")]);
});

test("the room row and the header never claim a condition Rust did not project (#1327)", async ({
  page
}) => {
  await pushRoomList(page);

  // A knock-restricted row with no usable condition still says a request is
  // possible, so it is not presented as plain invite-only.
  const noUsable = page
    .getByRole("region", { name: t("roomList.categoryRooms"), exact: true })
    .getByRole("button", { name: "No Usable Request Room", exact: true });
  await expect(noUsable.locator("[data-room-access]")).toHaveAttribute("data-room-access", "hand");
  await expect(noUsable).toContainText("No Usable Request Room");
  const describedBy = await noUsable.getAttribute("aria-describedby");
  expect(describedBy).toBeTruthy();
  await expect(page.locator(`[id="${describedBy}"]`)).toHaveText(
    t("access.restrictedNoUsableConditionsCanRequestDescription")
  );

  // The header follows the projection: an unknown rule is unknown, and a room
  // whose rule was never projected is checking — never a guessed condition.
  const header = page.locator(".channel-header [data-room-access]");
  await pushRoomRule(page, "unknown");
  await expect(header).toHaveText(t("access.unknown"));
  await expect(header).toHaveAttribute("data-room-access", "circleHelp");

  await pushRoomRule(page, undefined);
  await expect(header).toHaveText(t("access.checking"));
  await expect(header).toHaveAttribute("data-room-access", "checking");
});
