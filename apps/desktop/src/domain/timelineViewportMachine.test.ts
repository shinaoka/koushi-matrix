import { describe, expect, it } from "vitest";

import {
  createTimelineViewportMachineState,
  reduceTimelineViewportMachine,
  type TimelineViewportMachineState
} from "./timelineViewportMachine";
import type { TimelineScrollAnchor } from "./types";

const anchor: TimelineScrollAnchor = {
  event_id: "$event",
  edge: "bottom",
  offset_px: 12,
  updated_at_ms: 1_820_000_000_000
};

function withRetainedAnchor(): TimelineViewportMachineState {
  return reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
    type: "room-anchor-restored",
    signature: "!room\u0000$event\u0000bottom\u000012",
    anchor,
    scrollTop: 480
  });
}

describe("timeline viewport machine", () => {
  it("keeps a retained room anchor through layout-only updates until the user scrolls", () => {
    const retained = withRetainedAnchor();

    const afterLayout = reduceTimelineViewportMachine(retained, {
      type: "layout-changed"
    });

    expect(afterLayout.retainedRoomAnchor).toEqual(retained.retainedRoomAnchor);

    const afterUserScroll = reduceTimelineViewportMachine(afterLayout, {
      type: "scroll-observed",
      programmaticEcho: false,
      atBottom: false,
      userInput: true
    });

    expect(afterUserScroll.intent).toEqual({ kind: "free-scroll" });
    expect(afterUserScroll.retainedRoomAnchor).toBeNull();
  });

  it("does not release live edge for programmatic scroll echoes", () => {
    const liveEdge = reduceTimelineViewportMachine(
      reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
        type: "live-edge-requested"
      }),
      {
        type: "programmatic-scroll-assigned",
        scrollHeight: 1200,
        scrollTop: 800
      }
    );

    const observed = reduceTimelineViewportMachine(liveEdge, {
      type: "scroll-observed",
      programmaticEcho: true,
      atBottom: true,
      userInput: false
    });

    expect(observed.intent).toEqual({ kind: "live-edge" });
    expect(observed.programmaticScrollSignature).toEqual({
      scrollHeight: 1200,
      scrollTop: 800
    });
  });

  it("does not release retained anchors for programmatic scroll echoes", () => {
    const retained = reduceTimelineViewportMachine(withRetainedAnchor(), {
      type: "programmatic-scroll-assigned",
      scrollHeight: 1200,
      scrollTop: 800
    });

    const observed = reduceTimelineViewportMachine(retained, {
      type: "scroll-observed",
      programmaticEcho: true,
      atBottom: false,
      userInput: false
    });

    expect(observed.retainedRoomAnchor).toEqual(retained.retainedRoomAnchor);
  });

  it("resets transient viewport state on room changes", () => {
    const active = reduceTimelineViewportMachine(
      reduceTimelineViewportMachine(withRetainedAnchor(), {
        type: "room-anchor-restore-requested",
        signature: "!room\u0000$event\u0000bottom\u000012\u00005"
      }),
      {
        type: "mark-user-scroll-input"
      }
    );

    const reset = reduceTimelineViewportMachine(active, {
      type: "timeline-key-changed"
    });

    expect(reset).toEqual(createTimelineViewportMachineState());
  });

  it("tracks requested and exhausted live anchor restore signatures explicitly", () => {
    const requested = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "room-anchor-restore-requested",
      signature: "!room\u0000$missing\u0000bottom\u00000\u00007"
    });

    expect(requested.roomAnchorRestorePending).toBe(true);
    expect(requested.requestedRoomAnchorRestoreSignature).toBe(
      "!room\u0000$missing\u0000bottom\u00000\u00007"
    );

    const exhausted = reduceTimelineViewportMachine(requested, {
      type: "room-anchor-restore-finished",
      status: "not-found"
    });

    expect(exhausted.roomAnchorRestorePending).toBe(false);
    expect(exhausted.exhaustedRoomAnchorRestoreSignature).toBe(
      "!room\u0000$missing\u0000bottom\u00000\u00007"
    );
  });
});
