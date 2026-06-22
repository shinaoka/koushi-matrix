import { beforeEach, describe, expect, test } from "vitest";

import {
  getRecentJsErrors,
  installJsErrorCapture,
  recordJsError,
  resetJsErrors
} from "./jsErrorLog";

describe("jsErrorLog", () => {
  beforeEach(() => {
    resetJsErrors();
  });

  test("starts empty", () => {
    expect(getRecentJsErrors()).toEqual([]);
  });

  test("captures kind, message, and explicit source location", () => {
    recordJsError(new TypeError("boom"), "app.js:10:5");

    expect(getRecentJsErrors()).toEqual([
      { kind: "TypeError", message: "boom", source: "app.js:10:5" }
    ]);
  });

  test("strips room/user/event identifiers from the message", () => {
    recordJsError(
      new Error("undefined is not an object (evaluating x of !room:server / @user:server / $evt:server)")
    );

    const [captured] = getRecentJsErrors();
    expect(captured.message).not.toContain("!room:server");
    expect(captured.message).not.toContain("@user:server");
    expect(captured.message).not.toContain("$evt:server");
    expect(captured.message).toContain("<room>");
    expect(captured.message).toContain("<user>");
    expect(captured.message).toContain("<event>");
  });

  test("bounds the buffer to the most recent errors", () => {
    for (let i = 0; i < 30; i += 1) {
      recordJsError(new Error(`e${i}`));
    }
    const captured = getRecentJsErrors();
    expect(captured.length).toBe(20);
    expect(captured[captured.length - 1].message).toBe("e29");
  });

  describe("installJsErrorCapture", () => {
    function makeFakeWindow() {
      const handlers: Record<string, Set<(event: unknown) => void>> = {};
      return {
        addEventListener(type: string, handler: (event: unknown) => void) {
          (handlers[type] ??= new Set()).add(handler);
        },
        removeEventListener(type: string, handler: (event: unknown) => void) {
          handlers[type]?.delete(handler);
        },
        emit(type: string, event: unknown) {
          handlers[type]?.forEach((handler) => handler(event));
        }
      };
    }

    test("records error events through the installed listener", () => {
      const fake = makeFakeWindow();
      installJsErrorCapture(fake as unknown as Window);

      fake.emit("error", {
        error: new RangeError("dispatched"),
        filename: "chunk.js",
        lineno: 7,
        colno: 3
      });

      expect(getRecentJsErrors()).toEqual([
        { kind: "RangeError", message: "dispatched", source: "chunk.js:7:3" }
      ]);
    });

    test("records unhandled rejection reasons", () => {
      const fake = makeFakeWindow();
      installJsErrorCapture(fake as unknown as Window);

      fake.emit("unhandledrejection", { reason: new Error("rejected") });

      expect(getRecentJsErrors()[0]).toMatchObject({
        kind: "Error",
        message: "rejected"
      });
    });

    test("stops recording after uninstall", () => {
      const fake = makeFakeWindow();
      const stop = installJsErrorCapture(fake as unknown as Window);
      stop();

      fake.emit("error", { error: new Error("after") });

      expect(getRecentJsErrors()).toEqual([]);
    });
  });
});
