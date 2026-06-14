import { describe, expect, test } from "vitest";
import { catalogs, pseudoLocalize, t, type Locale } from "./messages";

describe("i18n message catalog", () => {
  test("all locales expose the same message ids", () => {
    const locales = Object.keys(catalogs) as Locale[];
    const baseline = Object.keys(catalogs.en).sort();
    for (const locale of locales) {
      expect(Object.keys(catalogs[locale]).sort()).toEqual(baseline);
    }
  });

  test("interpolates named values", () => {
    expect(t("composer.placeholder", { roomName: "Synthetic Room" })).toBe(
      "Message Synthetic Room"
    );
  });

  test("keeps Japanese locale on the English fallback until localization is staffed", () => {
    expect(t("composer.replying", {}, "ja")).toBe("Replying");
  });

  test("pseudo locale expands labels while preserving interpolation placeholders", () => {
    const pseudo = pseudoLocalize("Message {roomName}");

    expect(pseudo).toContain("{roomName}");
    expect(pseudo.length).toBeGreaterThan("Message {roomName}".length);
    expect(pseudo).not.toContain("roomName roomName");
  });

  test("pseudo catalog expansion handles RTL, CJK, and combining mark samples", () => {
    const sample = "Cafe\u0301 日本語 العربية {roomName}";
    const pseudo = pseudoLocalize(sample);

    expect(pseudo).toContain("\u0301");
    expect(pseudo).toContain("日本語");
    expect(pseudo).toContain("العربية");
    expect(pseudo).toContain("{roomName}");
    expect(pseudo.length).toBeGreaterThan(sample.length);
  });

  test("bidi pseudo mode is distinguishable from accented pseudo mode", () => {
    const sample = "Message {roomName}";
    const accented = pseudoLocalize(sample, "accented");
    const bidi = pseudoLocalize(sample, "bidi");

    expect(bidi).toContain("{roomName}");
    expect(bidi).not.toBe(accented);
    expect(bidi).toContain("\u202e");
    expect(bidi).toContain("\u202c");
  });

  test("runtime pseudo translation keeps interpolated values private-data-owned by caller", () => {
    const pseudo = t("workspace.searchPlaceholder", { spaceName: "Synthetic Space" }, "pseudo");

    expect(pseudo).toContain("Synthetic Space");
    expect(pseudo.length).toBeGreaterThan(
      t("workspace.searchPlaceholder", { spaceName: "Synthetic Space" }, "en").length
    );
  });
});
