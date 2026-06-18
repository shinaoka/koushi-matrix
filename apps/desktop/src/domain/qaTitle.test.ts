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
    expect(title).toContain("pinned=");
    expect(title).toContain("pin_ops=");
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
              overlay_icon: "unknown",
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

  test("includes focused context state without room or event identifiers", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const title = qaWindowTitle({
      ...snapshot,
      state: {
        ...snapshot.state,
        focused_context: {
          kind: "open",
          room_id: "!private-room:example.test",
          event_id: "$private-event:example.test",
          is_subscribed: true
        }
      }
    });

    expect(title).toContain("focused=open");
    expect(title).not.toContain("private-room");
    expect(title).not.toContain("private-event");
  });

  test("summarizes pinned state as counts without identifiers or bodies", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const title = qaWindowTitle({
      ...snapshot,
      state: {
        ...snapshot.state,
        room_interactions: {
          "!private-room:example.test": {
            pinned_events: [
              {
                event_id: "$private-event:example.test",
                sender: "@private-user:example.test",
                body_preview: "private body",
                redacted: false
              }
            ],
            pin_operation: {
              kind: "pending",
              request_id: 1,
              room_id: "!private-room:example.test",
              event_id: "$private-event:example.test",
              op: "pin"
            }
          }
        }
      }
    });

    expect(title).toContain("pinned=1");
    expect(title).toContain("pin_ops=1");
    expect(title).not.toContain("private-room");
    expect(title).not.toContain("private-event");
    expect(title).not.toContain("private-user");
    expect(title).not.toContain("private body");
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
