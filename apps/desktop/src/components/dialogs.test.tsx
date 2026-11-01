// @vitest-environment jsdom

import { renderToStaticMarkup } from "react-dom/server";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ComposerDocument, CreateRoomAccessPreview, StagedUploadItem } from "../domain/types";
import type { MentionCandidate } from "../domain/projectionTypes";
import { documentFromText } from "../domain/composerDocument";
import { t } from "../i18n/messages";
import {
  inlineMentionEditorSelection,
  setInlineMentionEditorSelection
} from "./ImeTextControl";
import {
  CreateEntityDialog,
  DirectoryPreviewDialog,
  InviteTargetsDialog,
  LeaveSpaceDialog,
  ResetLocalDataConfirmationDialog,
  SuggestedRoomsDialog,
  UploadStagingDialog
} from "./dialogs";
import type {
  DirectoryPreviewState,
  InviteWorkflowState,
  SpaceLeaveCandidate
} from "../domain/types";

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

    expect(markup).toContain("<dialog ");
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

  it("opens on the first caption editor, never the clear action", () => {
    render(
      dialog([
        stagedImage("", { kind: "preparing" }, "staged-1", "first.png"),
        stagedImage("", { kind: "preparing" }, "staged-2", "second.png")
      ])
    );
    const caption = screen.getByRole("textbox", { name: "Caption for first.png" });

    // The clear action is destructive and explains itself with a bubble, so an
    // attachment never opens the staging dialog on it.
    expect(document.activeElement).toBe(caption);
    expect(document.activeElement).not.toBe(screen.getByRole("button", { name: t("upload.clear") }));
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("cancels the staging dialog from an Escape resolved in the caption", async () => {
    const onClear = vi.fn();
    const resolveComposerKeyAction = vi.fn(async () => "cancel" as const);
    render(
      <UploadStagingDialog
        items={[stagedImage("", { kind: "preparing" })]}
        onClear={onClear}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onSendAttachments={vi.fn()}
        onRetryPreparation={vi.fn()}
        onUseOriginal={vi.fn()}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={resolveComposerKeyAction}
        surface="main"
      />
    );

    // The caption consumes Escape before the modal sees it, so the dialog's
    // cancel path has to be reachable from the editor it opens on.
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Caption for synthetic.png" }), {
      key: "Escape",
      code: "Escape"
    });

    await waitFor(() => expect(onClear).toHaveBeenCalledTimes(1));
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

describe("CreateEntityDialog room access", () => {
  it("offers Space members, invite and Anyone in a Space and maps the choice", () => {
    const onRoomOptionsChange = vi.fn();
    const onSetRoomAccessDraft = vi.fn();
    const view = (visibility: "private" | "public") => (
      <CreateEntityDialog
        targetSpaceName="春学"
        kind="room"
        isBusy={false}
        roomOptions={{
          aliasLocalpart: "",
          encrypted: true,
          invitedOnly: false,
          topic: "",
          visibility
        }}
        value="秘密ルーム"
        onCancel={vi.fn()}
        onRoomOptionsChange={onRoomOptionsChange}
        onSubmit={vi.fn()}
        onValueChange={vi.fn()}
        onSetRoomAccessDraft={onSetRoomAccessDraft}
      />
    );
    const { rerender } = render(view("private"));

    expect(screen.getByRole("group", { name: t("dialog.createWhoCanJoin") })).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: t("dialog.createJoinPublic") }));
    expect(onSetRoomAccessDraft).toHaveBeenCalledWith({
      kind: "rule",
      scope: { kind: "create", sessionId: 0 },
      rule: "public"
    });
    expect(onRoomOptionsChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ visibility: "public" })
    );

    fireEvent.click(
      screen.getByRole("radio", { name: t("dialog.createJoinSpaceMembers", { spaceName: "春学" }) })
    );
    expect(onSetRoomAccessDraft).toHaveBeenCalledWith({
      kind: "rule",
      scope: { kind: "create", sessionId: 0 },
      rule: "restricted"
    });
    expect(onRoomOptionsChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ visibility: "private", invitedOnly: false })
    );

    rerender(view("public"));
    fireEvent.click(screen.getByRole("radio", { name: t("dialog.createJoinInvite") }));
    expect(onSetRoomAccessDraft).toHaveBeenCalledWith({
      kind: "rule",
      scope: { kind: "create", sessionId: 0 },
      rule: "invite"
    });
  });

  it("offers only invite and Anyone at Home (#1416)", () => {
    render(
      <CreateEntityDialog
        kind="room"
        isBusy={false}
        value=""
        onCancel={vi.fn()}
        onSubmit={vi.fn()}
        onValueChange={vi.fn()}
      />
    );
    const radios = within(screen.getByRole("group", { name: t("dialog.createWhoCanJoin") }))
      .getAllByRole("radio")
      .map((radio) => radio.getAttribute("value"));
    expect(radios).toEqual(["invite", "public"]);
  });

  it("offers Ask to join at Home only when Rust reports it available (#1421)", () => {
    const preview = (knockAvailable: boolean): CreateRoomAccessPreview => ({
      scope: { kind: "create" as const, sessionId: 1 },
      confirmed: false,
      effectiveRule: "invite" as const,
      effectiveHistory: "shared" as const,
      roomVersionPinned: false,
      knockAvailable,
      knockRestrictedAvailable: false,
      outcome: {
        join: { messageId: "room.accessOutcomeJoinInvite" },
        history: { messageId: "room.accessOutcomeHistoryShared" },
        encryption: { messageId: "room.accessOutcomeEncrypted" },
        directory: { messageId: "room.accessOutcomeDirectoryLoading" },
        nonRetroactive: { messageId: "room.historyNonRetroactive" }
      }
    });
    const { rerender } = render(
      <CreateEntityDialog
        kind="room"
        isBusy={false}
        value=""
        createAccessPreview={preview(true)}
        onCancel={vi.fn()}
        onSubmit={vi.fn()}
        onValueChange={vi.fn()}
      />
    );
    const radios = () =>
      within(screen.getByRole("group", { name: t("dialog.createWhoCanJoin") }))
        .getAllByRole("radio")
        .map((radio) => radio.getAttribute("value"));
    expect(radios()).toEqual(["knock", "invite", "public"]);
    rerender(
      <CreateEntityDialog
        kind="room"
        isBusy={false}
        value=""
        createAccessPreview={preview(false)}
        onCancel={vi.fn()}
        onSubmit={vi.fn()}
        onValueChange={vi.fn()}
      />
    );
    expect(radios()).toEqual(["invite", "public"]);
  });

  it("chooses earlier-message access with two plain choices and summarizes above the buttons", () => {
    const onSetRoomAccessDraft = vi.fn();
    render(
      <CreateEntityDialog
        kind="room"
        isBusy={false}
        value=""
        roomOptions={{
          aliasLocalpart: "",
          encrypted: true,
          invitedOnly: false,
          topic: "",
          visibility: "private"
        }}
        roomAccessDraft={{
          scope: { kind: "create", sessionId: 1 },
          revision: 1,
          rule: "invite",
          history: "shared"
        }}
        createAccessPreview={{
          scope: { kind: "create", sessionId: 1 },
          confirmed: false,
          effectiveRule: "invite",
          effectiveHistory: "shared",
          roomVersionPinned: false,
          knockAvailable: true,
          knockRestrictedAvailable: false,
          outcome: {
            join: { messageId: "room.accessOutcomeJoinInvite" },
            history: { messageId: "room.accessOutcomeHistoryShared" },
            encryption: { messageId: "room.accessOutcomeEncrypted" },
            directory: { messageId: "room.accessOutcomeDirectoryLoading" },
            nonRetroactive: { messageId: "room.historyNonRetroactive" }
          }
        }}
        createAccessSessionId={1}
        onCancel={vi.fn()}
        onRoomOptionsChange={vi.fn()}
        onSubmit={vi.fn()}
        onValueChange={vi.fn()}
        onSetRoomAccessDraft={onSetRoomAccessDraft}
      />
    );
    const history = screen.getByRole("group", { name: t("dialog.createEarlierMessages") });
    expect(within(history).getAllByRole("radio")).toHaveLength(2);
    // Reading without joining is offered only for a public room.
    expect(screen.queryByRole("checkbox", { name: t("dialog.createHistoryWorldReadable") })).toBeNull();
    fireEvent.click(screen.getByRole("radio", { name: t("dialog.createHistoryInvited") }));
    expect(onSetRoomAccessDraft).toHaveBeenCalledWith({
      kind: "history",
      scope: { kind: "create", sessionId: 1 },
      history: "invited"
    });
    const summary = document.querySelector(".create-dialog-footer .create-room-summary");
    expect(summary?.textContent).toBe(
      [
        t("room.accessOutcomeJoinInvite"),
        t("room.accessOutcomeHistoryShared"),
        t("room.accessOutcomeEncrypted")
      ].join(" · ")
    );
    expect(summary?.textContent).not.toContain(t("room.historyNonRetroactive"));
  });

  it.each([
    ["publicWithRestrictedAccess", "room.accessRejectionPublicWithRestrictedAccess"],
    ["explicitPolicyWithInvitedOnly", "room.accessRejectionExplicitPolicyWithInvitedOnly"],
    ["emptyAccessTargets", "room.accessRejectionEmptyAccessTargets"],
    ["publicWithInvitedOnly", "room.accessRejectionPublicWithInvitedOnly"]
  ] as const)("surfaces the Rust rejection %s and disables Create", (rejection, messageId) => {
    render(
      <CreateEntityDialog
        kind="room"
        isBusy={false}
        value="papers"
        roomOptions={{
          aliasLocalpart: "papers",
          encrypted: true,
          invitedOnly: false,
          topic: "",
          visibility: "private"
        }}
        addressPreview={{
          localpart: "papers",
          full_alias: "#papers:example.invalid",
          error: null,
          server_name: "example.invalid"
        }}
        createAccessPreview={{
          scope: { kind: "create", sessionId: 0 },
          confirmed: false,
          rejection,
          effectiveRule: "restricted",
          effectiveHistory: "shared",
          roomVersionPinned: true,
          knockAvailable: true,
          knockRestrictedAvailable: false,
          outcome: {
            join: { messageId: "room.accessOutcomeJoinSpaceMembers", substitutions: ["Design"] },
            history: { messageId: "room.accessOutcomeHistoryShared" },
            encryption: { messageId: "room.accessOutcomeEncrypted" },
            directory: { messageId: "room.accessOutcomeDirectoryLoading" },
            nonRetroactive: { messageId: "room.historyNonRetroactive" }
          }
        }}
        onCancel={vi.fn()}
        onRoomOptionsChange={vi.fn()}
        onSubmit={vi.fn()}
        onValueChange={vi.fn()}
      />
    );
    expect(screen.getByText(t(messageId))).toBeTruthy();
    expect(screen.getByText(t("room.accessRoomVersionPinned"))).toBeTruthy();
    expect((screen.getByRole("button", { name: t("dialog.submitCreateRoom") }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("disables the access controls while Create is pending", () => {
    render(
      <CreateEntityDialog
        kind="room"
        isBusy
        value="papers"
        roomOptions={{
          aliasLocalpart: "papers",
          encrypted: true,
          invitedOnly: false,
          topic: "",
          visibility: "private"
        }}
        addressPreview={{
          localpart: "papers",
          full_alias: "#papers:example.invalid",
          error: null,
          server_name: "example.invalid"
        }}
        roomAccessDraft={{
          scope: { kind: "create", sessionId: 1 },
          revision: 1,
          rule: "restricted",
          allowTargets: ["!space:example.invalid"]
        }}
        createAccessPreview={{
          scope: { kind: "create", sessionId: 1 },
          confirmed: false,
          effectiveRule: "restricted",
          effectiveHistory: "shared",
          roomVersionPinned: true,
          knockAvailable: true,
          knockRestrictedAvailable: false,
          outcome: {
            join: { messageId: "room.accessOutcomeJoinMembershipRoute" },
            history: { messageId: "room.accessOutcomeHistoryShared" },
            encryption: { messageId: "room.accessOutcomeEncrypted" },
            directory: { messageId: "room.accessOutcomeDirectoryLoading" },
            nonRetroactive: { messageId: "room.historyNonRetroactive" }
          }
        }}
        createAccessSessionId={1}
        targetSpaceName="Design"
        joinedSpaces={[
          { id: "!space:example.invalid", name: "Design" },
          { id: "!other:example.invalid", name: "Physics" }
        ]}
        onCancel={vi.fn()}
        onRoomOptionsChange={vi.fn()}
        onSubmit={vi.fn()}
        onValueChange={vi.fn()}
        onSetRoomAccessDraft={vi.fn()}
      />
    );

    expect(
      (screen.getByRole("radio", { name: t("dialog.createJoinPublic") }) as HTMLInputElement).matches(":disabled")
    ).toBe(true);
    expect(
      (screen.getByRole("checkbox", { name: "Design" }) as HTMLInputElement).matches(":disabled")
    ).toBe(true);
    expect(
      (screen.getByRole("checkbox", { name: t("dialog.encryptedRoom") }) as HTMLInputElement)
        .matches(":disabled")
    ).toBe(true);
  });
});

describe("DirectoryPreviewDialog text direction", () => {
  it("resolves name, alias and topic independently", () => {
    const preview: DirectoryPreviewState = {
      kind: "ready",
      request_id: 1,
      room_id_or_alias: "#public:example.invalid",
      via_servers: [],
      room: {
        room_id: "!public:example.invalid",
        canonical_alias: "#חדר:example.invalid",
        room_type: "m.space",
        name: "חדר ציבורי 7!",
        topic: "נושא החדר 99!",
        joined_members: 3,
        joinability: "open",
        membership: "none"
      }
    };

    render(
      <DirectoryPreviewDialog
        isBusy={false}
        preview={preview}
        onCancel={vi.fn()}
        onConfirm={vi.fn()}
      />
    );

    const name = screen.getByText("חדר ציבורי 7!", { exact: true });
    expect(name.getAttribute("dir")).toBe("auto");
    expect(name.closest("h2")?.getAttribute("dir")).not.toBe("auto");
    expect(screen.getByText("#חדר:example.invalid", { exact: true }).getAttribute("dir")).toBe(
      "auto"
    );
    expect(screen.getByText("נושא החדר 99!", { exact: true }).getAttribute("dir")).toBe("auto");
  });
});

describe("InviteTargetsDialog text direction", () => {
  it("marks each selected and candidate identity string independently", () => {
    const target = {
      user_id: "@rtl:example.invalid",
      display_label: "שלום 42!",
      original_display_label: "שלום 42!",
      avatar: null,
      source: "profile" as const,
      status: "selectable" as const,
      status_message: null
    };
    const current: InviteWorkflowState = {
      query: {
        room_id: "!room:example.invalid",
        query: "rtl",
        candidates: [target],
        explicit_user_id: target
      },
      selected_targets: [target],
      scope_plan: null,
      selected_scope: null,
      history_policy: null,
      operation: { kind: "idle" }
    };
    const { container } = render(
      <InviteTargetsDialog
        isBusy={false}
        query="rtl"
        title={t("dialog.invitePeopleTitle")}
        workflow={current}
        onCancel={vi.fn()}
        onQueryChange={vi.fn()}
        onRemoveTarget={vi.fn()}
        onScopeChange={vi.fn()}
        onSelectCandidate={vi.fn()}
        onSubmit={vi.fn()}
      />
    );

    expect(
      container.querySelector(".invite-selected-target span")?.getAttribute("dir")
    ).toBe("auto");
    for (const candidate of container.querySelectorAll(".invite-target-candidate")) {
      expect(candidate.querySelectorAll("span[dir=auto]")).toHaveLength(2);
    }
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

  function renderInvite(
    policy: Partial<NonNullable<InviteWorkflowState["history_policy"]>>,
    handlers: { onOpenRoomInfo?: () => void; onOpenRecovery?: () => void } = {}
  ) {
    const base = workflow();
    return render(
      <InviteTargetsDialog
        isBusy={false}
        query="alice"
        title={t("dialog.invitePeopleTitle")}
        workflow={{ ...base, history_policy: { ...base.history_policy!, ...policy } }}
        onCancel={vi.fn()}
        onOpenRecovery={handlers.onOpenRecovery}
        onOpenRoomInfo={handlers.onOpenRoomInfo}
        onQueryChange={vi.fn()}
        onRemoveTarget={vi.fn()}
        onScopeChange={vi.fn()}
        onSelectCandidate={vi.fn()}
        onSubmit={vi.fn()}
      />
    );
  }

  // #1418: the invite dialog reports what the invitee can read in one
  // sentence; it never presents the room setting as choices.
  it.each([
    ["shared", "dialog.inviteHistoryShared"],
    ["worldReadable", "dialog.inviteHistoryShared"],
    ["invited", "dialog.inviteHistoryInvited"],
    ["joined", "dialog.inviteHistoryJoined"]
  ] as const)("summarizes %s history in one sentence without options", (visibility, key) => {
    renderInvite({ current_visibility: visibility, encrypted: false });
    const region = screen.getByRole("region", { name: t("dialog.inviteHistory") });
    expect(within(region).getByText(t(key))).toBeTruthy();
    expect(within(region).queryAllByRole("listitem")).toHaveLength(0);
    expect(within(region).queryAllByRole("list")).toHaveLength(0);
    expect(within(region).queryAllByRole("radio")).toHaveLength(0);
    expect(within(region).queryByText("Current")).toBeNull();
  });

  it("adds the encrypted caveat only when earlier messages depend on key sharing", () => {
    renderInvite({ current_visibility: "shared", encrypted: true });
    expect(screen.getByText(t("dialog.inviteHistoryEncryptedShared"))).toBeTruthy();
    cleanup();
    renderInvite({ current_visibility: "joined", encrypted: true });
    expect(screen.queryByText(t("dialog.inviteHistoryEncryptedShared"))).toBeNull();
    cleanup();
    renderInvite({ current_visibility: "shared", encrypted: false });
    expect(screen.queryByText(t("dialog.inviteHistoryEncryptedShared"))).toBeNull();
  });

  it("links to Room Info only when the user can change history visibility", () => {
    const onOpenRoomInfo = vi.fn();
    renderInvite({ can_edit: true }, { onOpenRoomInfo });
    fireEvent.click(screen.getByRole("button", { name: t("dialog.inviteHistoryChange") }));
    expect(onOpenRoomInfo).toHaveBeenCalledTimes(1);
    cleanup();
    renderInvite({ can_edit: false }, { onOpenRoomInfo });
    expect(screen.queryByRole("button", { name: t("dialog.inviteHistoryChange") })).toBeNull();
  });

  it("keeps the recovery alert", () => {
    const onOpenRecovery = vi.fn();
    renderInvite({ readiness: "recoveryRequired" }, { onOpenRecovery });
    expect(screen.getByRole("alert").textContent).toContain(t("dialog.inviteHistoryRecovery"));
    fireEvent.click(screen.getByRole("button", { name: "Open recovery" }));
    expect(onOpenRecovery).toHaveBeenCalledTimes(1);
  });

  it("puts the actions in a footer outside the scrolling body", () => {
    const { container } = renderInvite({});
    const body = container.ownerDocument.querySelector(".invite-dialog-body");
    const footer = container.ownerDocument.querySelector(".invite-dialog-footer");
    expect(body).not.toBeNull();
    expect(footer).not.toBeNull();
    expect(body!.contains(footer)).toBe(false);
    expect(footer!.querySelector(".dialog-actions")).not.toBeNull();
  });

  it("offers Done after a completed invite instead of leaving only Cancel", () => {
    const onCancel = vi.fn();
    render(
      <InviteTargetsDialog
        isBusy={false}
        query=""
        title={t("dialog.invitePeopleTitle")}
        workflow={{
          ...workflow(),
          operation: {
            kind: "completed",
            request_id: 7,
            room_id: "!room:example.invalid",
            results: [
              {
                user_id: "@alice:example.invalid",
                destination: { kind: "room", room_id: "!room:example.invalid" },
                kind: "invited",
                message: null
              }
            ],
            notice: null
          }
        }}
        onCancel={onCancel}
        onQueryChange={vi.fn()}
        onRemoveTarget={vi.fn()}
        onScopeChange={vi.fn()}
        onSelectCandidate={vi.fn()}
        onSubmit={vi.fn()}
      />
    );

    expect(screen.getByRole("button", { name: "Done" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
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
      screen.getByRole<HTMLButtonElement>("button", { name: "Send attachments" }).matches(":disabled")
    ).toBe(true);
  });
});

describe("LeaveSpaceDialog", () => {
  const candidates: SpaceLeaveCandidate[] = [
    { room_id: "!alpha:example.invalid", display_name: "Alpha", avatar: null, in_other_space: false },
    { room_id: "!beta:example.invalid", display_name: "Beta", avatar: null, in_other_space: true },
    { room_id: "!gamma:example.invalid", display_name: "Gamma", avatar: null, in_other_space: false }
  ];

  function renderDialog(onConfirm = vi.fn(), roomCandidates = candidates, isBusy = false) {
    render(
      <LeaveSpaceDialog
        spaceName="Synthetic Workspace"
        candidates={roomCandidates}
        isBusy={isBusy}
        onCancel={vi.fn()}
        onConfirm={onConfirm}
      />
    );
    return onConfirm;
  }

  it("leaves only the Space by default", () => {
    const onConfirm = renderDialog();
    expect(screen.getByRole("radio", { name: t("space.leaveSpaceOnly") })).toHaveProperty("checked", true);
    expect(screen.queryByRole("list", { name: t("space.leaveRoomsList") })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: t("space.leaveConfirmAction") }));
    expect(onConfirm).toHaveBeenCalledWith([]);
  });

  it("leaves every candidate room when all rooms are chosen", () => {
    const onConfirm = renderDialog();
    fireEvent.click(screen.getByRole("radio", { name: t("space.leaveAllRooms", { count: 3 }) }));
    const checkboxes = within(screen.getByRole("list", { name: t("space.leaveRoomsList") }))
      .getAllByRole("checkbox");
    expect(checkboxes.map((box) => (box as HTMLInputElement).checked)).toEqual([true, true, true]);
    fireEvent.click(
      screen.getByRole("button", { name: t("space.leaveConfirmActionWithRooms", { count: 3 }) })
    );
    expect(onConfirm).toHaveBeenCalledWith([
      "!alpha:example.invalid",
      "!beta:example.invalid",
      "!gamma:example.invalid"
    ]);
  });

  it("leaves only the selected rooms, in candidate order, and marks shared rooms", () => {
    const onConfirm = renderDialog();
    fireEvent.click(screen.getByRole("radio", { name: t("space.leaveSelectedRooms") }));
    expect(screen.getByText(t("space.leaveRoomInOtherSpace"))).toBeTruthy();
    fireEvent.click(screen.getByRole("checkbox", { name: /Gamma/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /Alpha/ }));
    fireEvent.click(
      screen.getByRole("button", { name: t("space.leaveConfirmActionWithRooms", { count: 2 }) })
    );
    expect(onConfirm).toHaveBeenCalledWith(["!alpha:example.invalid", "!gamma:example.invalid"]);
  });

  it("offers no room choice when the Space has no joined rooms", () => {
    const onConfirm = renderDialog(vi.fn(), []);
    expect(screen.queryByRole("radiogroup")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: t("space.leaveConfirmAction") }));
    expect(onConfirm).toHaveBeenCalledWith([]);
  });

  it("disables every control while the leave is in flight", () => {
    renderDialog(vi.fn(), candidates, true);
    expect(screen.getByRole("button", { name: t("space.leaveConfirmAction") })).toHaveProperty("disabled", true);
    expect(screen.getByRole("radio", { name: t("space.leaveSpaceOnly") })).toHaveProperty("disabled", true);
  });
});

describe("upload staging failure affordances (#1147)", () => {
  function renderFailedItem(overrides: { canUseOriginal?: boolean; failureKind?: "empty" | "unsupported" | "decode" | "encode" | "missingPreparedBytes" } = {}) {
    const onRetryPreparation = vi.fn();
    const onUseOriginal = vi.fn();
    const onSendAttachments = vi.fn();
    render(
      <UploadStagingDialog
        items={[
          stagedImage(
            "holiday",
            {
              kind: "failed",
              failure_kind: overrides.failureKind ?? "decode",
              can_use_original: overrides.canUseOriginal ?? true
            },
            "staged-failed",
            "camera.HEIC"
          )
        ]}
        onClear={vi.fn()}
        onUpdateCaption={vi.fn()}
        onSelectOutput={vi.fn()}
        onRetryPreparation={onRetryPreparation}
        onUseOriginal={onUseOriginal}
        onSendAttachments={onSendAttachments}
        loadPreview={vi.fn(async () => [])}
        resolveComposerKeyAction={vi.fn(async () => "noop" as const)}
        surface="main"
      />
    );
    return { onRetryPreparation, onUseOriginal, onSendAttachments };
  }

  it("offers retry and original instead of the output toolbar", () => {
    const { onRetryPreparation, onUseOriginal } = renderFailedItem();

    expect(screen.getByText("Attachment preparation failed")).toBeTruthy();
    // No resize/format controls and no preview for a failed preparation.
    expect(screen.queryByRole("radiogroup", { name: "Resize" })).toBeNull();
    expect(screen.queryByRole("radiogroup", { name: "Format" })).toBeNull();
    expect(document.querySelector("img.upload-staging-preview")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Retry preparation" }));
    fireEvent.click(screen.getByRole("button", { name: "Use original" }));
    expect(onRetryPreparation).toHaveBeenCalledWith("staged-failed");
    expect(onUseOriginal).toHaveBeenCalledWith("staged-failed");
  });

  it.each([
    ["empty", "The attachment is empty"],
    ["unsupported", "The attachment format is not supported"],
    ["decode", "The image could not be decoded"],
    ["encode", "The image could not be encoded"],
    ["missingPreparedBytes", "The prepared attachment data is missing"]
  ] as const)("shows the %s preparation failure", (failureKind, message) => {
    renderFailedItem({ failureKind });
    expect(screen.getByText(message)).toBeTruthy();
  });

  it("hides the original fallback when there are no retained source bytes", () => {
    renderFailedItem({ canUseOriginal: false });
    expect(screen.getByRole("button", { name: "Retry preparation" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Use original" })).toBeNull();
  });
});

// Issue #1410: the one-time suggested-rooms offer. The rooms come from Rust;
// the dialog only asks, and nothing is joined without Join all.
describe("SuggestedRoomsDialog", () => {
  const rooms = [
    { room_id: "!alpha:example.invalid", display_name: "Alpha" },
    { room_id: "!beta:example.invalid", display_name: "Beta" }
  ];

  it("names the Space, lists the rooms, and joins only on Join all", () => {
    const onJoinAll = vi.fn();
    const onDismiss = vi.fn();
    render(
      <SuggestedRoomsDialog
        offer={{ kind: "offered", space_id: "!space:example.invalid", rooms }}
        spaceName="Synthetic Workspace"
        onJoinAll={onJoinAll}
        onDismiss={onDismiss}
      />
    );
    expect(
      screen.getByRole("dialog", {
        name: t("suggestedRooms.offerTitle", { space: "Synthetic Workspace" })
      })
    ).toBeTruthy();
    const list = screen.getByRole("list", { name: t("suggestedRooms.listLabel") });
    expect(within(list).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "Alpha",
      "Beta"
    ]);
    expect(onJoinAll).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: t("suggestedRooms.skip") }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onJoinAll).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: t("suggestedRooms.joinAll") }));
    expect(onJoinAll).toHaveBeenCalledTimes(1);
  });

  it("disables both choices while joining", () => {
    render(
      <SuggestedRoomsDialog
        offer={{ kind: "joining", request_id: 3, space_id: "!space:example.invalid", rooms }}
        spaceName="Synthetic Workspace"
        onJoinAll={vi.fn()}
        onDismiss={vi.fn()}
      />
    );
    expect(
      (screen.getByRole("button", { name: t("suggestedRooms.joining") }) as HTMLButtonElement)
        .matches(":disabled")
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: t("suggestedRooms.skip") }) as HTMLButtonElement).matches(":disabled")
    ).toBe(true);
  });

  it("reports each room's outcome after a partial failure", () => {
    const onDismiss = vi.fn();
    render(
      <SuggestedRoomsDialog
        offer={{
          kind: "completed",
          space_id: "!space:example.invalid",
          results: [
            { room_id: "!alpha:example.invalid", display_name: "Alpha", failure: null },
            { room_id: "!beta:example.invalid", display_name: "Beta", failure: "forbidden" }
          ]
        }}
        spaceName="Synthetic Workspace"
        onJoinAll={vi.fn()}
        onDismiss={onDismiss}
      />
    );
    const items = within(
      screen.getByRole("list", { name: t("suggestedRooms.listLabel") })
    ).getAllByRole("listitem");
    expect(items[0].textContent).toContain(t("suggestedRooms.joined"));
    expect(items[1].textContent).toContain("Beta");
    expect(items[1].textContent).not.toContain(t("suggestedRooms.joined"));
    fireEvent.click(screen.getByRole("button", { name: t("suggestedRooms.done") }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
