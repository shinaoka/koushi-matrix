import { describe, expect, test } from "vitest";
import { catalogs, t, type Locale } from "./messages";

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
});
