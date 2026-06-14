import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

describe("styles.css token system", () => {
  test("defines the blue brand token in light and dark", () => {
    expect(css).toContain("--brand: #2D6FEF;");
    expect(css).toMatch(/:root\[data-theme="dark"\]/);
    expect(css).toContain("--brand: #5C8DF6;");
  });

  test("supports OS dark mode and a color-scheme", () => {
    expect(css).toContain("@media (prefers-color-scheme: dark)");
    expect(css).toContain("color-scheme: light dark;");
  });

  test("ships the full semantic token set in :root", () => {
    for (const token of [
      "--brand-hover",
      "--brand-weak",
      "--brand-contrast",
      "--rail",
      "--rail-item",
      "--sidebar",
      "--surface",
      "--surface-hover",
      "--text",
      "--text-muted",
      "--text-faint",
      "--line",
      "--unread",
      "--danger",
      "--success",
      "--warning"
    ]) {
      expect(css).toContain(`${token}:`);
    }
  });

  test("contains no legacy purple or green literals", () => {
    for (const legacy of [
      "#34123d",
      "#4a1858",
      "#5b236a",
      "#1f6fb2",
      "#2f9d68",
      "--accent",
      "--brand-2",
      "--muted:",
      "--faint:"
    ]) {
      expect(css).not.toContain(legacy);
    }
  });

  test("selected room row uses a logical brand start bar", () => {
    expect(css).toMatch(/border-inline-start-color:\s*var\(--brand\)/);
    expect(css).not.toContain("box-shadow: inset 3px 0 0 0 var(--brand)");
  });

  test("locale-sensitive layout uses logical properties instead of physical left/right declarations", () => {
    expect(css).not.toMatch(
      /\b(?:left|right|margin-left|margin-right|padding-left|padding-right|border-left|border-right|inset-left|inset-right)\s*:/
    );
    expect(css).not.toMatch(/text-align:\s*(?:left|right)\b/);
  });
});
