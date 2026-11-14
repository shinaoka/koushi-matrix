// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import {
  captureAnchor,
  captureFreeScrollAnchor,
  capturePrependAnchor,
  wholePixelScrollTop
} from "./TimelineViewportAnchors";
import { linearVisibleNodeSpan, visibleNodeSpan } from "./TimelineViewportRows";

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

describe("visible range search (#1889)", () => {
  const box = { top: 10, bottom: 210 };
  const rows = (tops: number[]) => {
    const container = document.createElement("div");
    for (const [index, top] of tops.entries()) {
      const row = document.createElement("div");
      row.dataset.itemId = `row-${index}`;
      row.getBoundingClientRect = () => ({ top, bottom: top + 72, height: 72 } as DOMRect);
      container.appendChild(row);
    }
    return Array.from(container.children) as HTMLElement[];
  };

  it("finds the visible run without reading every row", () => {
    const nodes = rows(Array.from({ length: 200 }, (_, index) => index * 72 - 7000));
    let reads = 0;
    for (const node of nodes) {
      const rect = node.getBoundingClientRect.bind(node);
      node.getBoundingClientRect = () => {
        reads += 1;
        return rect();
      };
    }
    const span = visibleNodeSpan(box, nodes);
    expect(span).not.toBeNull();
    expect(span!.end - span!.start).toBeLessThan(5);
    expect(nodes.slice(span!.start, span!.end + 1).every((node) => {
      const rect = node.getBoundingClientRect();
      return rect.bottom > box.top && rect.top < box.bottom;
    })).toBe(true);
    expect(reads).toBeLessThan(nodes.length);
  });

  it("matches the linear scan when a moved row breaks DOM order", () => {
    // The latest-reply projection can leave a row at its old position while a
    // later row moves above it, so bottoms do not increase down the list.
    const tops = Array.from({ length: 60 }, (_, index) => index * 72 - 2000);
    tops[30] = -4000;
    const nodes = rows(tops);
    expect(visibleNodeSpan(box, nodes)).toEqual(linearVisibleNodeSpan(box, nodes));
    expect(visibleNodeSpan(box, nodes)).toEqual({ start: 27, end: 29 });
  });

  it("reports nothing when every row is outside the viewport", () => {
    expect(visibleNodeSpan(box, rows([-200, 900]))).toBeNull();
    expect(visibleNodeSpan(box, [])).toBeNull();
  });
});
