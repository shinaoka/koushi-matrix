import { describe, expect, test } from "vitest";
import { COMPOSER_DRAFT_REVISION_ZERO } from "./composerDraftRevision";

import { createDesktopApiFixture } from "../test/desktopApiFixture";
import {
  createDiagnosticLogBuffer,
  DEFAULT_DIAGNOSTIC_LOG_LIMIT,
  diagnosticReport,
  diagnosticReportPreview,
  schemaMismatchDiagnosticEntry,
  type DiagnosticLogEntry
} from "./diagnostics";

test("creates a fixed private-data-free schema mismatch diagnostic", () => {
  expect(schemaMismatchDiagnosticEntry(42)).toEqual({
    timestampMs: 42,
    source: "snapshot",
    message: "schema_mismatch"
  });
});

test("bounds the rendered diagnostic preview while preserving the report tail", () => {
  const report = `Koushi diagnostics\n${"old\n".repeat(100)}terminal_failure`;
  const preview = diagnosticReportPreview(report, 80);

  expect(preview.length).toBeLessThan(report.length);
  expect(preview).toContain("Koushi diagnostics");
  expect(preview).toContain("preview_truncated=true");
  expect(preview).toContain("terminal_failure");
});

describe("diagnosticReport", () => {
  test("renders the Rust-owned Sliding Sync snapshot in deterministic private-safe order", async () => {
    const api = createDesktopApiFixture();
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
      domDiagnostics: { screen: "timeline", rootChildren: 1, bodyTextLength: 0 },
      uiLatencyDiagnostics: {
        samples: 0,
        lastFrameGapMs: 0,
        averageFrameGapMs: 0,
        maxFrameGapMs: 0,
        longFrameCount: 0
      },
      slidingSyncDiagnostics: {
        discoveryState: "supported",
        advertised: true,
        discoverySource: "versions",
        lastProbeAgeBucket: "<1m",
        lastHttpStatusClass: "success",
        requestSchema: "element_x_all_rooms",
        engine: "SyncService",
        sdkSlidingSyncVersion: "native",
        roomListSharePos: true,
        encryptionSharePos: false,
        encryptionConnectionProfile: "sdk_default_encryption",
        encryptionExtensionProfile: "e2ee_to_device",
        provisionalEncryptionStarted: true,
        provisionalFirstResponseSeen: false,
        provisionalStoppedBeforeFirstResponse: true,
        provisionalToNormalHandoffBucket: "under100_milliseconds",
        lifecycle: "reconnecting",
        connectivityProven: false,
        committedGeneration: 0,
        lastSuccessAgeBucket: "never",
        consecutiveFailureCount: 29,
        lastFailureOrigin: "room_list",
        lastFailureKind: "sync_failed_http",
        lastFailureStage: "room_list_sliding_sync",
        lastHttpErrorSource: "server_response",
        lastHttpStatus: "unauthorized",
        lastMatrixErrorKind: "unknown_token",
        lastFailureRetryability: "permanent",
        roomListTaskRunning: false,
        encryptionTaskRunning: false,
        posPresent: false,
        directAccountDataSource: "sliding_sync_event",
        directMappedRoomCount: 4,
        directTargetCount: 5,
        projectedDmCount: 2,
        explicitDmCount: 1,
        fallbackDmCount: 1,
        directNonDmCount: 7,
        directInvalidEntryCount: 0,
        directEventWakeCount: 1,
        directEventAppliedCount: 1,
        directEventStreamRunning: true
      }
    });

    const expected = [
      "Sliding Sync:",
      "sliding_sync.discovery_state=supported",
      "sliding_sync.advertised=true",
      "sliding_sync.discovery_source=versions",
      "sliding_sync.last_probe_age_bucket=<1m",
      "sliding_sync.last_http_status_class=success",
      "sliding_sync.request_schema=element_x_all_rooms",
      "sync.engine=SyncService",
      "sync.sdk_sliding_sync_version=native",
      "sync.room_list_share_pos=true",
      "sync.encryption_share_pos=false",
      "sync.encryption_connection_profile=sdk_default_encryption",
      "sync.encryption_extension_profile=e2ee_to_device",
      "sync.provisional_encryption_started=true",
      "sync.provisional_first_response_seen=false",
      "sync.provisional_stopped_before_first_response=true",
      "sync.provisional_to_normal_handoff_bucket=under100_milliseconds",
      "sync.lifecycle=reconnecting",
      "sync.connectivity_proven=false",
      "sync.committed_generation=0",
      "sync.last_success_age_bucket=never",
      "sync.consecutive_failure_count=29",
      "sync.last_failure_origin=room_list",
      "sync.last_failure_kind=sync_failed_http",
      "sync.last_failure_stage=room_list_sliding_sync",
      "sync.last_http_error_source=server_response",
      "sync.last_http_status=unauthorized",
      "sync.last_matrix_error_kind=unknown_token",
      "sync.last_failure_retryability=permanent",
      "sync.room_list_task_running=false",
      "sync.encryption_task_running=false",
      "sync.pos_present=false",
      "direct_classification.source=sliding_sync_event",
      "direct_classification.mapped_room_count=4",
      "direct_classification.target_count=5",
      "direct_classification.projected_dm_count=2",
      "direct_classification.explicit_dm_count=1",
      "direct_classification.fallback_dm_count=1",
      "direct_classification.non_dm_count=7",
      "direct_classification.invalid_entry_count=0",
      "direct_classification.event_wake_count=1",
      "direct_classification.event_applied_count=1",
      "direct_classification.event_stream_running=true"
    ].join("\n");
    expect(report).toContain(expected);
    expect(report).not.toContain("https://");
    expect(report).not.toContain("@alice:");
  });

  test("summarizes sync, timeline, and crawler progress without private identifiers or message bodies", async () => {
    const api = createDesktopApiFixture();
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
              intent: "existingThread",
              is_subscribed: true,
              composer: {
                accepted_submission_ids: [],
                draft: "",
                document: { version: 2, inlines: [] },
                pending_transaction_id: null,
                draft_revision: COMPOSER_DRAFT_REVISION_ZERO,
                last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
                mode: "Plain"
              },
              staged_uploads: []
            },
            threads_list: {
              kind: "open",
              room_id: "!private-room:example.invalid",
              request_id: 7,
              items: [
                {
                  room_id: "!private-room:example.invalid",
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
      "Room classification: domain_dms=1 sidebar_dms=1 room_list_items=1 room_list_dm_items=0 active_filter=rooms"
    );
    expect(report).toContain("Timeline matches active room: true");
    expect(report).toContain("Timeline visible items: 3");
    expect(report).toContain(
      "Timeline avatars (item counts, not downloads; pending includes unrequested): mxc=2 ready=1 pending=1 failed=0 missing=1 rendered=1 broken=0"
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

  test("merges frontend and runtime diagnostic records chronologically without mutating inputs", async () => {
    const api = createDesktopApiFixture();
    const snapshot = await api.getSnapshot();
    const frontendEntries: DiagnosticLogEntry[] = [
      {
        timestampMs: Date.parse("2026-06-20T06:03:02.000Z"),
        source: "frontend.timeline",
        message: "avatars ready=1 pending=2 failed=0 missing=3"
      }
    ];
    const runtimeEntries: DiagnosticLogEntry[] = [
      {
        timestampMs: Date.parse("2026-06-20T06:03:01.000Z"),
        source: "core.timeline",
        message: "items visible=12 downloaded=34 backfill=Idle"
      },
      {
        timestampMs: Date.parse("2026-06-20T06:03:03.000Z"),
        source: "core.timeline",
        message: "avatars ready=2 pending=1 failed=0 missing=2"
      }
    ];
    const mergedEntries = [...frontendEntries, ...runtimeEntries];

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
      logEntries: mergedEntries,
      droppedLogEntries: 7
    });

    expect(report).toContain("Diagnostic log:");
    expect(report).toContain("Diagnostic records dropped: 7");
    expect(report.indexOf("[2026-06-20T06:03:01.000Z] core.timeline items visible=12"))
      .toBeLessThan(report.indexOf("[2026-06-20T06:03:02.000Z] frontend.timeline avatars ready=1"));
    expect(report.indexOf("[2026-06-20T06:03:02.000Z] frontend.timeline avatars ready=1"))
      .toBeLessThan(report.indexOf("[2026-06-20T06:03:03.000Z] core.timeline avatars ready=2"));
    expect(frontendEntries).toEqual([
      {
        timestampMs: Date.parse("2026-06-20T06:03:02.000Z"),
        source: "frontend.timeline",
        message: "avatars ready=1 pending=2 failed=0 missing=3"
      }
    ]);
    expect(runtimeEntries).toEqual([
      {
        timestampMs: Date.parse("2026-06-20T06:03:01.000Z"),
        source: "core.timeline",
        message: "items visible=12 downloaded=34 backfill=Idle"
      },
      {
        timestampMs: Date.parse("2026-06-20T06:03:03.000Z"),
        source: "core.timeline",
        message: "avatars ready=2 pending=1 failed=0 missing=2"
      }
    ]);
    expect(mergedEntries).toEqual([...frontendEntries, ...runtimeEntries]);
    expect(report).not.toContain("!");
    expect(report).not.toContain("@");
    expect(report).not.toContain("$");
  });

  test("always includes supplied security diagnostics without a build flag", async () => {
    const api = createDesktopApiFixture();
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

    const report = diagnosticReport({
      ...baseInput,
      securityDiagnostics: {
        secureContext: true,
        locationProtocol: "http:",
        locationOrigin: "http://localhost:5173",
        avatarImageSchemes: { asset: 3, data: 1 },
        avatarBrokenImages: 1
      }
    });

    expect(report).toContain("Security diagnostics:");
    expect(report).toContain("security.secure_context=true");
    expect(report).toContain("security.location_protocol=http:");
    expect(report).toContain("security.location_origin=http://localhost:5173");
    expect(report).toContain("security.avatar_src_schemes=asset:3,data:1");
    expect(report).toContain("security.avatar_broken_images=1");
    expect(report).not.toContain("Verbose diagnostics:");
  });

  test("normalizes invalid dropped diagnostic counts", async () => {
    const api = createDesktopApiFixture();
    const snapshot = await api.getSnapshot();
    const baseInput = {
      snapshot,
      panelMode: "closed" as const,
      sendStatus: "idle" as const,
      timelineDiagnostics: {
        visibleItems: 0,
        downloadedItems: 0,
        backfill: "Idle" as const,
        avatarMxcItems: 0,
        avatarReadyItems: 0,
        avatarPendingItems: 0,
        avatarFailedItems: 0,
        avatarMissingItems: 0,
        avatarRenderedImages: 0,
        avatarBrokenImages: 0
      },
      domDiagnostics: { screen: "timeline", rootChildren: 1, bodyTextLength: 0 },
      uiLatencyDiagnostics: {
        samples: 0,
        lastFrameGapMs: 0,
        averageFrameGapMs: 0,
        maxFrameGapMs: 0,
        longFrameCount: 0
      },
    };

    expect(diagnosticReport({ ...baseInput, droppedLogEntries: -2.8 })).toContain(
      "Diagnostic records dropped: 0"
    );
    expect(diagnosticReport({ ...baseInput, droppedLogEntries: Number.NaN })).toContain(
      "Diagnostic records dropped: 0"
    );
    expect(diagnosticReport({ ...baseInput, droppedLogEntries: 4.9 })).toContain(
      "Diagnostic records dropped: 4"
    );
  });

  test("includes state-transport delta health tokens when provided", async () => {
    const api = createDesktopApiFixture();
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
        keyMismatchGroups: { "room:thread:account_match:room_match": 9 },
        initialItemsApplied: 0,
        lastInitialItemsCount: 0,
        resync: 3
      }
    });

    expect(report).toContain(
      "State transport: delta_applied=12 stale_ignored=340 gap_refresh=2"
    );
    expect(report).toContain("state_delta_applied=12");
    expect(report).toContain("state_delta_stale_ignored=340");
    expect(report).toContain("state_delta_gap_refresh=2");
    expect(report).toContain(
      "Timeline transport: received=9 key_dropped=9 mismatch_groups=room:thread:account_match:room_match:9 initial_applied=0 last_initial_items=0 resync=3"
    );
    expect(report).toContain("timeline_evt_received=9");
    expect(report).toContain("timeline_evt_key_dropped=9");
    expect(report).toContain("timeline_initial_applied=0");
    expect(report).toContain("timeline_last_initial_items=0");
    expect(report).toContain("timeline_resync=3");
  });

  test("renders only coarse captured JS error kinds and channels with a count token", async () => {
    const api = createDesktopApiFixture();
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
        {
          kind: "type_error",
          channel: "window_error",
          ageBucket: "<1m",
          fingerprint: "f1_0123abcd"
        },
        {
          kind: "unknown",
          channel: "unhandled_rejection",
          ageBucket: "1m-5m",
          fingerprint: "f1_deadbeef"
        }
      ]
    });

    expect(report).toContain("JS errors: 2");
    expect(report).toContain(
      "[js-error] channel=window_error kind=type_error age_bucket=<1m fingerprint=f1_0123abcd"
    );
    expect(report).toContain(
      "[js-error] channel=unhandled_rejection kind=unknown age_bucket=1m-5m fingerprint=f1_deadbeef"
    );
    expect(report).toContain("js_error_count=2");
    expect(report).not.toContain("source=");
    expect(report).not.toContain("message=");
  });

  test("bounds diagnostic log entries while preserving chronological append order", () => {
    expect(DEFAULT_DIAGNOSTIC_LOG_LIMIT).toBeGreaterThanOrEqual(10_000);
    const buffer = createDiagnosticLogBuffer(2);
    buffer.append({ timestampMs: 1, source: "timeline", message: "old" });
    buffer.append({ timestampMs: 2, source: "timeline", message: "middle" });
    buffer.append({ timestampMs: 3, source: "timeline", message: "new" });

    expect(buffer.snapshot()).toEqual({ entries: [
      { timestampMs: 2, source: "timeline", message: "middle" },
      { timestampMs: 3, source: "timeline", message: "new" }
    ], droppedEntries: 1 });
  });
});
