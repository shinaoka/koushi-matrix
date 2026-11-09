// @vitest-environment jsdom
/**
 * The window is the only cap on the drawn picker size (#1839). The picker used
 * to draw `min(saved size, room the anchor leaves)`, so the same saved size
 * drew a shorter panel depending on where the message sat. It now draws the
 * saved size and scrolls its grid, which means a smaller panel can only mean
 * the window itself could not hold the size — the one case the picker reports,
 * as the dialog's description so a keyboard or screen-reader user gets it
 * without a new tab stop.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";

import { t } from "../i18n/messages";
import { EmojiPicker } from "./EmojiPicker";
import { FLOATING_LAYER_VIEWPORT_MARGIN_PX } from "./floatingLayer";

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

/** jsdom's window is 1024×768; the layer keeps its own margin on every side. */
const WINDOW_BLOCK_SIZE = 768 - 2 * FLOATING_LAYER_VIEWPORT_MARGIN_PX;

/** The mid-window anchor a message row would give the picker. */
const MID_WINDOW_ANCHOR = measuredRect(400, 340, 420, 300);

function renderPicker(size: [number, number]): HTMLElement {
  const trigger = document.createElement("button");
  trigger.getBoundingClientRect = () => MID_WINDOW_ANCHOR;
  document.body.append(trigger);

  render(
    <EmojiPicker
      anchorRef={{ current: trigger }}
      onClose={() => {}}
      onSelect={() => {}}
      onSizeChange={() => {}}
      size={size}
    />
  );
  return screen.getByRole("dialog", { name: t("composer.emoji") });
}

it("draws the saved size even when the anchor leaves less room than that", () => {
  // The anchor's own room would leave roughly half of this height; the picker
  // scrolls instead of pretending the size is smaller (#1839).
  const dialog = renderPicker([400, 700]);

  expect(dialog.style.blockSize).toBe("700px");
  expect(dialog.style.inlineSize).toBe("400px");
  expect(dialog.hasAttribute("aria-describedby")).toBe(false);
  expect(document.querySelector(".emoji-picker-clip")).toBeNull();
});

it("says the drawn size is smaller than the size it was given when the window narrows it", () => {
  const dialog = renderPicker([400, 900]);
  const expected = t("composer.emojiClipped", {
    drawn: `400×${WINDOW_BLOCK_SIZE}`,
    saved: "400×900"
  });

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

it("stays silent when the window can hold the box it draws", () => {
  const dialog = renderPicker([400, 300]);

  expect(document.querySelector(".emoji-picker-clip")).toBeNull();
  expect(dialog.hasAttribute("aria-describedby")).toBe(false);
});

it("makes the scrolling grid a labelled, focusable region", () => {
  const dialog = renderPicker([400, 700]);

  const region = screen.getByRole("region", { name: t("composer.emojiList") });
  expect(dialog.contains(region)).toBe(true);
  expect(region.tabIndex).toBe(0);
  expect(region.className).toContain("emoji-picker-body");
});
