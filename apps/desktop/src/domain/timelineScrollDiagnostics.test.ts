import { describe, expect, test } from "vitest";

import {
  createInitialTimelineScrollDiagnostics,
  recordTimelineScrollCommit,
  recordTimelineScrollEstimate,
  recordTimelineScrollFrame,
  recordTimelineScrollHeightCommit,
  recordTimelineScrollMeasurementFlush,
  recordTimelineScrollRangeCommit,
  recordTimelineScrollWrite,
  timelineScrollDiagnosticTokens,
  type TimelineScrollDiagnostics
} from "./timelineScrollDiagnostics";

describe("timelineScrollDiagnostics", () => {
  test("records private-data-free scroll counters and tokens", () => {
    let diagnostics: TimelineScrollDiagnostics = createInitialTimelineScrollDiagnostics();

    diagnostics = recordTimelineScrollCommit(diagnostics);
    diagnostics = recordTimelineScrollFrame(diagnostics, {
      scrollActivity: "active",
      viewportIntent: "anchored",
      userInputPending: true,
      virtualized: true,
      startIndex: 120,
      endIndex: 240,
      paddingTop: 8640,
      paddingBottom: 2800,
      anchorTopDeltaPx: 0,
      changedMeasuredRowCount: 0,
      heightDeltaAboveViewportPx: 0,
      heightDeltaInsideViewportPx: 0,
      heightDeltaBelowViewportPx: 0
    });
    diagnostics = recordTimelineScrollRangeCommit(diagnostics);
    diagnostics = recordTimelineScrollHeightCommit(diagnostics, "idleFlush");
    diagnostics = recordTimelineScrollMeasurementFlush(diagnostics, 3);
    diagnostics = recordTimelineScrollWrite(diagnostics, "measurementFlush");
    diagnostics = recordTimelineScrollEstimate(diagnostics, {
      rowKind: "media",
      estimatedPx: 72,
      measuredPx: 180
    });

    expect(timelineScrollDiagnosticTokens(diagnostics)).toEqual([
      "timeline_scroll_commits=1",
      "timeline_scroll_frames=1",
      "timeline_scroll_active_frames=1",
      "timeline_scroll_range_commits=1",
      "timeline_scroll_height_commits=1",
      "timeline_scroll_flushes=1",
      "timeline_scroll_pending_heights=0",
      "timeline_scroll_writes=1",
      "timeline_scroll_max_anchor_delta_px=0",
      "timeline_scroll_media_estimate_error_px=108"
    ]);

    const serialized = JSON.stringify(diagnostics);
    expect(serialized).not.toContain("!room");
    expect(serialized).not.toContain("$event");
    expect(serialized).not.toContain("@user");
    expect(serialized).not.toContain("mxc://");
    expect(serialized).not.toContain("message body");
  });
});
