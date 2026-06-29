import { describe, expect, test } from "vitest";

import {
  decideTimelineViewportCoverage,
  type TimelineViewportCoverageInput
} from "./timelineViewportCoverage";

function input(
  overrides: Partial<TimelineViewportCoverageInput>
): TimelineViewportCoverageInput {
  return {
    source: "programmatic-restore",
    relativeScrollTopPx: 48,
    clientHeightPx: 500,
    loadedContentHeightPx: 1200,
    coverageMarginPx: 1000,
    backwardPaginationState: "Idle",
    autoLoadOlderMessages: true,
    forceUserBackfill: false,
    suppressPaginationUi: false,
    backfillInFlight: false,
    blockingAnchorWork: false,
    viewportMode: "anchored",
    ...overrides
  };
}

describe("timeline viewport coverage", () => {
  test("requests backward pagination when required render range extends before loaded content", () => {
    expect(decideTimelineViewportCoverage(input({ relativeScrollTopPx: 48 }))).toEqual({
      kind: "request-backward",
      reason: "required-range-before-loaded-start"
    });
  });

  test("does not request when the required render range is covered", () => {
    expect(decideTimelineViewportCoverage(input({ relativeScrollTopPx: 1400 }))).toEqual({
      kind: "covered",
      reason: "required-range-within-loaded-content"
    });
  });

  test("does not request when backward pagination is exhausted or already running", () => {
    expect(
      decideTimelineViewportCoverage(input({ backwardPaginationState: "EndReached" }))
    ).toEqual({ kind: "blocked", reason: "backward-pagination-unavailable" });
    expect(decideTimelineViewportCoverage(input({ backfillInFlight: true }))).toEqual({
      kind: "blocked",
      reason: "backfill-in-flight"
    });
  });

  test("allows explicit user backfill even when automatic loading is disabled", () => {
    expect(
      decideTimelineViewportCoverage(
        input({
          source: "user-scroll",
          autoLoadOlderMessages: false,
          forceUserBackfill: true
        })
      )
    ).toEqual({
      kind: "request-backward",
      reason: "required-range-before-loaded-start"
    });
  });

  test("blocks automatic programmatic backfill when automatic loading is disabled", () => {
    expect(
      decideTimelineViewportCoverage(
        input({
          source: "programmatic-restore",
          autoLoadOlderMessages: false,
          forceUserBackfill: false
        })
      )
    ).toEqual({ kind: "blocked", reason: "automatic-backfill-disabled" });
  });

  test("blocks viewport-driven backward pagination while live-edge owns startup", () => {
    expect(
      decideTimelineViewportCoverage(
        input({
          source: "layout-settle",
          viewportMode: "liveEdge",
          autoLoadOlderMessages: true,
          relativeScrollTopPx: 48
        })
      )
    ).toEqual({ kind: "blocked", reason: "live-edge-does-not-backfill" });
  });
});
