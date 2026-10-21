// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { EmojiPicker } from "./EmojiPicker";

afterEach(cleanup);

it("puts a resize zone on every edge and corner with one keyboard handle", () => {
  render(
    <EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} size={[400, 400]} onSizeChange={vi.fn()} />,
  );
  const zones = [...document.querySelectorAll("[data-emoji-picker-resize]")];
  expect(zones.map((zone) => zone.getAttribute("data-emoji-picker-resize")).sort()).toEqual([
    "bottom",
    "bottom-left",
    "bottom-right",
    "left",
    "right",
    "top",
    "top-left",
    "top-right",
  ]);
  // Pointer-only zones stay out of the tab order; the marked corner is the
  // single accessible handle (#1583).
  expect(document.querySelectorAll("button[data-emoji-picker-resize]")).toHaveLength(1);
  const marked = document.querySelector<HTMLElement>(".emoji-picker-resize.is-marked");
  expect(marked?.tagName).toBe("BUTTON");
  expect(marked).toBe(screen.getByRole("button", { name: /resize emoji picker/i }));
});

it("offers keyboard resizing on the marked corner and commits the requested size", () => {
  const onSizeChange = vi.fn();
  render(
    <EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} size={[400, 400]} onSizeChange={onSizeChange} />,
  );
  const handle = screen.getByRole("button", { name: /resize emoji picker/i });
  // The arrow keys move the marked corner's own edges, the same way a pointer
  // drag does.
  const zone = handle.getAttribute("data-emoji-picker-resize") ?? "";
  const horizontal = zone.includes("left") ? -1 : 1;
  const vertical = zone.includes("top") ? -1 : 1;
  fireEvent.keyDown(handle, { key: "ArrowRight" });
  expect(onSizeChange).toHaveBeenLastCalledWith([400 + horizontal * 20, 400]);
  fireEvent.keyDown(handle, { key: "ArrowDown" });
  expect(onSizeChange).toHaveBeenLastCalledWith([400 + horizontal * 20, 400 + vertical * 20]);
});
