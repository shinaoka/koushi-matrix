// @vitest-environment jsdom

import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EmojiPicker } from "./EmojiPicker";
import { EMOJI_BY_CATEGORY, EMOJI_CATEGORIES } from "./emojiData";

describe("EmojiPicker", () => {
  afterEach(() => {
    cleanup();
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

    fireEvent.keyDown(document, { key: "Escape" });

    expect(onClose).toHaveBeenCalled();
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
});
