import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { qaWindowTitle } from "./qaTitle";

describe("qaWindowTitle", () => {
  test("summarizes session, sync, room, and timeline state without private names", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const title = qaWindowTitle(snapshot);

    expect(title).toContain("matrix-desktop qa");
    expect(title).toContain("session=ready");
    expect(title).toContain("sync=running");
    expect(title).toContain("rooms=");
    expect(title).toContain("active_room=true");
    expect(title).toContain("timeline_subscribed=true");
    expect(title).toContain("timeline_items=");
    expect(title).not.toContain("Alpha");
    expect(title).not.toContain("@");
    expect(title).not.toContain("!");
    expect(title).not.toContain("$");
  });
});
