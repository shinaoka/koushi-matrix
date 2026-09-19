// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

// Node >= 25 predefines Web Storage on its global object, and Vitest's jsdom
// environment keeps existing globals. Without the shim in dialogPolyfill.ts
// the DOM tests see Node's `localStorage` getter (undefined) instead of
// jsdom's Storage, so this fails on Node 25+ and passes on Node 22.
describe("vitest jsdom Web Storage globals", () => {
  it("exposes jsdom Storage objects for localStorage and sessionStorage", () => {
    for (const storage of [localStorage, sessionStorage]) {
      expect(storage).toBeInstanceOf(Storage);
      storage.setItem("probe", "1");
      expect(storage.getItem("probe")).toBe("1");
      storage.clear();
      expect(storage.getItem("probe")).toBeNull();
    }
  });
});
