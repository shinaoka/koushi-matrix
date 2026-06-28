import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { computeBrowserRoomListProjection } from "../backend/roomListProjection";
import { composeSidebar, projectRoomSummaries, roomListSections, visibleRooms } from "./desktopModel";
import type { DesktopSnapshot, RoomSummary, RoomTags, SpaceSummary } from "./types";

describe("desktop model", () => {
  test("Home (null space) global_dms includes all DMs regardless of dm_space_ids", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        avatar: null,
        child_room_ids: ["!room-a:example.invalid"]
      }
    ];
    const rooms: RoomSummary[] = [
      roomSummary("!room-a:example.invalid", "Alpha room", false),
      roomSummaryWithDmSpaces("!dm-in-space:example.invalid", "In-space DM", [
        "!space-a:example.invalid"
      ]),
      roomSummaryWithDmSpaces("!dm-no-space:example.invalid", "No-space DM", [])
    ];

    const sidebar = composeSidebar(null, spaces, rooms);

    expect(sidebar.global_dms.map((room) => room.room_id)).toEqual([
      "!dm-in-space:example.invalid",
      "!dm-no-space:example.invalid"
    ]);
  });

  test("aggregate sidebar badges ignore muted rooms while room items keep counts", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        avatar: null,
        child_room_ids: ["!room-a:example.invalid", "!room-b:example.invalid"]
      }
    ];
    const mutedRoom = {
      ...roomSummary("!room-a:example.invalid", "Muted room", false, undefined, [
        "!space-a:example.invalid"
      ]),
      unread_count: 5,
      highlight_count: 1
    };
    const normalRoom = {
      ...roomSummary("!room-b:example.invalid", "Normal room", false, undefined, [
        "!space-a:example.invalid"
      ]),
      unread_count: 2
    };
    const dmRoom = {
      ...roomSummaryWithDmSpaces("!dm-a:example.invalid", "DM", ["!space-a:example.invalid"]),
      unread_count: 3
    };

    const sidebar = composeSidebar(
      "!space-a:example.invalid",
      spaces,
      [mutedRoom, normalRoom, dmRoom],
      {
        "!room-a:example.invalid": { mode: { kind: "mute" }, operation: { kind: "idle" } }
      }
    );

    expect(
      sidebar.space_rooms.find((room) => room.room_id === mutedRoom.room_id)?.unread_count
    ).toBe(5);
    expect(sidebar.account_home.unread_count).toBe(5);
    expect(sidebar.account_home.highlight_count).toBe(0);
    expect(sidebar.space_rail[0]?.unread_count).toBe(2);
    expect(sidebar.space_unread_count).toBe(2);
    expect(sidebar.dm_unread_count).toBe(3);
  });

  test("active space global_dms shows only DMs whose dm_space_ids includes that space", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        avatar: null,
        child_room_ids: ["!room-a:example.invalid"]
      }
    ];
    const rooms: RoomSummary[] = [
      roomSummary("!room-a:example.invalid", "Alpha room", false),
      roomSummaryWithDmSpaces("!dm-in-space:example.invalid", "In-space DM", [
        "!space-a:example.invalid"
      ]),
      roomSummaryWithDmSpaces("!dm-no-space:example.invalid", "No-space DM", [])
    ];

    const sidebar = composeSidebar("!space-a:example.invalid", spaces, rooms);

    expect(sidebar.space_rooms.map((room) => room.room_id)).toEqual([
      "!room-a:example.invalid"
    ]);
    expect(sidebar.global_dms.map((room) => room.room_id)).toEqual([
      "!dm-in-space:example.invalid"
    ]);
  });

  test("DM with multiple dm_space_ids appears under each matching space", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        avatar: null,
        child_room_ids: []
      },
      {
        space_id: "!space-b:example.invalid",
        display_name: "Beta",
        avatar: null,
        child_room_ids: []
      }
    ];
    const rooms: RoomSummary[] = [
      roomSummaryWithDmSpaces("!dm-both:example.invalid", "Both spaces DM", [
        "!space-a:example.invalid",
        "!space-b:example.invalid"
      ]),
      roomSummaryWithDmSpaces("!dm-only-a:example.invalid", "Space-A only DM", [
        "!space-a:example.invalid"
      ])
    ];

    const sidebarA = composeSidebar("!space-a:example.invalid", spaces, rooms);
    const sidebarB = composeSidebar("!space-b:example.invalid", spaces, rooms);

    expect(sidebarA.global_dms.map((room) => room.room_id)).toEqual([
      "!dm-both:example.invalid",
      "!dm-only-a:example.invalid"
    ]);
    expect(sidebarB.global_dms.map((room) => room.room_id)).toEqual([
      "!dm-both:example.invalid"
    ]);
  });

  test("fake API: selecting a space shows only DMs with matching dm_space_ids", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-beta:example.invalid");

    const rooms = visibleRooms(snapshot);

    // space_rooms has only the non-DM child room of space-beta
    expect(rooms.spaceRooms.map((room) => room.room_id)).toEqual([
      "!room-search:example.invalid"
    ]);
    // fake DMs have dm_space_ids: [] so none appear under an active space (C1 behaviour)
    expect(rooms.globalDms.map((room) => room.room_id)).toEqual([]);
    expect(rooms.spaceRooms.every((room) => !room.room_id.startsWith("!dm-"))).toBe(
      true
    );
  });

  test("account home lists only non-DM rooms that are not in any space while DMs stay global", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        avatar: null,
        child_room_ids: ["!room-a:example.invalid", "!dm-a:example.invalid"]
      }
    ];
    const rooms: RoomSummary[] = [
      {
        room_id: "!room-a:example.invalid",
        display_name: "Alpha room",
        display_label: "Alpha room",
        original_display_label: "Alpha room",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: { favourite: { order: "0.25" }, low_priority: null },
        parent_space_ids: ["!space-a:example.invalid"],
        dm_space_ids: [],
        is_encrypted: false,
        unread_count: 5
      },
      {
        room_id: "!global-room:example.invalid",
        display_name: "Global room",
        display_label: "Global room",
        original_display_label: "Global room",
        avatar: null,
        is_dm: false,
        dm_user_ids: [],
        tags: { favourite: null, low_priority: null },
        parent_space_ids: [],
        dm_space_ids: [],
        is_encrypted: false,
        unread_count: 2
      },
      {
        room_id: "!dm-a:example.invalid",
        display_name: "Direct upstream",
        display_label: "Direct local",
        original_display_label: "Direct upstream",
        avatar: null,
        is_dm: true,
        dm_user_ids: ["@direct:example.invalid"],
        tags: { favourite: null, low_priority: null },
        parent_space_ids: ["!space-a:example.invalid"],
        dm_space_ids: [],
        is_encrypted: false,
        unread_count: 3
      }
    ];

    const sidebar = composeSidebar(null, spaces, rooms);

    expect(sidebar.account_home).toMatchObject({
      display_name: "Home",
      unread_count: 10,
      is_active: true
    });
    expect(sidebar.space_rooms.map((room) => room.room_id)).toEqual([
      "!global-room:example.invalid"
    ]);
    expect(sidebar.global_dms.map((room) => room.room_id)).toEqual([
      "!dm-a:example.invalid"
    ]);
    expect(sidebar.global_dms[0]?.display_name).toBe("Direct local");
  });

  test("projects direct-message room avatars from counterpart profiles", () => {
    const profileAvatar = {
      mxc_uri: "mxc://example.invalid/direct-avatar",
      thumbnail: {
        kind: "ready" as const,
        source_url: "asset://direct-avatar.png",
        width: 64,
        height: 64,
        mime_type: "image/png"
      }
    };
    const rooms = projectRoomSummaries(
      [roomSummary("!dm-a:example.invalid", "Direct upstream", true)],
      {
        own: { display_name: null, avatar: null },
        users: {
          "@dm-a:example.invalid": {
            user_id: "@dm-a:example.invalid",
            display_name: "Direct upstream",
            display_label: "Direct local",
            original_display_label: "Direct upstream",
            mention_search_terms: ["Direct local", "@dm-a:example.invalid"],
            avatar: profileAvatar
          }
        },
        local_aliases: {},
        local_alias_update: { kind: "idle" },
        ignored_user_ids: [],
        ignored_user_update: { kind: "idle" },
        update: { kind: "idle" }
      }
    );
    const sidebar = composeSidebar(null, [], rooms);

    expect(rooms[0]?.display_label).toBe("Direct local");
    expect(rooms[0]?.avatar).toBe(profileAvatar);
    expect(sidebar.global_dms[0]?.avatar).toBe(profileAvatar);
  });


  test("room list sections are derived from Rust-owned tags and DM classification", () => {
    const rooms: RoomSummary[] = [
      roomSummary("!fav:example.invalid", "Favourite", false, {
        favourite: { order: "0.10" },
        low_priority: null
      }),
      roomSummary("!plain:example.invalid", "Plain", false),
      roomSummary("!low:example.invalid", "Low", false, {
        favourite: null,
        low_priority: { order: "0.90" }
      }),
      roomSummary("!dm-fav:example.invalid", "Favourite DM", true, {
        favourite: { order: "0.05" },
        low_priority: null
      })
    ];
    const roomList: DesktopSnapshot["state"]["ui"]["room_list"] = {
      active_filter: { kind: "rooms" },
      sort: { kind: "activity" },
      items: null
    };

    const sections = roomListSections(roomList, null, [], rooms, []);

    expect(sections.favourites.map((room) => room.room_id)).toEqual([
      "!fav:example.invalid"
    ]);
    expect(sections.invites).toEqual([]);
    expect(sections.rooms.map((room) => room.room_id)).toEqual([
      "!plain:example.invalid"
    ]);
    expect(sections.lowPriority.map((room) => room.room_id)).toEqual([
      "!low:example.invalid"
    ]);
    expect(sections.people.map((room) => room.room_id)).toEqual([
      "!dm-fav:example.invalid"
    ]);
  });

  test("room list sections use Rust-projected items order and membership", () => {
    const rooms: RoomSummary[] = [
      roomSummary("!plain:example.invalid", "Plain", false),
      roomSummary("!fav:example.invalid", "Favourite", false, {
        favourite: { order: "0.10" },
        low_priority: null
      }),
      roomSummary("!dm-fav:example.invalid", "Favourite DM", true, {
        favourite: { order: "0.05" },
        low_priority: null
      })
    ];
    const roomList: DesktopSnapshot["state"]["ui"]["room_list"] = {
      active_filter: { kind: "rooms" },
      sort: { kind: "activity" },
      items: [
        { room_id: "!dm-fav:example.invalid", kind: "room" },
        { room_id: "!fav:example.invalid", kind: "room" },
        { room_id: "!plain:example.invalid", kind: "room" }
      ]
    };

    const sections = roomListSections(roomList, null, [], rooms, []);

    expect(sections.people.map((room) => room.room_id)).toEqual([
      "!dm-fav:example.invalid"
    ]);
    expect(sections.favourites.map((room) => room.room_id)).toEqual([
      "!fav:example.invalid"
    ]);
    expect(sections.rooms.map((room) => room.room_id)).toEqual([
      "!plain:example.invalid"
    ]);
  });

  test("room list projection sorts by activity with id fallback", () => {
    const rooms: RoomSummary[] = [
      roomSummaryWithActivity("!b:example.invalid", "Beta", false, 200),
      roomSummaryWithActivity("!a:example.invalid", "Alpha", false, 100)
    ];

    const projection = computeBrowserRoomListProjection(
      { kind: "rooms" },
      { kind: "activity" },
      null,
      [],
      rooms,
      []
    );

    expect(projection.items?.map((item) => item.room_id)).toEqual([
      "!b:example.invalid",
      "!a:example.invalid"
    ]);
  });

  test("room list projection sorts recentFirst by activity", () => {
    const rooms: RoomSummary[] = [
      roomSummaryWithActivity("!b:example.invalid", "Beta", false, 100),
      roomSummaryWithActivity("!a:example.invalid", "Alpha", false, 300),
      roomSummaryWithActivity("!c:example.invalid", "Charlie", false, 200)
    ];

    const projection = computeBrowserRoomListProjection(
      { kind: "rooms" },
      { kind: "recentFirst" },
      null,
      [],
      rooms,
      []
    );

    expect(projection.items?.map((item) => item.room_id)).toEqual([
      "!a:example.invalid",
      "!c:example.invalid",
      "!b:example.invalid"
    ]);
  });

  test("room list projection sorts alphabetically in normalLocale order", () => {
    const rooms: RoomSummary[] = [
      roomSummary("!b:example.invalid", "Beta", false),
      roomSummary("!a:example.invalid", "alpha", false),
      roomSummary("!c:example.invalid", "Charlie", false)
    ];

    const projection = computeBrowserRoomListProjection(
      { kind: "rooms" },
      { kind: "normalLocale" },
      null,
      [],
      rooms,
      []
    );

    expect(projection.items?.map((item) => item.room_id)).toEqual([
      "!a:example.invalid",
      "!b:example.invalid",
      "!c:example.invalid"
    ]);
  });

  test("active space people projection includes only DMs tagged to that space", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        avatar: null,
        child_room_ids: []
      }
    ];
    const rooms: RoomSummary[] = [
      roomSummaryWithDmSpaces("!dm-in-space:example.invalid", "In-space DM", [
        "!space-a:example.invalid"
      ]),
      roomSummaryWithDmSpaces("!dm-no-space:example.invalid", "No-space DM", [])
    ];

    const homePeople = computeBrowserRoomListProjection(
      { kind: "people" },
      { kind: "activity" },
      null,
      spaces,
      rooms,
      []
    );
    expect(homePeople.items?.map((item) => item.room_id)).toEqual([
      "!dm-in-space:example.invalid",
      "!dm-no-space:example.invalid"
    ]);

    const spacePeople = computeBrowserRoomListProjection(
      { kind: "people" },
      { kind: "activity" },
      "!space-a:example.invalid",
      spaces,
      rooms,
      []
    );
    expect(spacePeople.items?.map((item) => item.room_id)).toEqual([
      "!dm-in-space:example.invalid"
    ]);
  });

  test("room list projection keeps DMs in the people filter only", () => {
    const rooms: RoomSummary[] = [
      roomSummary("!room:example.invalid", "Room", false),
      roomSummary("!dm:example.invalid", "Direct", true)
    ];

    const roomsFilter = computeBrowserRoomListProjection(
      { kind: "rooms" },
      { kind: "activity" },
      null,
      [],
      rooms,
      []
    );
    const peopleFilter = computeBrowserRoomListProjection(
      { kind: "people" },
      { kind: "activity" },
      null,
      [],
      rooms,
      []
    );

    expect(roomsFilter.items?.map((item) => item.room_id)).toEqual([
      "!room:example.invalid"
    ]);
    expect(peopleFilter.items?.map((item) => item.room_id)).toEqual(["!dm:example.invalid"]);
  });

  test("browser fake updateSettings reprojects room-list sort", async () => {
    const api = createBrowserFakeApi();
    await api.selectSpace("!space-alpha:example.invalid");

    const localeSorted = await api.updateSettings({
      room_list_sort: { kind: "normalLocale" }
    });
    expect(localeSorted.state.ui.room_list.sort).toEqual({ kind: "normalLocale" });
    expect(localeSorted.state.ui.room_list.items?.map((item) => item.room_id)).toEqual([
      "!room-planning:example.invalid",
      "!room-alpha:example.invalid"
    ]);

    const activitySorted = await api.updateSettings({
      room_list_sort: { kind: "activity" }
    });
    expect(activitySorted.state.ui.room_list.sort).toEqual({ kind: "activity" });
    expect(activitySorted.state.ui.room_list.items?.map((item) => item.room_id)).toEqual([
      "!room-alpha:example.invalid",
      "!room-planning:example.invalid"
    ]);
  });

  test("room list sections keep DMs global when the active Rooms projection omits them", () => {
    const rooms: RoomSummary[] = [
      roomSummary("!plain:example.invalid", "Plain", false),
      roomSummary("!dm:example.invalid", "Direct", true)
    ];
    const roomList: DesktopSnapshot["state"]["ui"]["room_list"] = {
      active_filter: { kind: "rooms" },
      sort: { kind: "activity" },
      items: [{ room_id: "!plain:example.invalid", kind: "room" }]
    };

    const sections = roomListSections(roomList, null, [], rooms, []);

    expect(sections.rooms.map((room) => room.room_id)).toEqual([
      "!plain:example.invalid"
    ]);
    expect(sections.people.map((room) => room.room_id)).toEqual([
      "!dm:example.invalid"
    ]);
  });

  test("room list sections keep invite projection entries out of favourites", () => {
    const roomList: DesktopSnapshot["state"]["ui"]["room_list"] = {
      active_filter: { kind: "invites" },
      sort: { kind: "activity" },
      items: [{ room_id: "!invite:example.invalid", kind: "invite" }]
    };

    const sections = roomListSections(
      roomList,
      null,
      [],
      [],
      [
        {
          room_id: "!invite:example.invalid",
          display_name: "Invite Room",
          avatar: null,
          topic: null,
          inviter_display_name: "Inviter",
          inviter_user_id: "@inviter:example.invalid",
          is_dm: false
        }
      ]
    );

    expect(sections.invites.map((room) => room.room_id)).toEqual([
      "!invite:example.invalid"
    ]);
    expect(sections.favourites).toEqual([]);
    expect(sections.rooms).toEqual([]);
  });

  test("room list sections fall back to sidebar when projection is unavailable", () => {
    const rooms: RoomSummary[] = [
      roomSummary("!plain:example.invalid", "Plain", false),
      roomSummary("!fav:example.invalid", "Favourite", false, {
        favourite: { order: "0.10" },
        low_priority: null
      })
    ];
    const roomList: DesktopSnapshot["state"]["ui"]["room_list"] = {
      active_filter: { kind: "rooms" },
      sort: { kind: "activity" },
      items: null
    };

    const sections = roomListSections(roomList, null, [], rooms, []);

    expect(sections.rooms.map((room) => room.room_id)).toEqual([
      "!plain:example.invalid"
    ]);
    expect(sections.favourites.map((room) => room.room_id)).toEqual([
      "!fav:example.invalid"
    ]);
  });

  test("room list fallback respects the active space scope", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        avatar: null,
        child_room_ids: ["!in-space:example.invalid"]
      },
      {
        space_id: "!space-empty:example.invalid",
        display_name: "Empty",
        avatar: null,
        child_room_ids: []
      }
    ];
    const rooms: RoomSummary[] = [
      roomSummary("!in-space:example.invalid", "In space", false),
      roomSummary("!outside:example.invalid", "Outside", false),
      // DM is tagged to space-a so it shows there; not tagged to space-empty so absent there
      roomSummaryWithDmSpaces("!dm:example.invalid", "Direct", ["!space-a:example.invalid"])
    ];
    const roomList: DesktopSnapshot["state"]["ui"]["room_list"] = {
      active_filter: { kind: "rooms" },
      sort: { kind: "activity" },
      items: null
    };

    const sections = roomListSections(
      roomList,
      "!space-a:example.invalid",
      spaces,
      rooms,
      []
    );
    const emptySections = roomListSections(
      roomList,
      "!space-empty:example.invalid",
      spaces,
      rooms,
      []
    );

    expect(sections.rooms.map((room) => room.room_id)).toEqual([
      "!in-space:example.invalid"
    ]);
    // DM is tagged to space-a so it appears here (C1)
    expect(sections.people.map((room) => room.room_id)).toEqual([
      "!dm:example.invalid"
    ]);
    expect(emptySections.rooms).toEqual([]);
    expect(emptySections.favourites).toEqual([]);
    expect(emptySections.lowPriority).toEqual([]);
    // DM is NOT tagged to space-empty so it is absent (C1)
    expect(emptySections.people).toEqual([]);
  });

  test("fake search keeps exact matches and drops ngram false positives", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("Alpha", "allRooms");

    const results =
      snapshot.state.domain.search.kind === "results" ? snapshot.state.domain.search.results : [];

    expect(results.map((result) => result.event_id)).toEqual(["$alpha-update"]);
    expect(results[0]?.match_field).toBe("messageBody");
    expect(results[0]?.highlights).toEqual([{ start_utf16: 0, end_utf16: 5 }]);
  });

  test("browser fake sends text into the active timeline", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.sendText(
      "!room-alpha:example.invalid",
      "Synthetic message from composer"
    );

    expect(snapshot.timeline.at(-1)).toMatchObject({
      room_id: "!room-alpha:example.invalid",
      sender: "@demo-user:example.invalid",
      body: "Synthetic message from composer"
    });
  });

  test("browser fake edits and redacts a sent timeline message", async () => {
    const api = createBrowserFakeApi();
    let snapshot = await api.sendText(
      "!room-alpha:example.invalid",
      "Synthetic message before edit"
    );
    const eventId = snapshot.timeline.at(-1)?.event_id;
    if (!eventId) {
      throw new Error("expected sent event id");
    }

    snapshot = await api.editMessage(
      "!room-alpha:example.invalid",
      eventId,
      "Synthetic message after edit"
    );

    expect(snapshot.timeline.at(-1)).toMatchObject({
      event_id: eventId,
      body: "Synthetic message after edit"
    });

    snapshot = await api.redactMessage("!room-alpha:example.invalid", eventId);

    expect(snapshot.timeline.map((message) => message.event_id)).not.toContain(eventId);
  });

  test("fake search includes attachment filenames as a separate match field", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("fixture_budget.xlsx", "allRooms");

    const results =
      snapshot.state.domain.search.kind === "results" ? snapshot.state.domain.search.results : [];

    expect(results).toHaveLength(1);
    expect(results[0]?.event_id).toBe("$budget-file");
    expect(results[0]?.match_field).toBe("attachmentFileName");
  });

  test("browser fake can start signed out and exposes the pre-login boundary", async () => {
    const api = createBrowserFakeApi({ restoreSession: false });

    let snapshot = await api.getSnapshot();

    expect(snapshot.state.domain.session.kind).toBe("signedOut");
    expect(snapshot.state.domain.rooms).toHaveLength(0);
    expect(snapshot.state.ui.errors).toHaveLength(0);

    snapshot = await api.submitLogin(
      "https://matrix.example.org",
      "demo-user",
      "synthetic-password",
      "Koushi Test"
    );

    expect(snapshot.state.domain.session.kind).toBe("signedOut");
    expect(snapshot.state.domain.rooms).toHaveLength(0);
    expect(snapshot.state.ui.errors).toHaveLength(1);
    expect(snapshot.state.ui.errors[0]?.code).toBe("login_failed");
    expect(JSON.stringify(snapshot)).not.toContain("synthetic-password");
  });

  test("browser fake discovers password and sso login methods", async () => {
    const api = createBrowserFakeApi({ restoreSession: false });

    const snapshot = await api.discoverLoginMethods("matrix.example.org:8448");

    expect(snapshot.state.domain.auth.kind).toBe("ready");
    if (snapshot.state.domain.auth.kind !== "ready") {
      throw new Error("expected discovered login methods");
    }

    expect(snapshot.state.domain.auth.homeserver).toBe("https://matrix.example.org:8448");
    expect(snapshot.state.domain.auth.flows.map((flow) => flow.kind)).toEqual([
      "password",
      "sso"
    ]);
    expect(snapshot.state.domain.auth.flows[1]?.delegated_oidc_compatibility).toBe(true);
  });

  test("browser fake can expose a post-login e2ee recovery step", async () => {
    const api = createBrowserFakeApi({ session: "needsRecovery" });

    let snapshot = await api.getSnapshot();

    expect(snapshot.state.domain.session.kind).toBe("needsRecovery");
    expect(snapshot.state.domain.rooms.length).toBeGreaterThan(0);
    expect(snapshot.timeline.length).toBeGreaterThan(0);
    expect(snapshot.state.ui.navigation.active_room_id).toBeTruthy();
    expect(snapshot.state.domain.sync).toBe("running");
    expect(snapshot.state.domain.session.recovery_methods).toEqual([
      "recoveryKey",
      "securityPhrase"
    ]);

    snapshot = await api.submitRecovery("synthetic-recovery-secret");

    expect(snapshot.state.domain.session.kind).toBe("ready");
    expect(snapshot.state.domain.sync).toBe("running");
    expect(JSON.stringify(snapshot)).not.toContain("synthetic-recovery-secret");
  });

  test("browser fake keeps synced room navigation and search available during recovery", async () => {
    const api = createBrowserFakeApi({ session: "needsRecovery" });

    let snapshot = await api.selectRoom("!room-planning:example.invalid");

    expect(snapshot.state.domain.session.kind).toBe("needsRecovery");
    expect(snapshot.state.ui.navigation.active_room_id).toBe("!room-planning:example.invalid");
    expect(snapshot.timeline.map((message) => message.event_id)).toEqual([
      "$late-original"
    ]);

    snapshot = await api.submitSearch("Final", "allRooms");

    expect(snapshot.state.domain.search.kind).toBe("results");
    if (snapshot.state.domain.search.kind !== "results") {
      throw new Error("expected recovery search results");
    }
    expect(snapshot.state.domain.search.results.map((result) => result.event_id)).toEqual([
      "$late-original"
    ]);
  });

  test("browser fake lists saved sessions and switches account identity", async () => {
    const api = createBrowserFakeApi();

    const sessions = await api.listSavedSessions();

    expect(sessions.map((session) => session.user_id)).toEqual([
      "@demo-user:example.invalid",
      "@second-user:example.invalid"
    ]);

    const snapshot = await api.switchAccount(sessions[1]);

    expect(snapshot.state.domain.session.kind).toBe("ready");
    expect(snapshot.state.domain.session.user_id).toBe("@second-user:example.invalid");
    expect(snapshot.state.domain.session.device_id).toBe("SECONDDEVICE");
    expect(snapshot.state.domain.sync).toBe("running");
  });

  test("createRoom appends a non-DM room and makes it active", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();
    const beforeRoomCount = before.state.domain.rooms.length;

    const snapshot = await api.createRoom("New Test Room");

    expect(snapshot.state.domain.rooms).toHaveLength(beforeRoomCount + 1);
    const newRoom = snapshot.state.domain.rooms[snapshot.state.domain.rooms.length - 1];
    expect(newRoom?.display_name).toBe("New Test Room");
    expect(newRoom?.is_dm).toBe(false);
    expect(snapshot.state.ui.navigation.active_room_id).toBe(newRoom?.room_id);
  });

  test("createSpace appends a space and makes it active", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();
    const beforeSpaceCount = before.state.domain.spaces.length;

    const snapshot = await api.createSpace("New Test Space");

    expect(snapshot.state.domain.spaces).toHaveLength(beforeSpaceCount + 1);
    const newSpace = snapshot.state.domain.spaces[snapshot.state.domain.spaces.length - 1];
    expect(newSpace?.display_name).toBe("New Test Space");
    expect(snapshot.state.ui.navigation.active_space_id).toBe(newSpace?.space_id);
  });

  test("setSpaceChild links both directions", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();

    // Use the first space and a room not already in it
    const spaceId = before.state.domain.spaces[0]?.space_id;
    if (!spaceId) {
      throw new Error("expected at least one space");
    }

    // Find a room not in that space
    const spaceChildIds = before.state.domain.spaces[0]?.child_room_ids ?? [];
    const unlinkedRoom = before.state.domain.rooms.find(
      (room) => !spaceChildIds.includes(room.room_id) && !room.is_dm
    );
    if (!unlinkedRoom) {
      throw new Error("expected an unlinked non-DM room");
    }
    const childRoomId = unlinkedRoom.room_id;

    const snapshot = await api.setSpaceChild(spaceId, childRoomId, "fake.local");

    const updatedSpace = snapshot.state.domain.spaces.find((s) => s.space_id === spaceId);
    expect(updatedSpace?.child_room_ids).toContain(childRoomId);

    const updatedRoom = snapshot.state.domain.rooms.find((r) => r.room_id === childRoomId);
    expect(updatedRoom?.parent_space_ids).toContain(spaceId);
  });

  test("sendReply appends a reply message and increments the root reply_count", async () => {
    const api = createBrowserFakeApi();
    const initial = await api.getSnapshot();

    // Find an existing message to reply to in the active room
    const rootMessage = initial.timeline.find((m) => m.room_id === initial.state.ui.navigation.active_room_id);
    if (!rootMessage) {
      throw new Error("expected at least one timeline message in active room");
    }
    const rootEventId = rootMessage.event_id;
    const rootReplyCountBefore = rootMessage.reply_count;
    const roomId = rootMessage.room_id;

    const snapshot = await api.sendReply(roomId, rootEventId, "Synthetic reply message");

    // New reply appended at end with reply_count: 0
    const lastMessage = snapshot.timeline[snapshot.timeline.length - 1];
    expect(lastMessage?.body).toBe("Synthetic reply message");
    expect(lastMessage?.reply_count).toBe(0);
    expect(lastMessage?.room_id).toBe(roomId);

    // Root message reply_count incremented
    const updatedRoot = snapshot.timeline.find((m) => m.event_id === rootEventId);
    expect(updatedRoot?.reply_count).toBe(rootReplyCountBefore + 1);
  });
});

function roomSummary(
  roomId: string,
  displayName: string,
  isDm: boolean,
  tags: RoomTags = { favourite: null, low_priority: null },
  parentSpaceIds: string[] = [],
  lastActivityMs?: number
): RoomSummary {
  return {
    room_id: roomId,
    display_name: displayName,
    display_label: displayName,
    original_display_label: displayName,
    avatar: null,
    is_dm: isDm,
    dm_user_ids: isDm ? [`@${roomId.replace(/^!/, "")}`] : [],
    tags,
    parent_space_ids: parentSpaceIds,
    dm_space_ids: [],
    is_encrypted: false,
    unread_count: 0,
    last_activity_ms: lastActivityMs
  };
}

function roomSummaryWithActivity(
  roomId: string,
  displayName: string,
  isDm: boolean,
  lastActivityMs: number
): RoomSummary {
  return roomSummary(roomId, displayName, isDm, undefined, undefined, lastActivityMs);
}

function roomSummaryWithDmSpaces(
  roomId: string,
  displayName: string,
  dmSpaceIds: string[]
): RoomSummary {
  return {
    room_id: roomId,
    display_name: displayName,
    display_label: displayName,
    original_display_label: displayName,
    avatar: null,
    is_dm: true,
    dm_user_ids: [`@${roomId.replace(/^!/, "")}`],
    tags: { favourite: null, low_priority: null },
    parent_space_ids: [],
    dm_space_ids: dmSpaceIds,
    is_encrypted: false,
    unread_count: 0
  };
}
