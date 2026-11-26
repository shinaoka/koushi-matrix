import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

// #2136: the unsent bar is `position: absolute`; every host of a TimelineView
// must be its positioned ancestor or the bar escapes to a distant ancestor.
const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");

it.each([".message-list", ".thread-timeline-panel", ".focused-context-panel"])(
  "%s is a positioned ancestor for the docked unsent bar",
  (selector) => {
    const escaped = selector.replace(".", "\\.");
    const blocks = [...css.matchAll(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`, "g"))].map((m) => m[1]);
    expect(blocks.some((b) => /position:\s*relative/.test(b))).toBe(true);
  }
);
