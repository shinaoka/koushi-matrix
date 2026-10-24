// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import {
  captureAnchor,
  captureFreeScrollAnchor,
  capturePrependAnchor,
  wholePixelScrollTop
} from "./TimelineViewportAnchors";

function viewport(tops: number[]) {
  const container = document.createElement("div");
  container.getBoundingClientRect = () => ({ top: 10, bottom: 210, height: 200 } as DOMRect);
  Object.defineProperty(container, "clientHeight", { value: 200 });
  tops.forEach((top, index) => {
    const row = document.createElement("div");
    row.dataset.itemId = `row-${index}`;
    row.getBoundingClientRect = () => ({ top, bottom: top + 72, height: 72 } as DOMRect);
    container.appendChild(row);
  });
  return container;
}

function threadRootViewport(tops: number[]) {
  const container = viewport(tops);
  for (const [index, node] of [...container.children].entries()) {
    node.setAttribute("data-content-event-id", `$root-${index}`);
    node.setAttribute("data-activity-event-id", `$row-${index}`);
  }
  return container;
}

describe("viewport anchor capture", () => {
  it("does not capture an old virtual window wholly below the viewport", () => {
    expect(captureAnchor(viewport([500, 572]))).toBeNull();
  });
  it("keeps a partially visible row and its negative offset", () => {
    expect(captureAnchor(viewport([-20, 52]))).toEqual({ itemId: "row-0", offsetTop: -30 });
  });
  it("excludes rows exactly beyond either edge", () => {
    expect(captureAnchor(viewport([-62, 210]))).toBeNull();
  });
});

describe("whole-pixel anchor writes (#1507)", () => {
  it("rounds to the nearest pixel instead of leaving truncation to the engine", () => {
    expect(wholePixelScrollTop(20553.71875)).toBe(20554);
    expect(wholePixelScrollTop(20724.40625)).toBe(20724);
  });
});

describe("pure prepend anchor (#1604)", () => {
  it("keeps latest-reply thread roots out of long-lived free-scroll memory", () => {
    expect(captureFreeScrollAnchor(threadRootViewport([-20, 52]))).toBeNull();
  });
  it("falls back to an intersecting thread root for prepend compensation", () => {
    expect(capturePrependAnchor(threadRootViewport([-20, 52]))).toEqual({
      itemId: "row-0",
      offsetTop: -30
    });
  });
});
