import { describe, expect, it } from "vitest";

import { findQueryHighlightRange } from "./searchHighlight";

describe("findQueryHighlightRange", () => {
  it("matches an exact CJK substring", () => {
    expect(findQueryHighlightRange("再アンケートです", "アンケート")).toEqual({
      start: 1,
      end: 6,
    });
  });

  // #162: highlight must use the same NFKC + case-fold rule as the Rust search
  // matcher so a visible highlight and the search count never disagree.
  it("matches a full-width query against half-width text (NFKC)", () => {
    const text = "say ABC now";
    const range = findQueryHighlightRange(text, "ＡＢＣ");
    expect(range).not.toBeNull();
    expect(text.slice(range!.start, range!.end)).toBe("ABC");
  });

  it("is case-insensitive", () => {
    expect(findQueryHighlightRange("Hello World", "hello")).toEqual({
      start: 0,
      end: 5,
    });
  });

  it("returns null when the query is absent", () => {
    expect(findQueryHighlightRange("hello", "zzz")).toBeNull();
  });

  it("returns null for a blank query", () => {
    expect(findQueryHighlightRange("hello", "  ")).toBeNull();
  });
});
