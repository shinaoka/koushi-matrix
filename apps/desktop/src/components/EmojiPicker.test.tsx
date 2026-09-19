// @vitest-environment jsdom

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  EMOJI_PICKER_GRID_COLUMNS,
  EMOJI_PICKER_INLINE_SIZE_PX,
  EmojiPicker,
} from "./EmojiPicker";
import { EMOJI_BY_CATEGORY, EMOJI_CATEGORIES } from "./emojiData";

describe("EmojiPicker", () => {
  afterEach(() => {
    cleanup();
    // Recent emojis are persisted, so a selection in one test must not change
    // which grid another test renders first.
    localStorage.clear();
  });

  it("renders category tabs and an emoji grid", () => {
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByRole("searchbox")).toBeTruthy();
    expect(screen.getByRole("tab", { name: /smileys & people/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: "grinning face" })).toBeTruthy();
  });

  it("uses Element-compatible emoji categories and data coverage", () => {
    expect(EMOJI_CATEGORIES).toEqual([
      "people",
      "nature",
      "foods",
      "activity",
      "places",
      "objects",
      "symbols",
      "flags",
    ]);
    expect((EMOJI_BY_CATEGORY as Record<string, unknown[]>)["flags"]?.length).toBeGreaterThan(200);
    expect(
      EMOJI_CATEGORIES.reduce(
        (total, category) => total + EMOJI_BY_CATEGORY[category].length,
        0,
      ),
    ).toBeGreaterThan(1_500);
  });

  it("calls onSelect and onClose when an emoji is clicked", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(<EmojiPicker onSelect={onSelect} onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: /grinning face$/i }));

    expect(onSelect).toHaveBeenCalledWith("😀");
    expect(onClose).toHaveBeenCalled();
  });

  it("filters emojis by search query", async () => {
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);

    const searchbox = screen.getByRole("searchbox");
    fireEvent.change(searchbox, { target: { value: "pizza" } });

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /pizza/i })).toBeTruthy();
    });
    expect(screen.queryByRole("button", { name: "grinning face" })).toBeNull();
  });

  it("searches Element emoji shortcodes and flags", async () => {
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);

    const searchbox = screen.getByRole("searchbox");
    fireEvent.change(searchbox, { target: { value: "checkered_flag" } });

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /flag/i })).toBeTruthy();
    });
  });

  it("shows an empty state when search has no matches", async () => {
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);

    const searchbox = screen.getByRole("searchbox");
    fireEvent.change(searchbox, { target: { value: "xyzabc" } });

    await waitFor(() => {
      expect(screen.getByText(/no emojis match/i)).toBeTruthy();
    });
  });

  it("closes when Escape is pressed", () => {
    const onClose = vi.fn();
    render(<EmojiPicker onSelect={vi.fn()} onClose={onClose} />);

    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Escape" });

    expect(onClose).toHaveBeenCalled();
  });

  it("leaves composition Escape to the IME", () => {
    const onClose = vi.fn();
    render(<EmojiPicker onSelect={vi.fn()} onClose={onClose} />);
    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Escape", isComposing: true });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("closes when clicking outside", () => {
    const onClose = vi.fn();
    render(
      <div>
        <EmojiPicker onSelect={vi.fn()} onClose={onClose} />
        <button type="button" data-testid="outside" />
      </div>,
    );

    fireEvent.mouseDown(screen.getByTestId("outside"));

    expect(onClose).toHaveBeenCalled();
  });

  it("renders in a viewport-fixed floating layer outside the anchor subtree", () => {
    function Host() {
      const anchorRef = useRef<HTMLButtonElement>(null);
      return (
        <div data-testid="clipped-pane" style={{ overflow: "hidden" }}>
          <button ref={anchorRef} type="button" data-testid="anchor" />
          <EmojiPicker anchorRef={anchorRef} onSelect={vi.fn()} onClose={vi.fn()} />
        </div>
      );
    }
    render(<Host />);

    const panel = screen.getByRole("dialog");
    expect(panel.parentElement).toBe(document.body);
    expect(screen.getByTestId("clipped-pane").contains(panel)).toBe(false);
    expect(panel.style.getPropertyValue("inline-size")).toBe(
      `${EMOJI_PICKER_INLINE_SIZE_PX}px`,
    );
    expect(panel.style.left).not.toBe("");
    expect(panel.style.top).not.toBe("");
  });

  it("keeps the rendered grid column count and the keyboard step in sync", () => {
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);

    const grid = screen.getAllByRole("grid")[0];
    expect(grid.style.getPropertyValue("--emoji-picker-columns")).toBe(
      String(EMOJI_PICKER_GRID_COLUMNS),
    );

    const cells = within(grid).getAllByRole("button");
    cells[0].focus();
    fireEvent.keyDown(cells[0], { key: "ArrowDown" });

    expect(document.activeElement).toBe(cells[EMOJI_PICKER_GRID_COLUMNS]);
  });
});
