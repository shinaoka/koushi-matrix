import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { visibleRooms } from "./desktopModel";

describe("desktop model", () => {
  test("space rooms exclude DMs while DMs stay global", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace("!space-lab:example.org");

    const rooms = visibleRooms(snapshot);

    expect(rooms.spaceRooms.map((room) => room.room_id)).toEqual([
      "!search-dev:example.org"
    ]);
    expect(rooms.globalDms.map((room) => room.room_id)).toContain(
      "!dm-akio:example.org"
    );
    expect(rooms.spaceRooms.every((room) => !room.room_id.startsWith("!dm-"))).toBe(
      true
    );
  });

  test("fake search keeps exact matches and drops ngram false positives", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("Zoom", "allRooms");

    const results =
      snapshot.state.search.kind === "results" ? snapshot.state.search.results : [];

    expect(results.map((result) => result.event_id)).toEqual(["$zoom-invite"]);
    expect(results[0]?.match_field).toBe("messageBody");
    expect(results[0]?.highlights).toEqual([{ start_utf16: 33, end_utf16: 37 }]);
  });

  test("fake search includes attachment filenames as a separate match field", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("seminar_budget.xlsx", "allRooms");

    const results =
      snapshot.state.search.kind === "results" ? snapshot.state.search.results : [];

    expect(results).toHaveLength(1);
    expect(results[0]?.event_id).toBe("$budget-file");
    expect(results[0]?.match_field).toBe("attachmentFileName");
  });
});
