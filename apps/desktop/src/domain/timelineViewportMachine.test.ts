import { describe, expect, it } from "vitest";

import {
  createTimelineViewportMachineState,
  eventTimelineViewportTarget,
  reduceTimelineViewportMachine,
  timelineViewportCanPersistAnchor,
  timelineViewportHasBlockingAnchorWork,
  timelineViewportIsLiveEdge,
  timelineViewportProgrammaticScrollEchoMatches,
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
  it("keeps a retained room anchor through programmatic work until the user scrolls", () => {
    const retained = withRetainedAnchor();

    const afterProgrammaticWork = reduceTimelineViewportMachine(retained, {
      type: "programmatic-scroll-assigned",
      scrollHeight: 1200,
      scrollTop: 800
    });

    expect(afterProgrammaticWork.retainedRoomAnchor).toEqual(
      retained.retainedRoomAnchor
    );

    const afterUserScroll = reduceTimelineViewportMachine(afterProgrammaticWork, {
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
        type: "room-anchor-materialize-requested",
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

  it("tracks requested and exhausted live anchor materialize signatures explicitly", () => {
    const requested = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "room-anchor-materialize-requested",
      signature: "!room\u0000$missing\u0000bottom\u00000\u00007"
    });

    expect(requested.roomAnchorMaterializePending).toBe(true);
    expect(requested.requestedRoomAnchorMaterializeSignature).toBe(
      "!room\u0000$missing\u0000bottom\u00000\u00007"
    );

    const exhausted = reduceTimelineViewportMachine(requested, {
      type: "room-anchor-materialize-finished",
      status: "not-found"
    });

    expect(exhausted.roomAnchorMaterializePending).toBe(false);
    expect(exhausted.exhaustedRoomAnchorMaterializeSignature).toBe(
      "!room\u0000$missing\u0000bottom\u00000\u00007"
    );
  });

  it("exposes selectors for the only states that may persist a room anchor", () => {
    let state = createTimelineViewportMachineState();

    expect(timelineViewportCanPersistAnchor(state)).toBe(true);
    expect(timelineViewportHasBlockingAnchorWork(state)).toBe(false);

    state = reduceTimelineViewportMachine(state, {
      type: "scroll-capture-suppression-started"
    });

    expect(timelineViewportCanPersistAnchor(state)).toBe(false);
    expect(timelineViewportCanPersistAnchor(state, { allowSuppressed: true })).toBe(
      true
    );

    state = reduceTimelineViewportMachine(state, {
      type: "room-anchor-materialize-requested",
      signature: "!room\u0000$event\u0000bottom\u00000\u00008"
    });

    expect(timelineViewportCanPersistAnchor(state, { allowSuppressed: true })).toBe(
      false
    );
    expect(timelineViewportHasBlockingAnchorWork(state)).toBe(true);
  });

  it("keeps programmatic scroll echo detection behind the state-machine API", () => {
    const state = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "programmatic-scroll-assigned",
      scrollHeight: 2048,
      scrollTop: 512
    });

    expect(
      timelineViewportProgrammaticScrollEchoMatches(state, {
        scrollHeight: 2048,
        scrollTop: 512
      })
    ).toBe(true);
    expect(
      timelineViewportProgrammaticScrollEchoMatches(state, {
        scrollHeight: 2048,
        scrollTop: 513
      })
    ).toBe(false);
  });

  it("builds explicit viewport targets for all event navigation entry points", () => {
    expect(timelineViewportIsLiveEdge(createTimelineViewportMachineState())).toBe(
      false
    );
    expect(eventTimelineViewportTarget("$event", "activity", "end")).toEqual({
      eventId: "$event",
      source: "activity",
      block: "end"
    });
  });
});
