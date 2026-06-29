import { useCallback, useRef } from "react";

import {
  createTimelineViewportMachineState,
  eventTimelineViewportTarget,
  reduceTimelineViewportMachine,
  timelineViewportCanPersistAnchor,
  timelineViewportCanRequestCoverageBackfill,
  timelineViewportCoverageMode,
  timelineViewportHasBlockingAnchorWork,
  timelineViewportIsLiveEdge,
  timelineViewportProgrammaticScrollTokenMatches,
  type TimelineViewportAnchorCaptureOptions,
  type TimelineViewportMachineEvent,
  type TimelineViewportMachineState,
  type TimelineViewportTargetBlock,
  type TimelineViewportTargetSource
} from "../domain/timelineViewportMachine";

type TimelineViewportScrollObservation = {
  atBottom: boolean;
  userInput: boolean;
};

const PROGRAMMATIC_SCROLL_TOKEN_DATASET_KEY = "timelineProgrammaticScrollToken";

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

  const programmaticScrollTokenMatches = useCallback(
    (token: number | null) =>
      timelineViewportProgrammaticScrollTokenMatches(stateRef.current, token),
    []
  );

  const clearProgrammaticScrollToken = useCallback((element: HTMLElement | null) => {
    if (!element) {
      return;
    }
    delete element.dataset[PROGRAMMATIC_SCROLL_TOKEN_DATASET_KEY];
  }, []);

  const consumeProgrammaticScrollToken = useCallback(
    (element: HTMLElement | null): number | null => {
      if (!element) {
        return null;
      }
      const rawToken = element.dataset[PROGRAMMATIC_SCROLL_TOKEN_DATASET_KEY];
      clearProgrammaticScrollToken(element);
      if (!rawToken) {
        return null;
      }
      const token = Number(rawToken);
      return Number.isSafeInteger(token) && token > 0 ? token : null;
    },
    [clearProgrammaticScrollToken]
  );

  const assignProgrammaticScrollToken = useCallback((element: HTMLElement): number => {
    const nextState = dispatch({ type: "programmatic-scroll-assigned" });
    const token = nextState.programmaticToken;
    element.dataset[PROGRAMMATIC_SCROLL_TOKEN_DATASET_KEY] = String(token);
    return token;
  }, [dispatch]);

  const cancelProgrammaticScrollToken = useCallback(
    (element: HTMLElement, token: number | null) => {
      clearProgrammaticScrollToken(element);
      if (token !== null) {
        dispatch({ type: "programmatic-scroll-cancelled", token });
      }
    },
    [clearProgrammaticScrollToken, dispatch]
  );

  const scrollTo = useCallback(
    (element: HTMLElement, scrollTop: number): number | null => {
      if (element.scrollTop === scrollTop) {
        return null;
      }
      const beforeScrollTop = element.scrollTop;
      const token = assignProgrammaticScrollToken(element);
      element.scrollTop = scrollTop;
      if (element.scrollTop !== beforeScrollTop) {
        return token;
      }
      cancelProgrammaticScrollToken(element, token);
      return null;
    },
    [assignProgrammaticScrollToken, cancelProgrammaticScrollToken]
  );

  const scrollBy = useCallback(
    (element: HTMLElement, delta: number): number | null => {
      if (delta === 0) {
        return null;
      }
      const beforeScrollTop = element.scrollTop;
      const token = assignProgrammaticScrollToken(element);
      element.scrollTop = beforeScrollTop + delta;
      if (element.scrollTop !== beforeScrollTop) {
        return token;
      }
      cancelProgrammaticScrollToken(element, token);
      return null;
    },
    [assignProgrammaticScrollToken, cancelProgrammaticScrollToken]
  );

  const runProgrammaticScroll = useCallback(
    (element: HTMLElement, action: () => void): number | null => {
      const beforeScrollTop = element.scrollTop;
      const token = assignProgrammaticScrollToken(element);
      try {
        action();
      } catch (error) {
        cancelProgrammaticScrollToken(element, token);
        throw error;
      }
      if (element.scrollTop !== beforeScrollTop) {
        return token;
      }
      cancelProgrammaticScrollToken(element, token);
      return null;
    },
    [assignProgrammaticScrollToken, cancelProgrammaticScrollToken]
  );

  const observeScroll = useCallback(
    (
      element: HTMLElement,
      observation: TimelineViewportScrollObservation
    ): { programmaticEcho: boolean; state: TimelineViewportMachineState } => {
      const token = consumeProgrammaticScrollToken(element);
      const programmaticEcho = timelineViewportProgrammaticScrollTokenMatches(
        stateRef.current,
        token
      );
      const state = dispatch({
        type: "scroll-observed",
        programmaticToken: token,
        atBottom: observation.atBottom,
        userInput: observation.userInput && !programmaticEcho
      });
      return { programmaticEcho, state };
    },
    [consumeProgrammaticScrollToken, dispatch]
  );

  const markUserScrollInput = useCallback(
    (element?: HTMLElement | null) => {
      clearProgrammaticScrollToken(element ?? null);
      dispatch({ type: "mark-user-scroll-input" });
    },
    [clearProgrammaticScrollToken, dispatch]
  );

  const settleScrollActivityIdle = useCallback(() => {
    dispatch({ type: "scroll-activity-idle" });
  }, [dispatch]);

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

  const coverageMode = useCallback(
    () => timelineViewportCoverageMode(stateRef.current),
    []
  );

  return {
    current,
    dispatch,
    isLiveEdge,
    hasBlockingAnchorWork,
    canPersistAnchor,
    programmaticScrollTokenMatches,
    consumeProgrammaticScrollToken,
    clearProgrammaticScrollToken,
    scrollTo,
    scrollBy,
    runProgrammaticScroll,
    observeScroll,
    markUserScrollInput,
    settleScrollActivityIdle,
    eventTarget,
    canRequestCoverageBackfill,
    coverageMode
  };
}
