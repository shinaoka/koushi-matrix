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
  test("measured-row changes accumulate instead of living only on the last frame", () => {
    let diagnostics: TimelineScrollDiagnostics = createInitialTimelineScrollDiagnostics();

    function frame(changedMeasuredRowCount: number) {
      diagnostics = recordTimelineScrollFrame(diagnostics, {
        scrollActivity: "active",
        viewportIntent: "freeScroll",
        userInputPending: false,
        virtualized: true,
        startIndex: 0,
        endIndex: 10,
        paddingTop: 0,
        paddingBottom: 0,
        anchorTopDeltaPx: 0,
        changedMeasuredRowCount,
        heightDeltaAboveViewportPx: 0,
        heightDeltaInsideViewportPx: 0,
        heightDeltaBelowViewportPx: 0
      });
    }

    frame(2);
    // Any later frame overwrites `latestFrame`, and the component emits plenty
    // of them with no measured-row change. A test that asks "did a measurement
    // commit change rows?" through `latestFrame` is therefore a race: it
    // depends on the read landing before the next frame. It passed locally and
    // failed on a CI runner exactly this way.
    frame(0);

    expect(diagnostics.latestFrame?.changedMeasuredRowCount).toBe(0);
    // The cumulative counter answers the same question without the race.
    expect(diagnostics.changedMeasuredRows).toBe(2);
  });

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
    expect(diagnostics.changedMeasuredRows).toBe(3);
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
