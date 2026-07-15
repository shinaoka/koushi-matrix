import { describe, expect, test } from "vitest";

import type { PaginationState } from "./coreEvents";
import {
  evaluateTimelineBackfill,
  type TimelineBackfillBlocker,
  type TimelineBackfillSnapshot
} from "./timelineBackfillPolicy";

function snapshot(
  overrides: Partial<TimelineBackfillSnapshot> = {}
): TimelineBackfillSnapshot {
  return {
    trigger: "layout_settled",
    initialized: true,
    awaitingResync: false,
    suppressPaginationUi: false,
    autoLoadEnabled: true,
    paginationState: "Idle",
    requestInFlight: false,
    projectionSettled: true,
    virtualLayoutSettled: true,
    anchorSettled: true,
    itemCount: 20,
    projectedContentHeight: 1_200,
    clientHeight: 600,
    scrollHeight: 1_200,
    scrollTop: 400,
    nearTopThreshold: 160,
    genuineUserScroll: false,
    ...overrides
  };
}

describe("evaluateTimelineBackfill", () => {
  test("evaluates demand and blockers without event-order state", () => {
    const blockerCases: Array<{
      reason: TimelineBackfillBlocker;
      overrides: Partial<TimelineBackfillSnapshot>;
    }> = [
      { reason: "not_initialized", overrides: { initialized: false } },
      { reason: "awaiting_resync", overrides: { awaitingResync: true } },
      { reason: "suppressed_ui", overrides: { suppressPaginationUi: true } },
      { reason: "projection_unsettled", overrides: { projectionSettled: false } },
      {
        reason: "virtual_layout_unsettled",
        overrides: { virtualLayoutSettled: false }
      },
      { reason: "anchor_unsettled", overrides: { anchorSettled: false } },
      { reason: "request_in_flight", overrides: { requestInFlight: true } },
      {
        reason: "pagination_paginating",
        overrides: { paginationState: "Paginating" }
      },
      {
        reason: "pagination_end_reached",
        overrides: { paginationState: "EndReached" }
      }
    ];

    for (const blocker of blockerCases) {
      expect(
        evaluateTimelineBackfill(
          snapshot({
            projectedContentHeight: 300,
            scrollHeight: 300,
            ...blocker.overrides
          })
        ),
        blocker.reason
      ).toEqual({
        kind: "blocked",
        demand: "underfilled",
        reason: blocker.reason
      });
    }

    expect(
      evaluateTimelineBackfill(
        snapshot({ projectedContentHeight: 300, scrollHeight: 300 })
      )
    ).toEqual({ kind: "request", demand: "underfilled" });

    expect(evaluateTimelineBackfill(snapshot({ scrollTop: 80 }))).toEqual({
      kind: "request",
      demand: "near_top_prefetch"
    });

    expect(
      evaluateTimelineBackfill(
        snapshot({ autoLoadEnabled: false, genuineUserScroll: true, scrollTop: 0 })
      )
    ).toEqual({ kind: "request", demand: "explicit_top_scroll" });

    expect(
      evaluateTimelineBackfill(snapshot({ autoLoadEnabled: false }))
    ).toEqual({ kind: "idle", reason: "auto_load_disabled" });

    expect(evaluateTimelineBackfill(snapshot())).toEqual({
      kind: "idle",
      reason: "no_demand"
    });

    const failedState: PaginationState = { Failed: { kind: "Network" } };
    expect(
      evaluateTimelineBackfill(
        snapshot({
          paginationState: failedState,
          projectedContentHeight: 300,
          scrollHeight: 300
        })
      )
    ).toEqual({ kind: "request", demand: "underfilled" });
  });

  test("does not let projected height hide a DOM underfill", () => {
    expect(
      evaluateTimelineBackfill(
        snapshot({
          projectedContentHeight: 0,
          scrollHeight: 320,
          clientHeight: 600
        })
      )
    ).toEqual({ kind: "request", demand: "underfilled" });
  });

  test("does not treat a transient virtual DOM window as underfilled", () => {
    expect(
      evaluateTimelineBackfill(
        snapshot({
          projectedContentHeight: 240_000,
          scrollHeight: 367,
          clientHeight: 367,
          scrollTop: 200
        })
      )
    ).toEqual({ kind: "idle", reason: "no_demand" });
  });

  test("prefers explicit user top-scroll over automatic demand", () => {
    expect(
      evaluateTimelineBackfill(
        snapshot({
          genuineUserScroll: true,
          scrollTop: 0,
          projectedContentHeight: 300,
          scrollHeight: 300
        })
      )
    ).toEqual({ kind: "request", demand: "explicit_top_scroll" });
  });
});
