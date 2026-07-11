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

  test("captures only an allowlisted kind and fixed channel", () => {
    recordJsError(new TypeError("boom"), "window_error");

    expect(getRecentJsErrors()).toEqual([
      { kind: "type_error", channel: "window_error" }
    ]);
  });

  test("does not retain messages, custom names, locations, stacks, paths, URLs, or tokens", () => {
    const privateDetails = [
      "secret message body",
      "!private-room:example.invalid",
      "@private-user:example.invalid",
      "$private-event:example.invalid",
      "/Users/member/private/app.tsx",
      "https://private.example.invalid/room",
      "access_token=private-token"
    ];
    const error = new Error(privateDetails.join(" "));
    error.name = "PrivateCustomError";
    error.stack = `PrivateCustomError: ${privateDetails.join(" ")}\n at ${privateDetails[4]}:10:5`;

    recordJsError(error, "unhandled_rejection");

    const serialized = JSON.stringify(getRecentJsErrors());
    expect(getRecentJsErrors()).toEqual([
      { kind: "error", channel: "unhandled_rejection" }
    ]);
    for (const privateDetail of privateDetails) {
      expect(serialized).not.toContain(privateDetail);
    }
    expect(serialized).not.toContain("PrivateCustomError");
  });

  test("bounds the buffer to the most recent errors", () => {
    for (let i = 0; i < 30; i += 1) {
      recordJsError(new Error(`e${i}`), "window_error");
    }
    const captured = getRecentJsErrors();
    expect(captured.length).toBe(20);
    expect(captured[captured.length - 1]).toEqual({
      kind: "error",
      channel: "window_error"
    });
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
      const privateFilename =
        "/Users/member/private/chunk.js?source=https://private.example.invalid&access_token=private-token";

      fake.emit("error", {
        error: new RangeError("secret message body"),
        filename: privateFilename,
        lineno: 7,
        colno: 3
      });

      expect(getRecentJsErrors()).toEqual([
        { kind: "range_error", channel: "window_error" }
      ]);
      const serialized = JSON.stringify(getRecentJsErrors());
      expect(serialized).not.toContain("secret message body");
      expect(serialized).not.toContain(privateFilename);
      expect(serialized).not.toContain("private-token");
    });

    test("records unhandled rejection reasons", () => {
      const fake = makeFakeWindow();
      installJsErrorCapture(fake as unknown as Window);

      fake.emit("unhandledrejection", { reason: new Error("rejected") });

      expect(getRecentJsErrors()[0]).toMatchObject({
        kind: "error",
        channel: "unhandled_rejection"
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
