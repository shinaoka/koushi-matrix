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

describe("EmojiPicker", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders category tabs and an emoji grid", () => {
    render(<EmojiPicker onSelect={vi.fn()} onClose={vi.fn()} />);

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByRole("searchbox")).toBeTruthy();
    expect(screen.getByRole("tab", { name: /smileys/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: "grinning face" })).toBeTruthy();
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
