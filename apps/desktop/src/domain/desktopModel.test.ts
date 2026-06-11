import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { visibleRooms } from "./desktopModel";

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

  test("fake search keeps exact matches and drops ngram false positives", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("Alpha", "allRooms");

    const results =
      snapshot.state.search.kind === "results" ? snapshot.state.search.results : [];

    expect(results.map((result) => result.event_id)).toEqual(["$alpha-update"]);
    expect(results[0]?.match_field).toBe("messageBody");
    expect(results[0]?.highlights).toEqual([{ start_utf16: 0, end_utf16: 5 }]);
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

    snapshot = await api.submitLogin("https://matrix.example.org", "demo-user");

    expect(snapshot.state.session.kind).toBe("signedOut");
    expect(snapshot.state.rooms).toHaveLength(0);
    expect(snapshot.state.errors).toHaveLength(1);
    expect(snapshot.state.errors[0]?.code).toBe("login_failed");
  });
});
