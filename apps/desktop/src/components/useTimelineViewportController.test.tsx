// @vitest-environment jsdom

import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ReactNode } from "react";

import { useTimelineViewportController } from "./useTimelineViewportController";

afterEach(cleanup);

describe("useTimelineViewportController", () => {
  it("keeps reducer access behind a stable controller API", () => {
    let controller: ReturnType<typeof useTimelineViewportController> | null = null;

    const getController = () => {
      if (!controller) {
        throw new Error("controller was not initialized");
      }
      return controller;
    };

    function Harness({ children }: { children?: ReactNode }) {
      controller = useTimelineViewportController();
      return <>{children}</>;
    }

    render(<Harness />);

    const mountedController = getController();

    expect(mountedController.isLiveEdge()).toBe(false);
    expect(mountedController.canPersistAnchor()).toBe(true);

    act(() => {
      mountedController.dispatch({ type: "live-edge-requested" });
      mountedController.dispatch({
        type: "programmatic-scroll-assigned",
        scrollHeight: 1200,
        scrollTop: 700
      });
    });

    expect(mountedController.isLiveEdge()).toBe(true);
    expect(
      mountedController.programmaticScrollEchoMatches({
        scrollHeight: 1200,
        scrollTop: 700
      })
    ).toBe(true);

    act(() => {
      mountedController.dispatch({ type: "scroll-capture-suppression-started" });
    });

    expect(mountedController.canPersistAnchor()).toBe(false);
    expect(mountedController.canPersistAnchor({ allowSuppressed: true })).toBe(true);
  });
});
