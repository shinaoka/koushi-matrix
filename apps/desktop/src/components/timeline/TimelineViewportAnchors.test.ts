// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { captureAnchor } from "./TimelineViewportAnchors";

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
