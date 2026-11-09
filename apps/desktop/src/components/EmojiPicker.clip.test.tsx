// @vitest-environment jsdom
/**
 * The saved picker size is kept when the layer narrows the drawn box to the
 * space around its anchor (#1583), so a smaller picker alone cannot tell the
 * user whether the size persisted. The picker says so only while it is drawing
 * a smaller box than the size it was given, and states the fact as the dialog's
 * description so a keyboard or screen-reader user gets it without a new tab
 * stop.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";

import { t } from "../i18n/messages";
import { EmojiPicker } from "./EmojiPicker";

afterEach(cleanup);

function measuredRect(top: number, right: number, bottom: number, left: number): DOMRect {
  return {
    bottom,
    height: bottom - top,
    left,
    right,
    top,
    width: right - left,
    x: left,
    y: top,
    toJSON: () => ({})
  } as DOMRect;
}

/**
 * The picker shares a tall boundary with the message row it is opened from; the
 * space an anchor leaves is what narrows a large saved size.
 */
function renderInBoundary(size: [number, number], boundaryTop: number): HTMLElement {
  const anchor = document.createElement("button");
  anchor.getBoundingClientRect = () => measuredRect(700, 120, 720, 100);
  const boundary = document.createElement("div");
  boundary.getBoundingClientRect = () => measuredRect(boundaryTop, 800, 752, 60);
  document.body.append(anchor, boundary);

  render(
    <EmojiPicker
      anchorRef={{ current: anchor }}
      onClose={() => {}}
      onSelect={() => {}}
      onSizeChange={() => {}}
      resolveBoundaryElement={() => boundary}
      size={size}
    />
  );
  return screen.getByRole("dialog", { name: t("composer.emoji") });
}

it("says the drawn size is smaller than the size it was given", () => {
  const dialog = renderInBoundary([400, 900], 16);
  // Above the low anchor, inside the boundary and below the viewport margin.
  const expected = t("composer.emojiClipped", { drawn: "400×678", saved: "400×900" });

  const describedBy = dialog.getAttribute("aria-describedby");
  expect(describedBy).toBeTruthy();
  expect(document.getElementById(describedBy as string)?.textContent).toBe(expected);

  // The visible affordance is a marker, not a control: no tab stop, no click,
  // and the header keeps its one button (#1583's keyboard handle lives on the
  // frame, not here).
  const marker = document.querySelector<HTMLElement>(".emoji-picker-clip");
  expect(marker).toBeTruthy();
  expect(marker?.tagName).not.toBe("BUTTON");
  expect(marker?.getAttribute("tabindex")).toBe("-1");
  expect(dialog.querySelectorAll(".emoji-picker-header button")).toHaveLength(1);
});

it("stays silent when the box it draws is the size it was given", () => {
  const dialog = renderInBoundary([400, 300], 16);

  expect(document.querySelector(".emoji-picker-clip")).toBeNull();
  expect(dialog.hasAttribute("aria-describedby")).toBe(false);
});
