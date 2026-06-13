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
    expect(title).toContain("unread=");
    expect(title).toContain("badge=");
    expect(title).toContain("notify=");
    expect(title).not.toContain("Alpha");
    expect(title).not.toContain("@");
    expect(title).not.toContain("!");
    expect(title).not.toContain("$");
  });

  test("includes an optional panel token when provided", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const title = qaWindowTitle(snapshot, "keyboardSettings");

    expect(title).toContain("panel=keyboardSettings");
  });

  test("includes an optional send smoke status token when provided", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const title = qaWindowTitle(snapshot, "closed", "sent");

    expect(title).toContain("panel=closed");
    expect(title).toContain("send=sent");
  });

  test("includes the local send QA statuses when provided", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const idleTitle = qaWindowTitle(snapshot, "closed", "idle");
    const pendingTitle = qaWindowTitle(snapshot, "closed", "pending");

    expect(idleTitle).toContain("send=idle");
    expect(pendingTitle).toContain("send=pending");
  });
});
