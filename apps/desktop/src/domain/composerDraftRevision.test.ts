import { describe, expect, it } from "vitest";
import {
  COMPOSER_DRAFT_REVISION_ZERO,
  ComposerDraftRevisionExhaustedError,
  compareComposerDraftRevisions,
  nextComposerDraftRevision,
  parseComposerDraftRevision
} from "./composerDraftRevision";

describe("composer draft revision wire", () => {
  it("advances exactly above Number.MAX_SAFE_INTEGER", () => {
    const current = parseComposerDraftRevision("9007199254740993");
    expect(nextComposerDraftRevision(current, current)).toBe("9007199254740994");
  });

  it("rejects non-canonical and numeric-shaped input", () => {
    for (const value of ["", "00", "01", "-1", "+1", " 1", "1 ", "1.0", "1e3"]) {
      expect(() => parseComposerDraftRevision(value)).toThrow();
    }
    expect(() => parseComposerDraftRevision(1)).toThrow();
    expect(COMPOSER_DRAFT_REVISION_ZERO).toBe("0");
  });

  it("compares by bigint rather than lexicographic order", () => {
    expect(
      compareComposerDraftRevisions(
        parseComposerDraftRevision("10"),
        parseComposerDraftRevision("9")
      )
    ).toBeGreaterThan(0);
  });

  it("fails closed at u128 max", () => {
    const maximum = parseComposerDraftRevision(
      "340282366920938463463374607431768211455"
    );
    expect(() => nextComposerDraftRevision(maximum, maximum)).toThrow(
      ComposerDraftRevisionExhaustedError
    );
  });
});
