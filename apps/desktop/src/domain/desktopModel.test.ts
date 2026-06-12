import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { composeSidebar, visibleRooms } from "./desktopModel";
import type { RoomSummary, SpaceSummary } from "./types";

describe("desktop model", () => {
  test("space rooms exclude DMs while DMs stay global", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-beta:example.invalid");

    const rooms = visibleRooms(snapshot);

    expect(rooms.spaceRooms.map((room) => room.room_id)).toEqual([
      "!room-search:example.invalid"
    ]);
    expect(rooms.globalDms.map((room) => room.room_id)).toContain(
      "!dm-member-1:example.invalid"
    );
    expect(rooms.spaceRooms.every((room) => !room.room_id.startsWith("!dm-"))).toBe(
      true
    );
  });

  test("account home lists all non-DM rooms while DMs stay global", () => {
    const spaces: SpaceSummary[] = [
      {
        space_id: "!space-a:example.invalid",
        display_name: "Alpha",
        child_room_ids: ["!room-a:example.invalid", "!dm-a:example.invalid"]
      }
    ];
    const rooms: RoomSummary[] = [
      {
        room_id: "!room-a:example.invalid",
        display_name: "Alpha room",
        is_dm: false,
        parent_space_ids: ["!space-a:example.invalid"],
        unread_count: 5
      },
      {
        room_id: "!global-room:example.invalid",
        display_name: "Global room",
        is_dm: false,
        parent_space_ids: [],
        unread_count: 2
      },
      {
        room_id: "!dm-a:example.invalid",
        display_name: "Direct chat",
        is_dm: true,
        parent_space_ids: ["!space-a:example.invalid"],
        unread_count: 3
      }
    ];

    const sidebar = composeSidebar(null, spaces, rooms);

    expect(sidebar.account_home).toMatchObject({
      display_name: "Home",
      unread_count: 7,
      is_active: true
    });
    expect(sidebar.space_rooms.map((room) => room.room_id)).toEqual([
      "!room-a:example.invalid",
      "!global-room:example.invalid"
    ]);
    expect(sidebar.global_dms.map((room) => room.room_id)).toEqual([
      "!dm-a:example.invalid"
    ]);
  });

  test("fake search keeps exact matches and drops ngram false positives", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("Alpha", "allRooms");

    const results =
      snapshot.state.search.kind === "results" ? snapshot.state.search.results : [];

    expect(results.map((result) => result.event_id)).toEqual(["$alpha-update"]);
    expect(results[0]?.match_field).toBe("messageBody");
    expect(results[0]?.highlights).toEqual([{ start_utf16: 0, end_utf16: 5 }]);
  });

  test("browser fake backward pagination prepends older timeline messages", async () => {
    const api = createBrowserFakeApi();
    const before = await api.getSnapshot();
    const beforeEventIds = before.timeline.map((message) => message.event_id);

    expect(beforeEventIds).not.toContain("$alpha-history");

    const snapshot = await api.paginateTimelineBackwards("!room-alpha:example.invalid");
    const afterEventIds = snapshot.timeline.map((message) => message.event_id);

    expect(snapshot.state.timeline.is_paginating_backwards).toBe(false);
    expect(afterEventIds[0]).toBe("$alpha-history");
    expect(afterEventIds[1]).toBe(beforeEventIds[0]);
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
      snapshot.state.search.kind === "results" ? snapshot.state.search.results : [];

    expect(results).toHaveLength(1);
    expect(results[0]?.event_id).toBe("$budget-file");
    expect(results[0]?.match_field).toBe("attachmentFileName");
  });

  test("browser fake can start signed out and exposes the pre-login boundary", async () => {
    const api = createBrowserFakeApi({ restoreSession: false });

    let snapshot = await api.getSnapshot();

    expect(snapshot.state.session.kind).toBe("signedOut");
    expect(snapshot.state.rooms).toHaveLength(0);
    expect(snapshot.state.errors).toHaveLength(0);

    snapshot = await api.submitLogin(
      "https://matrix.example.org",
      "demo-user",
      "synthetic-password",
      "Matrix Desktop Test"
    );

    expect(snapshot.state.session.kind).toBe("signedOut");
    expect(snapshot.state.rooms).toHaveLength(0);
    expect(snapshot.state.errors).toHaveLength(1);
    expect(snapshot.state.errors[0]?.code).toBe("login_failed");
    expect(JSON.stringify(snapshot)).not.toContain("synthetic-password");
  });

  test("browser fake discovers password and sso login methods", async () => {
    const api = createBrowserFakeApi({ restoreSession: false });

    const snapshot = await api.discoverLoginMethods("matrix.example.org:8448");

    expect(snapshot.state.auth.kind).toBe("ready");
    if (snapshot.state.auth.kind !== "ready") {
      throw new Error("expected discovered login methods");
    }

    expect(snapshot.state.auth.homeserver).toBe("https://matrix.example.org:8448");
    expect(snapshot.state.auth.flows.map((flow) => flow.kind)).toEqual([
      "password",
      "sso"
    ]);
    expect(snapshot.state.auth.flows[1]?.delegated_oidc_compatibility).toBe(true);
  });

  test("browser fake can expose a post-login e2ee recovery step", async () => {
    const api = createBrowserFakeApi({ session: "needsRecovery" });

    let snapshot = await api.getSnapshot();

    expect(snapshot.state.session.kind).toBe("needsRecovery");
    expect(snapshot.state.rooms.length).toBeGreaterThan(0);
    expect(snapshot.timeline.length).toBeGreaterThan(0);
    expect(snapshot.state.navigation.active_room_id).toBeTruthy();
    expect(snapshot.state.sync).toBe("running");
    expect(snapshot.state.session.recovery_methods).toEqual([
      "recoveryKey",
      "securityPhrase"
    ]);

    snapshot = await api.submitRecovery("synthetic-recovery-secret");

    expect(snapshot.state.session.kind).toBe("ready");
    expect(snapshot.state.sync).toBe("running");
    expect(JSON.stringify(snapshot)).not.toContain("synthetic-recovery-secret");
  });

  test("browser fake keeps synced room navigation and search available during recovery", async () => {
    const api = createBrowserFakeApi({ session: "needsRecovery" });

    let snapshot = await api.selectRoom("!room-planning:example.invalid");

    expect(snapshot.state.session.kind).toBe("needsRecovery");
    expect(snapshot.state.navigation.active_room_id).toBe("!room-planning:example.invalid");
    expect(snapshot.timeline.map((message) => message.event_id)).toEqual([
      "$late-original"
    ]);

    snapshot = await api.submitSearch("Final", "allRooms");

    expect(snapshot.state.search.kind).toBe("results");
    if (snapshot.state.search.kind !== "results") {
      throw new Error("expected recovery search results");
    }
    expect(snapshot.state.search.results.map((result) => result.event_id)).toEqual([
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

    expect(snapshot.state.session.kind).toBe("ready");
    expect(snapshot.state.session.user_id).toBe("@second-user:example.invalid");
    expect(snapshot.state.session.device_id).toBe("SECONDDEVICE");
    expect(snapshot.state.sync).toBe("running");
  });
});
