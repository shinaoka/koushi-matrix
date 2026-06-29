import { describe, expect, it } from "vitest";

import {
  createTimelineViewportMachineState,
  eventTimelineViewportTarget,
  reduceTimelineViewportMachine,
  timelineViewportCanPersistAnchor,
  timelineViewportCanRequestCoverageBackfill,
  timelineViewportHasBlockingAnchorWork,
  timelineViewportIsLiveEdge,
  timelineViewportProgrammaticScrollTokenMatches,
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
      type: "programmatic-scroll-assigned"
    });

    expect(afterProgrammaticWork.retainedRoomAnchor).toEqual(
      retained.retainedRoomAnchor
    );

    const afterUserScroll = reduceTimelineViewportMachine(afterProgrammaticWork, {
      type: "scroll-observed",
      programmaticToken: null,
      atBottom: false,
      userInput: true
    });

    expect(afterUserScroll.intent).toEqual({ kind: "anchored" });
    expect(afterUserScroll.retainedRoomAnchor).toBeNull();
  });

  it("does not release live edge for programmatic scroll echoes", () => {
    const liveEdge = reduceTimelineViewportMachine(
      reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
        type: "live-edge-requested"
      }),
      {
        type: "programmatic-scroll-assigned"
      }
    );

    const token = liveEdge.programmaticToken;
    const observed = reduceTimelineViewportMachine(liveEdge, {
      type: "scroll-observed",
      programmaticToken: token,
      atBottom: true,
      userInput: false
    });

    expect(observed.intent).toEqual({ kind: "live-edge" });
    expect(observed.pendingProgrammaticScrollToken).toBeNull();
    expect(observed.scrollActivity).toBe("idle");
  });

  it("does not release retained anchors for programmatic scroll echoes", () => {
    const retained = reduceTimelineViewportMachine(withRetainedAnchor(), {
      type: "programmatic-scroll-assigned"
    });

    const observed = reduceTimelineViewportMachine(retained, {
      type: "scroll-observed",
      programmaticToken: retained.programmaticToken,
      atBottom: false,
      userInput: false
    });

    expect(observed.retainedRoomAnchor).toEqual(retained.retainedRoomAnchor);
  });

  it("blocks anchor persistence while a retained room anchor is protecting the viewport", () => {
    const started = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "room-anchor-restore-started"
    });

    const restored = reduceTimelineViewportMachine(started, {
      type: "room-anchor-restored",
      signature: "!room\u0000$event\u0000bottom\u000012",
      anchor,
      scrollTop: 480
    });

    expect(restored.roomAnchorRestorePending).toBe(false);
    expect(restored.retainedRoomAnchor).not.toBeNull();
    expect(timelineViewportCanPersistAnchor(restored)).toBe(false);
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
      type: "room-anchor-materialize-requested",
      signature: "!room\u0000$event\u0000bottom\u00000\u00008"
    });

    expect(timelineViewportCanPersistAnchor(state)).toBe(false);
    expect(timelineViewportHasBlockingAnchorWork(state)).toBe(true);
  });

  it("assigns monotonic programmatic scroll tokens behind the state-machine API", () => {
    const first = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "programmatic-scroll-assigned"
    });
    const second = reduceTimelineViewportMachine(first, {
      type: "programmatic-scroll-assigned"
    });

    expect(first.programmaticToken).toBe(1);
    expect(first.pendingProgrammaticScrollToken).toBe(1);
    expect(second.programmaticToken).toBe(2);
    expect(second.pendingProgrammaticScrollToken).toBe(2);
    expect(timelineViewportProgrammaticScrollTokenMatches(second, 1)).toBe(false);
    expect(timelineViewportProgrammaticScrollTokenMatches(second, 2)).toBe(true);
    expect(timelineViewportProgrammaticScrollTokenMatches(second, null)).toBe(false);
  });

  it("keeps non-programmatic inertial scroll activity durable until idle", () => {
    const liveEdge = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "live-edge-requested"
    });

    const active = reduceTimelineViewportMachine(liveEdge, {
      type: "scroll-observed",
      programmaticToken: null,
      atBottom: false,
      userInput: false
    });

    expect(active.intent).toEqual({ kind: "live-edge" });
    expect(active.scrollActivity).toBe("active");

    const idle = reduceTimelineViewportMachine(active, {
      type: "scroll-activity-idle"
    });

    expect(idle.scrollActivity).toBe("idle");
    expect(idle.userScrollInputPending).toBe(false);
  });

  it("clears stickToBottomAfterMeasurement when user scrolls away from bottom while live-edge is active", () => {
    const liveEdge = reduceTimelineViewportMachine(
      createTimelineViewportMachineState(),
      { type: "live-edge-requested" }
    );

    const withSticky = reduceTimelineViewportMachine(liveEdge, {
      type: "stick-to-bottom-after-measurement",
      value: true
    });

    expect(withSticky.stickToBottomAfterMeasurement).toBe(true);
    expect(withSticky.intent).toEqual({ kind: "live-edge" });

    const afterUserScroll = reduceTimelineViewportMachine(withSticky, {
      type: "scroll-observed",
      programmaticToken: null,
      atBottom: false,
      userInput: true
    });

    expect(afterUserScroll.intent).toEqual({ kind: "anchored" });
    expect(afterUserScroll.stickToBottomAfterMeasurement).toBe(false);
  });

  it("keeps live-edge intent through non-user scroll observations", () => {
    const stickyLiveEdge = reduceTimelineViewportMachine(
      reduceTimelineViewportMachine(
        createTimelineViewportMachineState(),
        { type: "live-edge-requested" }
      ),
      { type: "stick-to-bottom-after-measurement", value: true }
    );

    const afterScroll = reduceTimelineViewportMachine(stickyLiveEdge, {
      type: "scroll-observed",
      programmaticToken: null,
      atBottom: false,
      userInput: false
    });

    expect(afterScroll.intent).toEqual({ kind: "live-edge" });
    expect(afterScroll.stickToBottomAfterMeasurement).toBe(true);
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

  it("uses anchored as the explicit free-scroll viewport intent", () => {
    const state = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "free-scroll-requested"
    });

    expect(state.intent).toEqual({ kind: "anchored" });
  });

  it("does not promote anchored layout observations to live edge", () => {
    const anchored = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "free-scroll-requested"
    });

    const observed = reduceTimelineViewportMachine(anchored, {
      type: "scroll-observed",
      programmaticToken: null,
      atBottom: true,
      userInput: false
    });

    expect(observed.intent).toEqual({ kind: "anchored" });
  });

  it("models targeting as a bounded transition that settles to an anchor", () => {
    const targeting = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "targeting-requested",
      target: eventTimelineViewportTarget("$event", "activity", "end")
    });

    expect(targeting.intent).toEqual({
      kind: "targeting",
      target: eventTimelineViewportTarget("$event", "activity", "end")
    });

    const settled = reduceTimelineViewportMachine(targeting, {
      type: "targeting-settled"
    });

    expect(settled.intent).toEqual({ kind: "anchored" });
  });

  it("deduplicates coverage-driven backfill requests by signature", () => {
    const requested = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "coverage-backfill-requested",
      signature: "room:!r:generation:1:backward"
    });

    expect(
      timelineViewportCanRequestCoverageBackfill(requested, "room:!r:generation:1:backward")
    ).toBe(false);
    expect(
      timelineViewportCanRequestCoverageBackfill(requested, "room:!r:generation:2:backward")
    ).toBe(true);
  });

  it("resets coverage backfill state on timeline key change", () => {
    const requested = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
      type: "coverage-backfill-requested",
      signature: "room:!r:generation:1:backward"
    });

    const reset = reduceTimelineViewportMachine(requested, {
      type: "timeline-key-changed"
    });

    expect(reset.lastCoverageBackfillRequestSignature).toBeNull();
  });
});
