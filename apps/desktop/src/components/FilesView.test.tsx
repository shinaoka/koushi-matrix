// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FilesView } from "./FilesView";

afterEach(cleanup);

describe("FilesView", () => {
  it("does not apply a filename search for IME candidate-confirmation Enter", () => {
    const onChangeFilterSort = vi.fn();
    render(
      <FilesView
        filesView={{
          kind: "open",
          request_id: 1,
          scope: { kind: "account" },
          filter: {
            kinds: ["image", "video", "audio", "file", "sticker"],
            filename_query: null
          },
          sort: "newestFirst",
          items: [],
          selected_event_id: null
        }}
        onChangeFilterSort={onChangeFilterSort}
      />
    );
    const search = screen.getByRole("searchbox") as HTMLInputElement;

    fireEvent.compositionStart(search);
    fireEvent.change(search, { target: { value: "日本語" } });
    fireEvent.keyDown(search, {
      key: "Enter",
      code: "Enter",
      keyCode: 229,
      isComposing: true
    });

    expect(onChangeFilterSort).not.toHaveBeenCalled();
  });
});
