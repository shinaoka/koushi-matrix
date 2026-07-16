// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { StagedUploadItem } from "../domain/types";
import { CreateEntityDialog, UploadStagingDialog } from "./dialogs";

afterEach(cleanup);

function stagedImage(
  caption: string,
  preparation: StagedUploadItem["preparation"],
  stagedId = "staged-1",
  filename = "synthetic.png"
): StagedUploadItem {
  return {
    staged_id: stagedId,
    room_id: "!synthetic:example.invalid",
    position: 0,
    filename,
    mime_type: "image/png",
    byte_count: 128,
    kind: { kind: "image", width: 16, height: 16 },
    caption: caption
      ? {
          plain_body: caption,
          formatted_body: null,
          mentions: { targets: [] }
        }
      : null,
    compression_choice: { kind: "original" },
    preparation
  };
}

function dialog(items: StagedUploadItem[], onUpdateCaption = vi.fn()) {
  return (
    <UploadStagingDialog
      items={items}
      onClear={vi.fn()}
      onUpdateCaption={onUpdateCaption}
      onSelectVariant={vi.fn()}
      onRetryPreparation={vi.fn()}
      onUseOriginal={vi.fn()}
      loadPreview={vi.fn(async () => [])}
    />
  );
}

describe("UploadStagingDialog", () => {
  it("preserves active Japanese composition across stale preparation snapshots", () => {
    const onUpdateCaption = vi.fn();
    const { rerender } = render(
      dialog([stagedImage("before", { kind: "preparing" })], onUpdateCaption)
    );
    const caption = screen.getByRole("textbox", {
      name: "Caption for synthetic.png"
    }) as HTMLInputElement;

    fireEvent.compositionStart(caption);
    fireEvent.change(caption, { target: { value: "日本語変換中" } });
    caption.setSelectionRange(3, 5);
    rerender(
      dialog(
        [
          stagedImage("before", {
            kind: "ready",
            variants: [],
            selected_variant_id: "original"
          })
        ],
        onUpdateCaption
      )
    );

    expect(caption.value).toBe("日本語変換中");
    expect([caption.selectionStart, caption.selectionEnd]).toEqual([3, 5]);
    expect(onUpdateCaption).toHaveBeenCalledWith("staged-1", "日本語変換中");
  });

  it("preserves an ordinary dirty caption until Rust acknowledges it", () => {
    const { rerender } = render(dialog([stagedImage("before", { kind: "preparing" })]));
    const caption = screen.getByRole("textbox", {
      name: "Caption for synthetic.png"
    }) as HTMLInputElement;

    fireEvent.change(caption, { target: { value: "local caption" } });
    rerender(dialog([stagedImage("before", { kind: "preparing" })]));

    expect(caption.value).toBe("local caption");
  });

  it("isolates composition ownership by staged upload identity", () => {
    const { rerender } = render(
      dialog([
        stagedImage("first", { kind: "preparing" }, "staged-a", "first.png"),
        stagedImage("second", { kind: "preparing" }, "staged-b", "second.png")
      ])
    );
    const first = screen.getByRole("textbox", {
      name: "Caption for first.png"
    }) as HTMLInputElement;
    const second = screen.getByRole("textbox", {
      name: "Caption for second.png"
    }) as HTMLInputElement;

    fireEvent.compositionStart(first);
    fireEvent.change(first, { target: { value: "一つ目を変換中" } });
    rerender(
      dialog([
        stagedImage("first", { kind: "preparing" }, "staged-a", "first.png"),
        stagedImage("second from Rust", { kind: "preparing" }, "staged-b", "second.png")
      ])
    );

    expect(first.value).toBe("一つ目を変換中");
    expect(second.value).toBe("second from Rust");
  });
});

describe("dialog IME submit handling", () => {
  it("does not create a room for candidate-confirmation Enter", () => {
    const onSubmit = vi.fn();
    const { container } = render(
      <CreateEntityDialog
        kind="room"
        isBusy={false}
        value="合成ルーム"
        onCancel={vi.fn()}
        onSubmit={onSubmit}
        onValueChange={vi.fn()}
      />
    );
    const name = screen.getByRole("textbox", { name: "Room name" });

    fireEvent.compositionStart(name);
    fireEvent.keyDown(name, {
      key: "Enter",
      code: "Enter",
      keyCode: 229,
      isComposing: true
    });
    fireEvent.submit(container.querySelector("form")!);

    expect(onSubmit).not.toHaveBeenCalled();
  });
});
