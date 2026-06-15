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
    expect(title).toContain("timeline_room=true");
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

  test("distinguishes active navigation from an opened timeline room", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const title = qaWindowTitle({
      ...snapshot,
      state: {
        ...snapshot.state,
        timeline: {
          ...snapshot.state.timeline,
          room_id: null
        }
      }
    });

    expect(title).toContain("active_room=true");
    expect(title).toContain("timeline_room=false");
  });

  test("uses Rust-owned native attention tokens instead of room-list aggregation", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const title = qaWindowTitle({
      ...snapshot,
      state: {
        ...snapshot.state,
        rooms: snapshot.state.rooms.map((room) => ({
          ...room,
          unread_count: 99,
          notification_count: 99,
          highlight_count: 99
        })),
        native_attention: {
          summary: {
            unread_count: 2,
            highlight_count: 1,
            badge_count: 2,
            candidate: {
              room_display_name: "Hidden QA Room",
              kind: "mention",
              unread_count: 2,
              highlight_count: 1
            },
            capabilities: {
              notifications: "available",
              badge: "available",
              sound: "unknown",
              tray: "unknown",
              activation: "unknown"
            }
          },
          dispatch: { kind: "idle" }
        }
      }
    });

    expect(title).toContain("unread=2 badge=2 notify=mention");
    expect(title).not.toContain("unread=99");
    expect(title).not.toContain("Hidden QA Room");
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
