import type { TimelineScrollAnchor } from "./types";

export type TimelineViewportIntent = { kind: "free-scroll" } | { kind: "live-edge" };

export type TimelineProgrammaticScrollSignature = {
  scrollHeight: number;
  scrollTop: number;
};

export type TimelineRetainedRoomAnchor = {
  signature: string;
  anchor: TimelineScrollAnchor;
  scrollTop: number;
};

export type TimelineViewportMachineState = {
  intent: TimelineViewportIntent;
  userScrollInputPending: boolean;
  suppressScrollAnchorCapture: boolean;
  programmaticScrollSignature: TimelineProgrammaticScrollSignature | null;
  prependAnchorRestorePending: boolean;
  roomAnchorRestorePending: boolean;
  retainedRoomAnchor: TimelineRetainedRoomAnchor | null;
  lastPersistedViewportAnchorSignature: string | null;
  restoredRoomAnchorSignature: string | null;
  requestedRoomAnchorRestoreSignature: string | null;
  exhaustedRoomAnchorRestoreSignature: string | null;
  postSettleRestoredSignature: string | null;
  stickToBottomAfterMeasurement: boolean;
};

export type TimelineViewportMachineEvent =
  | { type: "timeline-key-changed" }
  | { type: "resync-required" }
  | { type: "layout-changed" }
  | { type: "prepend-anchor-restore-started" }
  | { type: "prepend-anchor-restore-finished" }
  | { type: "room-anchor-restore-started" }
  | {
      type: "room-anchor-restored";
      signature: string;
      anchor: TimelineScrollAnchor;
      scrollTop: number;
    }
  | { type: "room-anchor-restore-requested"; signature: string }
  | { type: "room-anchor-restore-finished"; status: "found" | "not-found" | "superseded" }
  | { type: "post-settle-restore-scheduled"; signature: string }
  | { type: "retain-room-anchor"; retained: TimelineRetainedRoomAnchor | null }
  | { type: "clear-retained-room-anchor" }
  | { type: "live-edge-requested" }
  | { type: "free-scroll-requested" }
  | { type: "mark-user-scroll-input" }
  | {
      type: "scroll-observed";
      programmaticEcho: boolean;
      atBottom: boolean;
      userInput: boolean;
    }
  | { type: "scroll-capture-suppression-started" }
  | { type: "scroll-capture-suppression-finished" }
  | { type: "programmatic-scroll-assigned"; scrollHeight: number; scrollTop: number }
  | { type: "persisted-room-anchor"; signature: string }
  | { type: "stick-to-bottom-after-measurement"; value: boolean };

export function createTimelineViewportMachineState(): TimelineViewportMachineState {
  return {
    intent: { kind: "free-scroll" },
    userScrollInputPending: false,
    suppressScrollAnchorCapture: false,
    programmaticScrollSignature: null,
    prependAnchorRestorePending: false,
    roomAnchorRestorePending: false,
    retainedRoomAnchor: null,
    lastPersistedViewportAnchorSignature: null,
    restoredRoomAnchorSignature: null,
    requestedRoomAnchorRestoreSignature: null,
    exhaustedRoomAnchorRestoreSignature: null,
    postSettleRestoredSignature: null,
    stickToBottomAfterMeasurement: false
  };
}

export function reduceTimelineViewportMachine(
  state: TimelineViewportMachineState,
  event: TimelineViewportMachineEvent
): TimelineViewportMachineState {
  switch (event.type) {
    case "timeline-key-changed":
    case "resync-required":
      return createTimelineViewportMachineState();
    case "layout-changed":
      return state;
    case "prepend-anchor-restore-started":
      return { ...state, prependAnchorRestorePending: true };
    case "prepend-anchor-restore-finished":
      return { ...state, prependAnchorRestorePending: false };
    case "room-anchor-restore-started":
      return { ...state, roomAnchorRestorePending: true };
    case "room-anchor-restored":
      return {
        ...state,
        roomAnchorRestorePending: false,
        restoredRoomAnchorSignature: event.signature,
        retainedRoomAnchor: {
          signature: event.signature,
          anchor: event.anchor,
          scrollTop: event.scrollTop
        }
      };
    case "room-anchor-restore-requested":
      return {
        ...state,
        roomAnchorRestorePending: true,
        requestedRoomAnchorRestoreSignature: event.signature,
        exhaustedRoomAnchorRestoreSignature: null
      };
    case "room-anchor-restore-finished":
      if (event.status === "superseded") {
        return state;
      }
      return {
        ...state,
        roomAnchorRestorePending: false,
        exhaustedRoomAnchorRestoreSignature:
          event.status === "not-found"
            ? state.requestedRoomAnchorRestoreSignature
            : state.exhaustedRoomAnchorRestoreSignature
      };
    case "post-settle-restore-scheduled":
      return { ...state, postSettleRestoredSignature: event.signature };
    case "retain-room-anchor":
      return { ...state, retainedRoomAnchor: event.retained };
    case "clear-retained-room-anchor":
      return { ...state, retainedRoomAnchor: null };
    case "live-edge-requested":
      return { ...state, intent: { kind: "live-edge" }, retainedRoomAnchor: null };
    case "free-scroll-requested":
      return {
        ...state,
        intent: { kind: "free-scroll" },
        userScrollInputPending: false,
        stickToBottomAfterMeasurement: false,
        retainedRoomAnchor: null
      };
    case "mark-user-scroll-input":
      return { ...state, userScrollInputPending: true };
    case "scroll-observed":
      if (event.programmaticEcho) {
        return state;
      }
      if (event.userInput && event.atBottom) {
        return {
          ...state,
          intent: { kind: "live-edge" },
          userScrollInputPending: false,
          retainedRoomAnchor: null
        };
      }
      if (event.userInput || state.intent.kind === "live-edge") {
        return reduceTimelineViewportMachine(state, { type: "free-scroll-requested" });
      }
      return { ...state, retainedRoomAnchor: null };
    case "scroll-capture-suppression-started":
      return { ...state, suppressScrollAnchorCapture: true };
    case "scroll-capture-suppression-finished":
      return {
        ...state,
        suppressScrollAnchorCapture: false,
        programmaticScrollSignature: null
      };
    case "programmatic-scroll-assigned":
      return {
        ...state,
        programmaticScrollSignature: {
          scrollHeight: event.scrollHeight,
          scrollTop: event.scrollTop
        }
      };
    case "persisted-room-anchor":
      return {
        ...state,
        lastPersistedViewportAnchorSignature: event.signature
      };
    case "stick-to-bottom-after-measurement":
      return {
        ...state,
        stickToBottomAfterMeasurement: event.value
      };
  }
}
