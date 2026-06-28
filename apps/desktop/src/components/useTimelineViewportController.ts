import { useCallback, useRef } from "react";

import {
  createTimelineViewportMachineState,
  eventTimelineViewportTarget,
  reduceTimelineViewportMachine,
  timelineViewportCanPersistAnchor,
  timelineViewportCanRequestCoverageBackfill,
  timelineViewportHasBlockingAnchorWork,
  timelineViewportIsLiveEdge,
  timelineViewportProgrammaticScrollEchoMatches,
  type TimelineViewportAnchorCaptureOptions,
  type TimelineViewportMachineEvent,
  type TimelineViewportMachineState,
  type TimelineViewportScrollMetrics,
  type TimelineViewportTargetBlock,
  type TimelineViewportTargetSource
} from "../domain/timelineViewportMachine";

export function useTimelineViewportController() {
  const stateRef = useRef(createTimelineViewportMachineState());

  const current = useCallback((): TimelineViewportMachineState => stateRef.current, []);

  const dispatch = useCallback((event: TimelineViewportMachineEvent) => {
    stateRef.current = reduceTimelineViewportMachine(stateRef.current, event);
    return stateRef.current;
  }, []);

  const isLiveEdge = useCallback(
    () => timelineViewportIsLiveEdge(stateRef.current),
    []
  );

  const hasBlockingAnchorWork = useCallback(
    () => timelineViewportHasBlockingAnchorWork(stateRef.current),
    []
  );

  const canPersistAnchor = useCallback(
    (options?: TimelineViewportAnchorCaptureOptions) =>
      timelineViewportCanPersistAnchor(stateRef.current, options),
    []
  );

  const programmaticScrollEchoMatches = useCallback(
    (metrics: TimelineViewportScrollMetrics) =>
      timelineViewportProgrammaticScrollEchoMatches(stateRef.current, metrics),
    []
  );

  const eventTarget = useCallback(
    (
      eventId: string,
      source: TimelineViewportTargetSource,
      block?: TimelineViewportTargetBlock
    ) => eventTimelineViewportTarget(eventId, source, block),
    []
  );

  const canRequestCoverageBackfill = useCallback(
    (signature: string) =>
      timelineViewportCanRequestCoverageBackfill(stateRef.current, signature),
    []
  );

  return {
    current,
    dispatch,
    isLiveEdge,
    hasBlockingAnchorWork,
    canPersistAnchor,
    programmaticScrollEchoMatches,
    eventTarget,
    canRequestCoverageBackfill
  };
}
