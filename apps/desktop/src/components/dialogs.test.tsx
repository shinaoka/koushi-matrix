// @vitest-environment jsdom

import { renderToStaticMarkup } from "react-dom/server";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ComposerDocument, StagedUploadItem } from "../domain/types";
import type { MentionCandidate } from "../domain/projectionTypes";
import { documentFromText } from "../domain/composerDocument";
import { t } from "../i18n/messages";
import {
  inlineMentionEditorSelection,
  setInlineMentionEditorSelection
} from "./ImeTextControl";
import {
  CreateEntityDialog,
  InviteTargetsDialog,
  ResetLocalDataConfirmationDialog,
  UploadStagingDialog
} from "./dialogs";
import type { InviteWorkflowState } from "../domain/types";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function stagedImage(
  caption: string | ComposerDocument,
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
    caption: typeof caption === "string" ? (caption ? documentFromText(caption) : null) : caption,
    compression_choice: { kind: "original" },
    preparation
  };
}

function changeEditorText(editor: Element, text: string) {
  const control = editor as HTMLDivElement;
  if (control.dataset.composing === "true") {
    let textNode = control.querySelector<HTMLElement>("[data-composer-text]");
    if (!textNode) {
      textNode = document.createElement("span");
      textNode.dataset.composerText = "";
      control.append(textNode);
    }
    textNode.textContent = text;
    fireEvent.input(control, { inputType: "insertCompositionText", isComposing: true });
    return;
  }
  setInlineMentionEditorSelection(control, 0, control.textContent?.length ?? 0);
  fireEvent(
    control,
    new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      inputType: "insertText",
      data: text
    })
  );
}

function dialog(items: StagedUploadItem[], onUpdateCaption = vi.fn()) {
  return (
    <UploadStagingDialog
      items={items}
      onClear={vi.fn()}
      onUpdateCaption={onUpdateCaption}
      onSelectOutput={vi.fn()}
      onSendAttachments={vi.fn()}
      onRetryPreparation={vi.fn()}
      onUseOriginal={vi.fn()}
      loadPreview={vi.fn(async () => [])}
      resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
      surface="main"
    />
  );
}

describe("ResetLocalDataConfirmationDialog", () => {
  it("renders an explicit destructive confirmation", () => {
    const markup = renderToStaticMarkup(
      <ResetLocalDataConfirmationDialog
        isBusy={false}
        onCancel={() => undefined}
        onConfirm={() => undefined}
      />
    );

    expect(markup).toContain('role="dialog"');
    expect(markup).toContain('aria-modal="true"');
    expect(markup).toContain('aria-label="Reset local data"');
    expect(markup).toContain(
      "Reset local data for this session? This removes the local Matrix store, cached history, and saved credentials on this device. This cannot be undone."
    );
    expect(markup).toContain(">Cancel</button>");
    expect(markup).toContain('class="dialog-button danger"');
    expect(markup).toContain(">Reset local data</button>");
  });
});

describe("UploadStagingDialog", () => {
  it("shows a prepared image before output controls and the compact caption editor", () => {
    render(
      dialog([
        stagedImage("", {
          kind: "ready",
          variants: [],
          selected: { resize: "original", format: "keep" },
          pending: null,
          generation: 0
        })
      ])
    );
    const item = document.querySelector(".upload-staging-item");
    const list = document.querySelector(".upload-staging-list");
    const preview = item?.querySelector(".upload-preview-viewport");
    const toolbar = item?.querySelector(".upload-output-toolbar");
    const caption = item?.querySelector(".upload-staging-caption");

    expect(list?.classList.contains("is-single")).toBe(true);
    expect(item?.classList.contains("has-preview")).toBe(true);
    expect(preview).not.toBeNull();
    expect(toolbar).not.toBeNull();
    expect(caption).not.toBeNull();
    expect(preview!.compareDocumentPosition(toolbar!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(toolbar!.compareDocumentPosition(caption!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(within(item as HTMLElement).queryByText("Caption for synthetic.png")).toBeNull();
    expect(screen.getByRole("textbox", { name: "Caption for synthetic.png" })).toBeTruthy();
  });

  it("renders attachment staging as a modal outside its clipped composer container", () => {
    const { container } = render(dialog([stagedImage("", { kind: "preparing" })]));
    const modal = screen.getByRole("dialog", { name: t("upload.dialogTitle") });
    expect(modal.getAttribute("aria-modal")).toBe("true");
    expect(container.contains(modal)).toBe(false);
    expect(document.body.contains(modal)).toBe(true);
    expect(screen.queryByRole("group", { name: t("upload.previewMode") })).toBeNull();
  });

  it("renders the shared caption editor and emits a structured document from formatting", () => {
    const onUpdateCaption = vi.fn();
    render(dialog([stagedImage("caption", { kind: "preparing" })], onUpdateCaption));

    expect(screen.getByRole("button", { name: /bold/i })).toBeTruthy();
    const caption = screen.getByRole("textbox", {
      name: "Caption for synthetic.png"
    });
    caption.focus();
    fireEvent.click(screen.getByRole("button", { name: /bold/i }));

    expect(onUpdateCaption).toHaveBeenLastCalledWith(
      "staged-1",
      expect.objectContaining({ version: 2, inlines: expect.any(Array) })
    );
  });

  it("preserves active Japanese composition across stale preparation snapshots", () => {
    const onUpdateCaption = vi.fn();
    const { rerender } = render(
      dialog([stagedImage("before", { kind: "preparing" })], onUpdateCaption)
    );
    const caption = screen.getByRole("textbox", {
      name: "Caption for synthetic.png"
    }) as HTMLDivElement;

    fireEvent.compositionStart(caption);
    changeEditorText(caption, "日本語変換中");
    setInlineMentionEditorSelection(caption, 3, 5);
    rerender(
      dialog(
        [
          stagedImage("before", {
            kind: "ready",
            variants: [],
            selected: { resize: "original", format: "keep" },
      pending: null,
      generation: 0
          })
        ],
        onUpdateCaption
      )
    );

    expect(caption.textContent).toBe("日本語変換中");
    expect(inlineMentionEditorSelection(caption)).toMatchObject({ start: 3, end: 5 });
    fireEvent.compositionEnd(caption);
    expect(onUpdateCaption).toHaveBeenCalledWith(
      "staged-1",
      expect.objectContaining({ version: 2, inlines: [{ kind: "text", text: "日本語変換中" }] })
    );
  });

  it("preserves an ordinary dirty caption until Rust acknowledges it", () => {
    const { rerender } = render(dialog([stagedImage("before", { kind: "preparing" })]));
    const caption = screen.getByRole("textbox", {
      name: "Caption for synthetic.png"
    }) as HTMLDivElement;

    changeEditorText(caption, "local caption");
    rerender(dialog([stagedImage("before", { kind: "preparing" })]));

    expect(caption.textContent).toBe("local caption");
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
    }) as HTMLDivElement;
    const second = screen.getByRole("textbox", {
      name: "Caption for second.png"
    }) as HTMLDivElement;

    fireEvent.compositionStart(first);
    changeEditorText(first, "一つ目を変換中");
    rerender(
      dialog([
        stagedImage("first", { kind: "preparing" }, "staged-a", "first.png"),
        stagedImage("second from Rust", { kind: "preparing" }, "staged-b", "second.png")
      ])
    );

    expect(first.textContent).toBe("一つ目を変換中");
    expect(second.textContent).toBe("second from Rust");
  });

  it.each(["button", "keyboard"] as const)(
    "waits for caption update before %s send",
    async (trigger) => {
      let resolveUpdate!: () => void;
      const update = new Promise<void>((resolve) => {
        resolveUpdate = resolve;
      });
      const onUpdateCaption = vi.fn(() => update);
      const onSendAttachments = vi.fn();
      const resolveComposerKeyAction = vi.fn(async () => "send" as const);
      render(
        <UploadStagingDialog
          items={[stagedImage("", { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 })]}
          onClear={vi.fn()}
          onUpdateCaption={onUpdateCaption}
          onSelectOutput={vi.fn()}
          onRetryPreparation={vi.fn()}
          onUseOriginal={vi.fn()}
          onSendAttachments={onSendAttachments}
          loadPreview={vi.fn(async () => [])}
          resolveComposerKeyAction={resolveComposerKeyAction}
          surface="main"
        />
      );
      const caption = screen.getByRole("textbox", { name: "Caption for synthetic.png" });
      changeEditorText(caption, "caption");
      if (trigger === "button") {
        fireEvent.click(screen.getByRole("button", { name: "Send attachments" }));
      } else {
        fireEvent.keyDown(caption, { key: "Enter", code: "Enter" });
      }
      expect(onSendAttachments).not.toHaveBeenCalled();

      resolveUpdate();
      await waitFor(() => expect(onSendAttachments).toHaveBeenCalledTimes(1));
    }
  );

  it("resolves caption Enter and sends attachments only for send", async () => {
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);
    const onSendAttachments = vi.fn();
    render(
      <UploadStagingDialog
        items={[stagedImage("", { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 })]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={onSendAttachments}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={resolveComposerKeyAction}
        surface="main"
      />
    );

    fireEvent.keyDown(screen.getByRole("textbox", { name: "Caption for synthetic.png" }), {
      key: "Enter",
      code: "Enter"
    });

    await waitFor(() => expect(onSendAttachments).toHaveBeenCalledTimes(1));
    expect(resolveComposerKeyAction).toHaveBeenCalledWith(
      "main",
      expect.objectContaining({ key: "enter", is_composing: false }),
      { autocomplete_open: false, send_enabled: true }
    );
  });

  it("does not send when the caption resolver returns another action", async () => {
    const resolveComposerKeyAction = vi.fn(async () => "insertNewline" as const);
    const onSendAttachments = vi.fn();
    render(
      <UploadStagingDialog
        items={[stagedImage("", { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 })]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={onSendAttachments}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={resolveComposerKeyAction}
        surface="thread"
      />
    );

    fireEvent.keyDown(screen.getByRole("textbox", { name: "Caption for synthetic.png" }), {
      key: "Enter",
      code: "Enter"
    });

    await waitFor(() => expect(resolveComposerKeyAction).toHaveBeenCalledTimes(1));
    expect(onSendAttachments).not.toHaveBeenCalled();
  });

  it("accepts a staged mention before forwarding Tab to Send", () => {
    const onUpdateCaption = vi.fn();
    const onSendAttachments = vi.fn();
    const candidate: MentionCandidate = {
      key: "@alice:example.invalid",
      label: "Alice",
      target: {
        kind: "user",
        user_id: "@alice:example.invalid",
        display_label: "Alice"
      }
    };
    render(
      <UploadStagingDialog
        items={[stagedImage(documentFromText("@a"), { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 })]}
        onClear={vi.fn()}
        onUpdateCaption={onUpdateCaption}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={onSendAttachments}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
        mentionCandidates={[candidate]}
      />
    );

    const caption = screen.getByRole("textbox", { name: "Caption for synthetic.png" });
    const send = screen.getByRole("button", { name: "Send attachments" });
    fireEvent.keyDown(caption, { key: "Tab", code: "Tab" });

    expect(onSendAttachments).not.toHaveBeenCalled();
    expect(document.activeElement).not.toBe(send);
    expect(onUpdateCaption).toHaveBeenLastCalledWith(
      "staged-1",
      expect.objectContaining({
        inlines: [
          {
            kind: "mention",
            target: { kind: "user", user_id: "@alice:example.invalid", display_label: "Alice" },
            display_label: "Alice"
          },
          { kind: "text", text: " " }
        ]
      })
    );
  });

  it("does not intercept composing Tab for the attachment Send button", () => {
    const onSendAttachments = vi.fn();
    render(
      <UploadStagingDialog
        items={[stagedImage("", { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 })]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={onSendAttachments}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
      />
    );
    const caption = screen.getByRole("textbox", { name: "Caption for synthetic.png" });
    const send = screen.getByRole("button", { name: "Send attachments" });
    fireEvent.compositionStart(caption);
    const event = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
      isComposing: true
    });
    caption.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
    expect(document.activeElement).not.toBe(send);
    expect(onSendAttachments).not.toHaveBeenCalled();
  });

  it("leaves composing Enter to ImeTextField", async () => {
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);
    const onSendAttachments = vi.fn();
    render(
      <UploadStagingDialog
        items={[stagedImage("", { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 })]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={onSendAttachments}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={resolveComposerKeyAction}
        surface="main"
      />
    );
    const caption = screen.getByRole("textbox", { name: "Caption for synthetic.png" });
    fireEvent.compositionStart(caption);
    fireEvent.keyDown(caption, {
      key: "Enter",
      code: "Enter",
      keyCode: 229,
      isComposing: true
    });

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(resolveComposerKeyAction).not.toHaveBeenCalled();
    expect(onSendAttachments).not.toHaveBeenCalled();
  });

  it("moves forward Tab from the last sendable caption to Send", () => {
    render(
      <UploadStagingDialog
        items={[
          stagedImage("", { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 }),
          stagedImage("", { kind: "ready", variants: [], selected: { resize: "original", format: "keep" }, pending: null, generation: 0 }, "staged-2", "second.png")
        ]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={vi.fn()}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
      />
    );
    const first = screen.getByRole("textbox", { name: "Caption for synthetic.png" });
    const last = screen.getByRole("textbox", { name: "Caption for second.png" });
    const send = screen.getByRole("button", { name: "Send attachments" });

    first.focus();
    fireEvent.keyDown(first, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    expect(document.activeElement).toBe(send);
  });

  it("keeps native Tab behavior for disabled Send, earlier captions, and Shift+Tab", () => {
    render(
      <UploadStagingDialog
        items={[
          stagedImage("", { kind: "preparing" }, "staged-1", "first.png"),
          stagedImage("", { kind: "preparing" }, "staged-2", "second.png")
        ]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={vi.fn()}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
      />
    );
    const first = screen.getByRole("textbox", { name: "Caption for first.png" });
    const last = screen.getByRole("textbox", { name: "Caption for second.png" });
    const send = screen.getByRole<HTMLButtonElement>("button", { name: "Send attachments" });

    first.focus();
    const earlierTab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    first.dispatchEvent(earlierTab);
    expect(earlierTab.defaultPrevented).toBe(false);
    last.focus();
    const disabledTab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    last.dispatchEvent(disabledTab);
    expect(disabledTab.defaultPrevented).toBe(false);
    const shiftTab = new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true });
    last.dispatchEvent(shiftTab);
    expect(shiftTab.defaultPrevented).toBe(false);
    for (const modifier of ["ctrlKey", "metaKey", "altKey"] as const) {
      const modifiedTab = new KeyboardEvent("keydown", {
        key: "Tab",
        [modifier]: true,
        bubbles: true,
        cancelable: true
      });
      last.dispatchEvent(modifiedTab);
      expect(modifiedTab.defaultPrevented).toBe(false);
    }
    expect(send.disabled).toBe(true);
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

describe("InviteTargetsDialog history policy", () => {
  function workflow(): InviteWorkflowState {
    return {
      query: {
        room_id: "!room:example.invalid",
        query: "alice",
        candidates: [],
        explicit_user_id: null
      },
      selected_targets: [],
      scope_plan: {
        room_id: "!room:example.invalid",
        destination_kind: "room",
        default_scope: { kind: "roomOnly" },
        options: [{ scope: { kind: "roomOnly" }, label: "Room only", detail: null }]
      },
      selected_scope: { kind: "roomOnly" },
      history_policy: {
        current_visibility: "shared",
        encrypted: true,
        can_edit: true,
        readiness: "recoveryRequired"
      },
      operation: { kind: "idle" }
    };
  }

  it("explains all normal history choices and exposes Room Info and Recovery", () => {
    const onOpenRoomInfo = vi.fn();
    const onOpenRecovery = vi.fn();
    render(
      <InviteTargetsDialog
        isBusy={false}
        query="alice"
        title={t("dialog.invitePeopleTitle")}
        workflow={workflow()}
        onCancel={vi.fn()}
        onOpenRecovery={onOpenRecovery}
        onOpenRoomInfo={onOpenRoomInfo}
        onQueryChange={vi.fn()}
        onRemoveTarget={vi.fn()}
        onScopeChange={vi.fn()}
        onSelectCandidate={vi.fn()}
        onSubmit={vi.fn()}
      />
    );

    expect(screen.getByText("Shared history")).toBeTruthy();
    expect(screen.getByText("Since invite")).toBeTruthy();
    expect(screen.getByText("Since join")).toBeTruthy();
    expect(screen.getByText("Current")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Open Room Info" }));
    fireEvent.click(screen.getByRole("button", { name: "Open recovery" }));
    expect(onOpenRoomInfo).toHaveBeenCalledTimes(1);
    expect(onOpenRecovery).toHaveBeenCalledTimes(1);
  });
});

// --- #305: compact resize/format controls ---

function preparedVariant(
  resize: "original" | "half" | "quarter" | "eighth",
  formatChoice: "keep" | "png" | "jpeg" | "webp",
  overrides: Partial<{ byte_count: number; width: number; height: number; savings_percent: number }> = {}
) {
  return {
    variant_id: `${resize}-${formatChoice}`,
    resize,
    format_choice: formatChoice,
    filename: `synthetic.${formatChoice === "keep" ? "png" : formatChoice}`,
    mime_type: formatChoice === "keep" ? "image/png" : `image/${formatChoice}`,
    byte_count: overrides.byte_count ?? 128,
    width: overrides.width ?? 1284,
    height: overrides.height ?? 918,
    format: (formatChoice === "keep" ? "original" : formatChoice) as
      | "original"
      | "png"
      | "jpeg"
      | "webp",
    savings_percent: overrides.savings_percent ?? 0,
    metadata_stripped: false,
    thumbnail_refreshed: false
  };
}

describe("UploadStagingDialog resize and format controls", () => {
  it("offers the four scales and four formats as compact radiogroups", () => {
    render(
      dialog([
        stagedImage("", {
          kind: "ready",
          variants: [preparedVariant("original", "keep")],
          selected: { resize: "original", format: "keep" },
          pending: null,
          generation: 0
        })
      ])
    );

    const resize = screen.getByRole("radiogroup", { name: "Resize" });
    expect(
      ["Original", "1/2", "1/4", "1/8"].map(
        (label) => within(resize).getByRole("radio", { name: label }).getAttribute("aria-checked")
      )
    ).toEqual(["true", "false", "false", "false"]);

    const format = screen.getByRole("radiogroup", { name: "Format" });
    expect(
      ["Keep", "WebP", "JPEG", "PNG"].map(
        (label) => within(format).getByRole("radio", { name: label }).getAttribute("aria-checked")
      )
    ).toEqual(["true", "false", "false", "false"]);

    // No large per-variant cards and no MIME strings survive.
    expect(document.querySelector(".upload-variant-button")).toBeNull();
    expect(screen.queryByText(/image\/png/)).toBeNull();
  });

  it("dispatches the chosen pair without changing the rendered selection itself", () => {
    const onSelectOutput = vi.fn();
    render(
      <UploadStagingDialog
        items={[
          stagedImage("", {
            kind: "ready",
            variants: [preparedVariant("original", "keep")],
            selected: { resize: "original", format: "keep" },
            pending: null,
            generation: 0
          })
        ]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={onSelectOutput}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={vi.fn()}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
      />
    );

    fireEvent.click(screen.getByRole("radio", { name: "1/2" }));
    expect(onSelectOutput).toHaveBeenCalledWith("staged-1", {
      resize: "half",
      format: "keep"
    });
    fireEvent.click(screen.getByRole("radio", { name: "JPEG" }));
    expect(onSelectOutput).toHaveBeenLastCalledWith("staged-1", {
      resize: "original",
      format: "jpeg"
    });
    // Rust owns the selection: the pressed state still reflects the snapshot.
    expect(
      screen.getByRole("radio", { name: "Original" }).getAttribute("aria-checked")
    ).toBe("true");
  });

  it("summarizes the selected output once and reports recompression", () => {
    const ready = stagedImage("", {
      kind: "ready",
      variants: [preparedVariant("half", "jpeg", { byte_count: 4096, width: 642, height: 459, savings_percent: 60 })],
      selected: { resize: "half", format: "jpeg" },
      pending: null,
      generation: 1
    });
    const { rerender } = render(dialog([ready]));

    const summary = screen.getByRole("status", { name: "Upload result" });
    expect(summary.textContent).toContain("642");
    expect(summary.textContent).toContain("459");
    expect(summary.textContent).toContain("60% smaller");
    expect(summary.textContent).not.toContain("image/jpeg");

    rerender(
      dialog([
        stagedImage("", {
          kind: "ready",
          variants: [preparedVariant("half", "jpeg", { byte_count: 4096, width: 642, height: 459 })],
          selected: { resize: "quarter", format: "jpeg" },
          pending: { resize: "quarter", format: "jpeg" },
          generation: 2
        })
      ])
    );

    // While recompressing, the summary reports the state instead of numbers
    // that would describe bytes nobody is going to upload.
    const pendingSummary = screen.getByRole("status", { name: "Upload result" });
    expect(pendingSummary.textContent).toContain("Recompressing…");
    expect(pendingSummary.getAttribute("data-upload-output-state")).toBe("recompressing");
    // The preview viewport stays mounted at a stable height.
    const viewport = document.querySelector(".upload-preview-viewport");
    expect(viewport).not.toBeNull();
    expect(viewport?.getAttribute("data-recompressing")).toBe("true");
  });
});

describe("UploadStagingDialog preview URL lifecycle", () => {
  it("keeps the active preview URL across caption-only rerenders", async () => {
    const preparation: StagedUploadItem["preparation"] = {
      kind: "ready",
      variants: [preparedVariant("original", "keep")],
      selected: { resize: "original", format: "keep" },
      pending: null,
      generation: 0
    };
    const loadPreview = vi.fn(async () => [1, 2, 3]);
    const createObjectURL = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:stable-upload-preview");
    const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
    const props = {
      onClear: vi.fn(),
      onUpdateCaption: vi.fn(),
      onSelectOutput: vi.fn(),
      onRetryPreparation: vi.fn(),
      onUseOriginal: vi.fn(),
      onSendAttachments: vi.fn(),
      loadPreview,
      resolveComposerKeyAction: vi.fn(async () => "noop" as const),
      surface: "main" as const
    };

    const { rerender, unmount } = render(
      <UploadStagingDialog items={[stagedImage("before", preparation)]} {...props} />
    );
    await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(1));

    rerender(<UploadStagingDialog items={[stagedImage("after", preparation)]} {...props} />);

    expect(loadPreview).toHaveBeenCalledTimes(1);
    expect(createObjectURL).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).not.toHaveBeenCalled();

    unmount();
    expect(revokeObjectURL).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:stable-upload-preview");
  });

  it("revokes the active preview URL when the dialog unmounts", async () => {
    const loadPreview = vi.fn(async () => [1, 2, 3]);
    const createObjectURL = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:upload-preview");
    const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);

    const { unmount } = render(
      <UploadStagingDialog
        items={[
          stagedImage("", {
            kind: "ready",
            variants: [preparedVariant("original", "keep")],
            selected: { resize: "original", format: "keep" },
            pending: null,
            generation: 0
          })
        ]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={vi.fn()}
        loadPreview={loadPreview}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
      />
    );

    await waitFor(() => expect(screen.getByRole("img", { name: "Prepared attachment preview" })).toBeTruthy());
    expect(createObjectURL).toHaveBeenCalledTimes(1);

    unmount();

    expect(revokeObjectURL).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:upload-preview");
  });
});

describe("UploadStagingDialog send action", () => {
  function readyItem(stagedId = "staged-1") {
    return stagedImage(
      "",
      {
        kind: "ready",
        variants: [preparedVariant("original", "keep")],
        selected: { resize: "original", format: "keep" },
        pending: null,
        generation: 0
      },
      stagedId
    );
  }

  function sendableDialog(
    items: StagedUploadItem[],
    onSendAttachments = vi.fn()
  ) {
    return (
      <UploadStagingDialog
        items={items}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        onSendAttachments={onSendAttachments}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
      />
    );
  }

  it("owns a distinctly labeled send action for the attachments", async () => {
    const onSendAttachments = vi.fn();
    render(sendableDialog([readyItem()], onSendAttachments));

    // A second button literally named "Send" on the same screen is what made
    // the two actions indistinguishable.
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    const send = screen.getByRole("button", { name: "Send attachments" });
    fireEvent.click(send);
    await waitFor(() => expect(onSendAttachments).toHaveBeenCalledTimes(1));
  });

  it("refuses to send while an output is still recompressing", () => {
    const onSendAttachments = vi.fn();
    render(
      sendableDialog(
        [
          stagedImage("", {
            kind: "ready",
            variants: [preparedVariant("original", "keep")],
            selected: { resize: "half", format: "keep" },
            pending: { resize: "half", format: "keep" },
            generation: 1
          })
        ],
        onSendAttachments
      )
    );

    const send = screen.getByRole<HTMLButtonElement>("button", {
      name: "Send attachments"
    });
    expect(send.disabled).toBe(true);
    fireEvent.click(send);
    expect(onSendAttachments).not.toHaveBeenCalled();
  });

  it("refuses to send while any attachment is still preparing", () => {
    render(sendableDialog([readyItem(), stagedImage("", { kind: "preparing" }, "staged-2")]));

    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: "Send attachments" }).disabled
    ).toBe(true);
  });
});
