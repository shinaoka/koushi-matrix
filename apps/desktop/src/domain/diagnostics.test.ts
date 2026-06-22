import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import {
  appendDiagnosticLogEntry,
  DEFAULT_DIAGNOSTIC_LOG_LIMIT,
  diagnosticReport,
  type DiagnosticLogEntry
} from "./diagnostics";

describe("diagnosticReport", () => {
  test("summarizes sync, timeline, and crawler progress without private identifiers or message bodies", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const report = diagnosticReport({
      snapshot: {
        ...snapshot,
        timeline: [
          {
            event_id: "$private-event:example.invalid",
            room_id: "!private-room:example.invalid",
            sender: "@alice:example.invalid",
            timestamp_ms: 1_800_000_000_000,
            body: "secret message body",
            attachment_filename: null,
            reply_count: 0
          }
        ],
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            search_crawler: {
              rooms: {
                "!queued-room:example.invalid": { kind: "queued" },
                "!private-room:example.invalid": {
                  kind: "running",
                  processed: 42,
                  indexed: 24
                }
              },
              last_active: {
                room_id: "!private-room:example.invalid",
                updated_at_ms: 1_800_000_000_000,
                status: "running",
                processed: 42,
                indexed: 24
              }
            }
          },
          ui: {
            ...snapshot.state.ui,
            thread: {
              kind: "open",
              room_id: "!private-room:example.invalid",
              root_event_id: "$private-event:example.invalid",
              is_subscribed: true,
              composer: {
                draft: "",
                pending_transaction_id: null,
                mode: "Plain"
              }
            },
            threads_list: {
              kind: "open",
              room_id: "!private-room:example.invalid",
              request_id: 7,
              items: [
                {
                  root_event_id: "$private-event:example.invalid",
                  root_sender: "@alice:example.invalid",
                  root_sender_label: "Alice",
                  root_body_preview: "secret thread body",
                  root_timestamp_ms: 1_800_000_000_000,
                  latest_event_id: "$private-reply:example.invalid",
                  latest_sender: "@alice:example.invalid",
                  latest_sender_label: "Alice",
                  latest_body_preview: "secret reply body",
                  latest_timestamp_ms: 1_800_000_001_000,
                  reply_count: 2
                }
              ],
              is_paginating: false,
              end_reached: true
            },
            errors: [
              {
                code: "timeline_subscription_failed",
                message: "SDK detail for !private-room:example.invalid",
                recoverable: true
              }
            ]
          }
        }
      },
      panelMode: "closed",
      sendStatus: "idle",
      timelineDiagnostics: {
        visibleItems: 3,
        downloadedItems: 7,
        backfill: "paginating",
        avatarMxcItems: 2,
        avatarReadyItems: 1,
        avatarPendingItems: 1,
        avatarFailedItems: 0,
        avatarMissingItems: 1,
        avatarRenderedImages: 1,
        avatarBrokenImages: 0
      },
      domDiagnostics: {
        screen: "timeline",
        rootChildren: 1,
        bodyTextLength: 99
      },
      uiLatencyDiagnostics: {
        samples: 8,
        lastFrameGapMs: 18,
        averageFrameGapMs: 26.5,
        maxFrameGapMs: 125,
        longFrameCount: 2
      }
    });

    expect(report).toContain("Koushi diagnostics");
    expect(report).toContain("Generated at:");
    expect(report).toContain(
      "Room classification: domain_dms=2 sidebar_dms=0 room_list_items=2 room_list_dm_items=0 active_filter=rooms"
    );
    expect(report).toContain("Timeline matches active room: true");
    expect(report).toContain("Timeline visible items: 3");
    expect(report).toContain(
      "Timeline avatars: mxc=2 ready=1 pending=1 failed=0 missing=1 rendered=1 broken=0"
    );
    expect(report).toContain(
      "Potential UI load: search crawler running=1 queued=1; worker=1"
    );
    expect(report).toContain(
      "Search crawler running=1 queued=1: processed=42 indexed=24"
    );
    expect(report).toContain("Potential UI lag: max frame gap 125 ms");
    expect(report).toContain("UI frame gap: last=18ms avg=26.5ms max=125ms longFrames=2 samples=8");
    expect(report).toContain("Thread panel: open subscribed=true");
    expect(report).toContain("Threads list: open items=1 paginating=false end=true");
    expect(report).toContain("ui_frame_max_ms=125");
    expect(report).toContain("timeline_matches_active=true");
    expect(report).toContain("Latest error code: timeline_subscription_failed");
    expect(report).not.toContain("secret message body");
    expect(report).not.toContain("secret thread body");
    expect(report).not.toContain("secret reply body");
    expect(report).not.toContain("!private-room");
    expect(report).not.toContain("@alice");
    expect(report).not.toContain("$private-event");
    expect(report).not.toContain("SDK detail");
  });

  test("includes Element-style timestamped diagnostic log entries in chronological order", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const logEntries: DiagnosticLogEntry[] = [
      {
        timestampMs: Date.parse("2026-06-20T06:03:02.000Z"),
        source: "timeline",
        message: "avatars ready=1 pending=2 failed=0 missing=3"
      },
      {
        timestampMs: Date.parse("2026-06-20T06:03:01.000Z"),
        source: "timeline",
        message: "items visible=12 downloaded=34 backfill=Idle"
      }
    ];

    const report = diagnosticReport({
      snapshot,
      panelMode: "closed",
      sendStatus: "idle",
      timelineDiagnostics: {
        visibleItems: 12,
        downloadedItems: 34,
        backfill: "Idle",
        avatarMxcItems: 3,
        avatarReadyItems: 1,
        avatarPendingItems: 2,
        avatarFailedItems: 0,
        avatarMissingItems: 3,
        avatarRenderedImages: 1,
        avatarBrokenImages: 0
      },
      domDiagnostics: {
        screen: "timeline",
        rootChildren: 1,
        bodyTextLength: 99
      },
      uiLatencyDiagnostics: {
        samples: 8,
        lastFrameGapMs: 18,
        averageFrameGapMs: 26.5,
        maxFrameGapMs: 30,
        longFrameCount: 0
      },
      logEntries
    });

    expect(report).toContain("Timeline log:");
    expect(report.indexOf("[2026-06-20T06:03:01.000Z] timeline items visible=12"))
      .toBeLessThan(report.indexOf("[2026-06-20T06:03:02.000Z] timeline avatars ready=1"));
    expect(report).not.toContain("!");
    expect(report).not.toContain("@");
    expect(report).not.toContain("$");
  });

  test("gates verbose security diagnostics behind the explicit diagnostic build flag", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const baseInput = {
      snapshot,
      panelMode: "closed" as const,
      sendStatus: "idle" as const,
      timelineDiagnostics: {
        visibleItems: 12,
        downloadedItems: 34,
        backfill: "Idle",
        avatarMxcItems: 3,
        avatarReadyItems: 1,
        avatarPendingItems: 2,
        avatarFailedItems: 0,
        avatarMissingItems: 3,
        avatarRenderedImages: 1,
        avatarBrokenImages: 0
      },
      domDiagnostics: {
        screen: "timeline",
        rootChildren: 1,
        bodyTextLength: 99
      },
      uiLatencyDiagnostics: {
        samples: 8,
        lastFrameGapMs: 18,
        averageFrameGapMs: 26.5,
        maxFrameGapMs: 30,
        longFrameCount: 0
      }
    };

    const normalReport = diagnosticReport({
      ...baseInput,
      verboseDiagnostics: {
        enabled: false,
        security: {
          secureContext: true,
          locationProtocol: "http:",
          locationOrigin: "http://localhost:5173",
          avatarImageSchemes: { asset: 3 },
          avatarBrokenImages: 1
        }
      }
    });
    const verboseReport = diagnosticReport({
      ...baseInput,
      verboseDiagnostics: {
        enabled: true,
        security: {
          secureContext: true,
          locationProtocol: "http:",
          locationOrigin: "http://localhost:5173",
          avatarImageSchemes: { asset: 3, data: 1 },
          avatarBrokenImages: 1
        }
      }
    });

    expect(normalReport).toContain("Verbose diagnostics: disabled");
    expect(normalReport).not.toContain("Security diagnostics:");
    expect(verboseReport).toContain("Verbose diagnostics: enabled");
    expect(verboseReport).toContain("Security diagnostics:");
    expect(verboseReport).toContain("security.secure_context=true");
    expect(verboseReport).toContain("security.location_protocol=http:");
    expect(verboseReport).toContain("security.location_origin=http://localhost:5173");
    expect(verboseReport).toContain("security.avatar_src_schemes=asset:3,data:1");
    expect(verboseReport).toContain("security.avatar_broken_images=1");
  });

  test("includes state-transport delta health tokens when provided", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const report = diagnosticReport({
      snapshot,
      panelMode: "closed",
      sendStatus: "idle",
      timelineDiagnostics: {
        visibleItems: 0,
        downloadedItems: 0,
        backfill: "Idle",
        avatarMxcItems: 0,
        avatarReadyItems: 0,
        avatarPendingItems: 0,
        avatarFailedItems: 0,
        avatarMissingItems: 0,
        avatarRenderedImages: 0,
        avatarBrokenImages: 0
      },
      domDiagnostics: {
        screen: "timeline",
        rootChildren: 1,
        bodyTextLength: 99
      },
      uiLatencyDiagnostics: {
        samples: 8,
        lastFrameGapMs: 18,
        averageFrameGapMs: 26.5,
        maxFrameGapMs: 30,
        longFrameCount: 0
      },
      stateDeltaStats: { applied: 12, staleIgnored: 340, gapRefreshRequested: 2 },
      timelineTransportStats: {
        received: 9,
        keyMismatchDropped: 9,
        initialItemsApplied: 0,
        lastInitialItemsCount: 0
      }
    });

    expect(report).toContain(
      "State transport: delta_applied=12 stale_ignored=340 gap_refresh=2"
    );
    expect(report).toContain("state_delta_applied=12");
    expect(report).toContain("state_delta_stale_ignored=340");
    expect(report).toContain("state_delta_gap_refresh=2");
    expect(report).toContain(
      "Timeline transport: received=9 key_dropped=9 initial_applied=0 last_initial_items=0"
    );
    expect(report).toContain("timeline_evt_received=9");
    expect(report).toContain("timeline_evt_key_dropped=9");
    expect(report).toContain("timeline_initial_applied=0");
    expect(report).toContain("timeline_last_initial_items=0");
  });

  test("renders captured JS errors and a count token when provided", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    const report = diagnosticReport({
      snapshot,
      panelMode: "closed",
      sendStatus: "idle",
      timelineDiagnostics: {
        visibleItems: 0,
        downloadedItems: 0,
        backfill: "Idle",
        avatarMxcItems: 0,
        avatarReadyItems: 0,
        avatarPendingItems: 0,
        avatarFailedItems: 0,
        avatarMissingItems: 0,
        avatarRenderedImages: 0,
        avatarBrokenImages: 0
      },
      domDiagnostics: { screen: "timeline", rootChildren: 1, bodyTextLength: 99 },
      uiLatencyDiagnostics: {
        samples: 8,
        lastFrameGapMs: 18,
        averageFrameGapMs: 26.5,
        maxFrameGapMs: 30,
        longFrameCount: 0
      },
      jsErrors: [
        { kind: "TypeError", message: "cannot read kind of undefined", source: "App.tsx:12:3" }
      ]
    });

    expect(report).toContain("JS errors: 1");
    expect(report).toContain(
      "[js-error] kind=TypeError source=App.tsx:12:3 message=cannot read kind of undefined"
    );
    expect(report).toContain("js_error_count=1");
  });

  test("bounds diagnostic log entries while preserving chronological append order", () => {
    expect(DEFAULT_DIAGNOSTIC_LOG_LIMIT).toBeGreaterThanOrEqual(10_000);

    const entries = appendDiagnosticLogEntry(
      [
        { timestampMs: 1, source: "timeline", message: "old" },
        { timestampMs: 2, source: "timeline", message: "middle" }
      ],
      { timestampMs: 3, source: "timeline", message: "new" },
      2
    );

    expect(entries).toEqual([
      { timestampMs: 2, source: "timeline", message: "middle" },
      { timestampMs: 3, source: "timeline", message: "new" }
    ]);
  });
});
