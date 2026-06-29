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
    });

    expect(mountedController.isLiveEdge()).toBe(true);

    act(() => {
      mountedController.dispatch({ type: "scroll-capture-suppression-started" });
    });

    expect(mountedController.canPersistAnchor()).toBe(false);
    expect(mountedController.canPersistAnchor({ allowSuppressed: true })).toBe(false);

    act(() => {
      mountedController.dispatch({ type: "free-scroll-requested" });
      mountedController.dispatch({ type: "scroll-capture-suppression-started" });
    });

    expect(mountedController.coverageMode()).toBe("anchored");
    expect(mountedController.canPersistAnchor()).toBe(false);
    expect(mountedController.canPersistAnchor({ allowSuppressed: true })).toBe(true);
  });

  it("owns programmatic scroll writes and classifies their token echoes", () => {
    let controller: ReturnType<typeof useTimelineViewportController> | null = null;

    const getController = () => {
      if (!controller) {
        throw new Error("controller was not initialized");
      }
      return controller;
    };

    function Harness() {
      controller = useTimelineViewportController();
      return null;
    }

    render(<Harness />);

    const mountedController = getController();
    const element = document.createElement("div");
    Object.defineProperty(element, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    let firstToken: number | null = null;
    act(() => {
      firstToken = mountedController.scrollTo(element, 120);
    });

    expect(firstToken).toBe(1);
    expect(element.scrollTop).toBe(120);
    expect(mountedController.current().programmaticToken).toBe(1);
    expect(mountedController.current().pendingProgrammaticScrollToken).toBe(1);

    const observed = mountedController.observeScroll(element, {
      atBottom: false,
      userInput: false
    });

    expect(observed.programmaticEcho).toBe(true);
    expect(mountedController.current().pendingProgrammaticScrollToken).toBeNull();
    expect(mountedController.current().scrollActivity).toBe("idle");

    let secondToken: number | null = null;
    act(() => {
      secondToken = mountedController.scrollBy(element, 30);
    });

    expect(secondToken).toBe(2);
    expect(element.scrollTop).toBe(150);

    act(() => {
      mountedController.markUserScrollInput(element);
    });

    expect(mountedController.consumeProgrammaticScrollToken(element)).toBeNull();
    expect(mountedController.current().pendingProgrammaticScrollToken).toBeNull();
    expect(mountedController.current().scrollActivity).toBe("active");

    act(() => {
      mountedController.settleScrollActivityIdle();
    });

    expect(mountedController.current().scrollActivity).toBe("idle");
    expect(mountedController.current().userScrollInputPending).toBe(false);
  });

  it("clears programmatic tokens when callback-owned scrolling throws", () => {
    let controller: ReturnType<typeof useTimelineViewportController> | null = null;

    const getController = () => {
      if (!controller) {
        throw new Error("controller was not initialized");
      }
      return controller;
    };

    function Harness() {
      controller = useTimelineViewportController();
      return null;
    }

    render(<Harness />);

    const mountedController = getController();
    const element = document.createElement("div");
    Object.defineProperty(element, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    act(() => {
      expect(() =>
        mountedController.runProgrammaticScroll(element, () => {
          throw new Error("scroll failed");
        })
      ).toThrow("scroll failed");
    });

    expect(mountedController.current().programmaticToken).toBe(1);
    expect(mountedController.current().pendingProgrammaticScrollToken).toBeNull();
    expect(mountedController.consumeProgrammaticScrollToken(element)).toBeNull();
  });
});
