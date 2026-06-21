import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import {
  qaDomDiagnosticTokens,
  qaSearchCrawlerDiagnosticTokens,
  qaTimelineDiagnosticTokens,
  qaUiLatencyDiagnosticTokens,
  qaWindowTitle
} from "./qaTitle";

describe("qaWindowTitle", () => {
  test("summarizes session, sync, room, and timeline state without private names", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const title = qaWindowTitle(snapshot);

    expect(title).toContain("koushi-desktop qa");
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
        ui: {
          ...snapshot.state.ui,
          timeline: {
            ...snapshot.state.ui.timeline,
            room_id: null
          }
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
        domain: {
          ...snapshot.state.domain,
          rooms: snapshot.state.domain.rooms.map((room) => ({
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
        ui: {
          ...snapshot.state.ui,
          focused_context: {
            kind: "open",
            room_id: "!private-room:example.test",
            event_id: "$private-event:example.test",
            is_subscribed: true
          }
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
        domain: {
          ...snapshot.state.domain,
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

  test("includes optional private-data-free diagnostic tokens", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const title = qaWindowTitle(snapshot, "closed", "pending", [
      "target_dm=encrypted",
      "target_selected=true",
      "target_members=2"
    ]);

    expect(title).toContain("target_dm=encrypted");
    expect(title).toContain("target_selected=true");
    expect(title).toContain("target_members=2");
    expect(title).not.toContain("@");
    expect(title).not.toContain("!");
  });

  test("includes the local send QA statuses when provided", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const idleTitle = qaWindowTitle(snapshot, "closed", "idle");
    const pendingTitle = qaWindowTitle(snapshot, "closed", "pending");

    expect(idleTitle).toContain("send=idle");
    expect(pendingTitle).toContain("send=pending");
  });

  test("includes only a coarse latest error code for QA diagnostics", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const title = qaWindowTitle({
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          errors: [
            {
              code: "send_text_failed",
              message: "private room or SDK detail",
              recoverable: true
            }
          ]
        }
      }
    });

    expect(title).toContain("errors=1");
    expect(title).toContain("error_code=send_text_failed");
    expect(title).not.toContain("private room");
    expect(title).not.toContain("SDK detail");
  });

  test("summarizes search crawler progress without room identifiers", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const tokens = qaSearchCrawlerDiagnosticTokens({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          search_crawler: {
            rooms: {
              "!private-a:example.test": { kind: "running", processed: 100, indexed: 40 },
              "!private-q:example.test": { kind: "queued" },
              "!private-b:example.test": { kind: "completed", indexed: 25 },
              "!private-c:example.test": { kind: "failed", failureKind: "sdk" }
            }
          }
        }
      }
    });

    expect(tokens).toEqual([
      "crawler_running=1",
      "crawler_queued=1",
      "crawler_completed=1",
      "crawler_failed=1",
      "crawler_processed=100",
      "crawler_indexed=65"
    ]);
    expect(tokens.join(" ")).not.toContain("private");
    expect(tokens.join(" ")).not.toContain("!");
  });

  test("summarizes event-driven timeline diagnostics without event identifiers", () => {
    const tokens = qaTimelineDiagnosticTokens({
      visibleItems: 12,
      downloadedItems: 34,
      backfill: "Paginating",
      avatarMxcItems: 8,
      avatarReadyItems: 5,
      avatarPendingItems: 2,
      avatarFailedItems: 1,
      avatarMissingItems: 4,
      avatarRenderedImages: 3,
      avatarBrokenImages: 1
    });

    expect(tokens).toEqual([
      "timeline_visible=12",
      "timeline_dl=34",
      "timeline_backfill=Paginating",
      "timeline_avatar_mxc=8",
      "timeline_avatar_ready=5",
      "timeline_avatar_pending=2",
      "timeline_avatar_failed=1",
      "timeline_avatar_missing=4",
      "timeline_avatar_rendered=3",
      "timeline_avatar_broken=1"
    ]);
  });

  test("summarizes rendered DOM diagnostics without private content", () => {
    const tokens = qaDomDiagnosticTokens({
      screen: "auth",
      rootChildren: 1,
      bodyTextLength: 42
    });

    expect(tokens).toEqual([
      "dom_screen=auth",
      "dom_root_children=1",
      "dom_text_len=42"
    ]);
  });

  test("summarizes UI latency diagnostics as coarse numeric tokens", () => {
    expect(
      qaUiLatencyDiagnosticTokens({
        samples: 12,
        lastFrameGapMs: 17.24,
        averageFrameGapMs: 22.65,
        maxFrameGapMs: 140.04,
        longFrameCount: 3
      })
    ).toEqual([
      "ui_frame_samples=12",
      "ui_frame_last_ms=17.2",
      "ui_frame_avg_ms=22.7",
      "ui_frame_max_ms=140",
      "ui_long_frames=3"
    ]);
  });
});
