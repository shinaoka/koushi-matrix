// @vitest-environment jsdom

import { useState } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { MentionCandidate } from "../domain/projectionTypes";
import { documentFromText, plainBodyFromDocument } from "../domain/composerDocument";
import type {
  ComposerDocument,
  ComposerResolvedAction,
  ComposerResolverOptions,
  ResolveComposerKeyAction
} from "../domain/types";
import { Composer, ThreadComposer } from "./composer";
import {
  inlineMentionEditorSelection,
  setInlineMentionEditorSelection
} from "./ImeTextControl";

function textChange(callback: (value: string) => void) {
  return (document: ComposerDocument) => callback(plainBodyFromDocument(document));
}

function textSend(callback: (value: string) => void | Promise<void>) {
  return (document: ComposerDocument) => callback(plainBodyFromDocument(document));
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

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Composer", () => {
  const mentionCandidates: MentionCandidate[] = [
    {
      key: "@alice:example.invalid",
      label: "Alice",
      target: {
        kind: "user",
        user_id: "@alice:example.invalid",
        display_label: "Alice"
      }
    },
    {
      key: "@bob:example.invalid",
      label: "Bob",
      target: {
        kind: "user",
        user_id: "@bob:example.invalid",
        display_label: "Bob"
      }
    },
    {
      key: "roomMention",
      label: "@room",
      target: {
        kind: "roomMention",
        display_label: "room"
      }
    }
  ];

  it("stages ordinary files dropped on every composer region in deterministic order", async () => {
    const onAttachFiles = vi.fn(async (_files: File[]) => undefined);
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("")}
        onAttachFiles={onAttachFiles}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );
    const pdf = new File(["pdf"], "document.pdf", { type: "application/pdf" });
    const archive = new File(["zip"], "archive.zip", { type: "application/zip" });
    const dataTransfer = {
      files: [pdf, archive],
      items: [
        { kind: "file", type: pdf.type },
        { kind: "file", type: archive.type }
      ],
      types: ["Files"]
    };
    const targets = [
      container.querySelector(".composer-inline-editor"),
      container.querySelector(".composer-tools"),
      container.querySelector(".composer-footer"),
      container.querySelector(".composer")
    ];

    for (const target of targets) {
      expect(target).not.toBeNull();
      fireEvent.drop(target!, { dataTransfer });
    }

    await waitFor(() => expect(onAttachFiles).toHaveBeenCalledTimes(4));
    for (const [files] of onAttachFiles.mock.calls) {
      expect(files.map((file) => file.name)).toEqual(["document.pdf", "archive.zip"]);
    }
  });

  it("ignores non-file drops on the composer surface", () => {
    const onAttachFiles = vi.fn();
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("")}
        onAttachFiles={onAttachFiles}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );

    fireEvent.drop(container.querySelector(".composer")!, {
      dataTransfer: {
        files: [],
        items: [{ kind: "string", type: "text/plain" }],
        types: ["text/plain"]
      }
    });

    expect(onAttachFiles).not.toHaveBeenCalled();
  });

  it("keeps the live conversion DOM value and selection across parent rerenders", () => {
    const props = {
      composerMode: { kind: "plain" as const },
      isSending: false,
      roomName: "Direct room",
      document: documentFromText("before"),
      onCancelReply: () => undefined,
      onSend: textSend(vi.fn()),
      onDocumentChange: textChange(vi.fn())
    };
    const { container, rerender } = render(<Composer {...props} />);
    const textarea = container.querySelector(".composer-inline-editor")!;

    fireEvent.compositionStart(textarea);
    changeEditorText(textarea, "日本語変換中" );
    setInlineMentionEditorSelection(textarea as HTMLDivElement, 3, 5);
    rerender(<Composer {...props} document={documentFromText("stale parent draft")} roomName="Renamed room" />);

    expect(textarea.textContent).toBe("日本語変換中");
    expect([inlineMentionEditorSelection(textarea as HTMLDivElement).start, inlineMentionEditorSelection(textarea as HTMLDivElement).end]).toEqual([3, 5]);
  });

  it("writes toolbar markdown replacements to the IME-owned textarea DOM", () => {
    const onValueChange = vi.fn();
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("")}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(onValueChange)}
      />
    );
    const textarea = container.querySelector(".composer-inline-editor")!;

    changeEditorText(textarea, "world" );
    setInlineMentionEditorSelection(textarea as HTMLDivElement, 0, 5);
    fireEvent.click(screen.getByRole("button", { name: /bold/i }));

    expect(textarea.textContent).toBe("**world**");
    expect(onValueChange).toHaveBeenLastCalledWith("**world**");
  });

  it("renders a localized notice for rejected slash commands and hides it without one", () => {
    // Issue #450: recognized-but-unavailable commands (/join, /invite) show a
    // clear localized explanation near the composer instead of appearing inert.
    const { rerender } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("")}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
        notice="This command is not available in this composer."
      />
    );
    const status = screen.getByRole("status");
    expect(status.textContent).toBe("This command is not available in this composer.");
    rerender(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("")}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
        notice={null}
      />
    );
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("renders the math mode toggle from props and requests Rust-owned settings updates", () => {
    const onMathModeChange = vi.fn();
    const { rerender } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mathModeEnabled
        roomName="Direct room"
        document={documentFromText("")}
        onCancelReply={() => undefined}
        onMathModeChange={onMathModeChange}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );

    // #453: the control is a switch, not an insert-markup button, so it must
    // expose switch semantics and a state a sighted user can read at rest.
    const toggle = screen.getByRole("switch", { name: /math formatting on/i });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.querySelector(".composer-math-switch-track")).not.toBeNull();
    fireEvent.click(toggle);
    expect(onMathModeChange).toHaveBeenCalledWith(false);

    rerender(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mathModeEnabled={false}
        roomName="Direct room"
        document={documentFromText("")}
        onCancelReply={() => undefined}
        onMathModeChange={onMathModeChange}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );
    expect(
      screen.getByRole("switch", { name: /math formatting off/i }).getAttribute("aria-checked")
    ).toBe("false");
  });

  it("gives the thread textarea the same live conversion ownership", () => {
    const props = {
      canEdit: true,
      document: documentFromText("before"),
      draftKey: "!room-a:$root-a",
      isSending: false,
      resolveComposerKeyAction: vi.fn(async () => "noop" as const),
      onDocumentChange: textChange(vi.fn()),
      onSend: textSend(vi.fn())
    };
    const { rerender } = render(<ThreadComposer {...props} />);
    const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLDivElement;

    fireEvent.compositionStart(textarea);
    changeEditorText(textarea, "日本語変換中" );
    setInlineMentionEditorSelection(textarea as HTMLDivElement, 3, 5);
    rerender(<ThreadComposer {...props} document={documentFromText("stale parent draft")} isSending />);

    expect(textarea.textContent).toBe("日本語変換中");
    expect([inlineMentionEditorSelection(textarea as HTMLDivElement).start, inlineMentionEditorSelection(textarea as HTMLDivElement).end]).toEqual([3, 5]);
  });

  it("reuses the full composer surface for thread formatting, attachments, and key resolution", async () => {
    const onAttachFiles = vi.fn(async (_files: File[]) => undefined);
    const resolveComposerKeyAction = vi.fn(async () => "noop" as const);
    const { container } = render(
      <ThreadComposer
        canEdit
        document={documentFromText("thread body")}
        draftKey="!room-a:$root-a"
        isSending={false}
        resolveComposerKeyAction={resolveComposerKeyAction}
        onAttachFiles={onAttachFiles}
        onDocumentChange={textChange(vi.fn())}
        onSend={textSend(vi.fn())}
      />
    );

    expect(screen.getByRole("button", { name: /bold/i })).not.toBeNull();
    expect(screen.getByRole("button", { name: /italic/i })).not.toBeNull();
    expect(screen.getByRole("button", { name: /link/i })).not.toBeNull();
    expect(screen.getByRole("button", { name: /list/i })).not.toBeNull();
    expect(screen.getByRole("button", { name: /code/i })).not.toBeNull();

    const file = new File(["pdf"], "thread.pdf", { type: "application/pdf" });
    fireEvent.drop(container.querySelector(".composer")!, {
      dataTransfer: {
        files: [file],
        items: [{ kind: "file", type: file.type }],
        types: ["Files"]
      }
    });
    await waitFor(() => expect(onAttachFiles).toHaveBeenCalledWith([file]));

    fireEvent.keyDown(container.querySelector(".composer-inline-editor")!, {
      key: "Enter",
      code: "Enter",
      keyCode: 13
    });
    await waitFor(() =>
      expect(resolveComposerKeyAction).toHaveBeenCalledWith(
        "thread",
        expect.anything(),
        expect.anything()
      )
    );
  });

  it("releases the old thread DOM when the room/root draft key switches", () => {
    vi.useFakeTimers();
    const props = {
      canEdit: true,
      document: documentFromText("thread A draft"),
      draftKey: "!room-a:$root-a",
      isSending: false,
      resolveComposerKeyAction: vi.fn(async () => "noop" as const),
      onDocumentChange: textChange(vi.fn()),
      onSend: textSend(vi.fn())
    };
    const { rerender } = render(<ThreadComposer {...props} />);
    const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLDivElement;
    fireEvent.compositionStart(textarea);
    changeEditorText(textarea, "private thread A conversion");
    fireEvent.compositionEnd(textarea);
    rerender(
      <ThreadComposer
        {...props}
        draftKey="!room-b:$root-b"
        document={documentFromText("thread B draft")}
      />
    );
    vi.runAllTimers();

    expect(textarea.textContent).toBe("thread B draft");
  });

  it("sends the visible thread DOM draft after a stale parent rerender", () => {
    const onSend = vi.fn();
    const props = {
      canEdit: true,
      document: documentFromText(""),
      draftKey: "!room-a:$root-a",
      isSending: false,
      resolveComposerKeyAction: vi.fn(async () => "send" as const),
      onDocumentChange: textChange(vi.fn()),
      onSend: textSend(onSend)
    };
    const { rerender } = render(<ThreadComposer {...props} />);
    const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLDivElement;
    fireEvent.compositionStart(textarea);
    changeEditorText(textarea, "visible reply" );
    rerender(<ThreadComposer {...props} document={documentFromText("")} />);
    fireEvent.compositionEnd(textarea);
    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("visible reply");
  });

  it("records privacy-safe button submission diagnostics through callback settlement", async () => {
    const onDiagnosticLogEntry = vi.fn();
    const onSend = vi.fn(async () => undefined);
    render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Room"
        document={documentFromText("private message content")}
        onCancelReply={() => undefined}
        onDocumentChange={textChange(vi.fn())}
        onSend={textSend(onSend)}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(onDiagnosticLogEntry).toHaveBeenCalledTimes(3));
    expect(onDiagnosticLogEntry.mock.calls.map(([entry]) => entry.message)).toEqual([
      expect.stringContaining(
        "stage=attempt surface=main trigger=button can_edit=true is_sending=false ime_composing=false body_length=21-100"
      ),
      "stage=handoff surface=main trigger=button target=text",
      "stage=callback_settled surface=main trigger=button outcome=resolved"
    ]);
    expect(JSON.stringify(onDiagnosticLogEntry.mock.calls)).not.toContain("private message content");
  });

  it("passes the visible thread DOM draft through keyboard send", async () => {
    vi.useFakeTimers();
    const onSend = vi.fn();
    const props = {
      canEdit: true,
      document: documentFromText(""),
      draftKey: "!room-a:$root-a",
      isSending: false,
      resolveComposerKeyAction: vi.fn(async () => "send" as const),
      onDocumentChange: textChange(vi.fn()),
      onSend: textSend(onSend)
    };
    const { rerender } = render(<ThreadComposer {...props} />);
    const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLDivElement;
    fireEvent.compositionStart(textarea);
    changeEditorText(textarea, "visible keyboard reply" );
    rerender(<ThreadComposer {...props} document={documentFromText("")} />);
    fireEvent.compositionEnd(textarea);
    vi.runAllTimers();
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    await Promise.resolve();

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("visible keyboard reply");
  });

  it("discards a stale deferred thread newline after newer DOM input", async () => {
    let resolveAction!: (action: "insertNewline") => void;
    const action = new Promise<"insertNewline">((resolve) => {
      resolveAction = resolve;
    });
    const onDraftChange = vi.fn();
    render(
      <ThreadComposer
        canEdit
        document={documentFromText("captured")}
        draftKey="!room-a:$root-a"
        isSending={false}
        resolveComposerKeyAction={() => action}
        onDocumentChange={textChange(onDraftChange)}
        onSend={textSend(vi.fn())}
      />
    );
    const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLDivElement;
    setInlineMentionEditorSelection(textarea as HTMLDivElement, 8, 8);
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    changeEditorText(textarea, "newer input" );
    await act(async () => resolveAction("insertNewline"));

    expect(textarea.textContent).toBe("newer input");
    expect(onDraftChange).toHaveBeenCalledTimes(1);
    expect(onDraftChange).toHaveBeenLastCalledWith("newer input");
  });

  it("discards a stale deferred main newline after newer DOM input", async () => {
    let resolveAction!: (action: "insertNewline") => void;
    const action = new Promise<"insertNewline">((resolve) => {
      resolveAction = resolve;
    });
    const onValueChange = vi.fn();
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Room"
        document={documentFromText("captured")}
        resolveComposerKeyAction={() => action}
        onCancelReply={() => undefined}
        onSend={textSend(vi.fn())}
        onDocumentChange={textChange(onValueChange)}
      />
    );
    const textarea = container.querySelector(".composer-inline-editor")!;
    setInlineMentionEditorSelection(textarea as HTMLDivElement, 8, 8);
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    changeEditorText(textarea, "newer input" );
    await act(async () => resolveAction("insertNewline"));

    expect(textarea.textContent).toBe("newer input");
    expect(onValueChange).toHaveBeenCalledTimes(1);
    expect(onValueChange).toHaveBeenLastCalledWith("newer input");
  });

  it("sends the main value captured when deferred Enter was pressed", async () => {
    let resolveAction!: (action: "send") => void;
    const action = new Promise<"send">((resolve) => {
      resolveAction = resolve;
    });
    const onSend = vi.fn();
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Room"
        document={documentFromText("intent snapshot")}
        resolveComposerKeyAction={() => action}
        onCancelReply={() => undefined}
        onSend={textSend(onSend)}
        onDocumentChange={textChange(vi.fn())}
      />
    );
    const textarea = container.querySelector(".composer-inline-editor")!;
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    changeEditorText(textarea, "later input" );
    await act(async () => resolveAction("send"));

    expect(onSend).toHaveBeenCalledWith("intent snapshot");
  });

  it.each([
    { initial: "", visible: "visible", sendEnabled: true },
    { initial: "stale local", visible: "", sendEnabled: false }
  ])("derives main send_enabled from the intent snapshot", async ({ initial, visible, sendEnabled }) => {
    const resolveComposerKeyAction = vi.fn(async () => "noop" as const);
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Room"
        document={documentFromText(initial)}
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={textSend(vi.fn())}
        onDocumentChange={textChange(vi.fn())}
      />
    );
    const textarea = container.querySelector(".composer-inline-editor")!;
    changeEditorText(textarea, visible);
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    await Promise.resolve();

    expect(resolveComposerKeyAction).toHaveBeenCalledWith(
      "main",
      expect.anything(),
      expect.objectContaining({ send_enabled: sendEnabled })
    );
  });

  it.each(["insertNewline", "send"] as const)(
    "discards deferred %s after the thread composer unmounts",
    async (resolvedAction) => {
      let resolveAction!: (action: typeof resolvedAction) => void;
      const action = new Promise<typeof resolvedAction>((resolve) => {
        resolveAction = resolve;
      });
      const onDraftChange = vi.fn();
      const onSend = vi.fn();
      const { unmount } = render(
        <ThreadComposer
          canEdit
          document={documentFromText("captured")}
          draftKey="!room-a:$root-a"
          isSending={false}
          resolveComposerKeyAction={() => action}
          onDocumentChange={textChange(onDraftChange)}
          onSend={textSend(onSend)}
        />
      );
      const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLDivElement;
      fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
      unmount();
      await act(async () => resolveAction(resolvedAction));

      expect(onDraftChange).not.toHaveBeenCalled();
      expect(onSend).not.toHaveBeenCalled();
    }
  );

  it("discards deferred reply cancel after the main composer unmounts", async () => {
    let resolveAction!: (action: "cancel") => void;
    const action = new Promise<"cancel">((resolve) => {
      resolveAction = resolve;
    });
    const onCancelReply = vi.fn();
    const { container, unmount } = render(
      <Composer
        composerMode={{ kind: "reply", in_reply_to_event_id: "$reply" }}
        isSending={false}
        roomName="Room"
        document={documentFromText("draft")}
        resolveComposerKeyAction={() => action}
        onCancelReply={onCancelReply}
        onSend={textSend(vi.fn())}
        onDocumentChange={textChange(vi.fn())}
      />
    );
    fireEvent.keyDown(container.querySelector(".composer-inline-editor")!, {
      key: "Escape",
      code: "Escape",
      keyCode: 27
    });
    unmount();
    await act(async () => resolveAction("cancel"));

    expect(onCancelReply).not.toHaveBeenCalled();
  });

  it("discards deferred autocomplete acceptance after newer input", async () => {
    let resolveAction!: (action: "acceptAutocomplete") => void;
    const action = new Promise<"acceptAutocomplete">((resolve) => {
      resolveAction = resolve;
    });
    const onValueChange = vi.fn();
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={[mentionCandidates[2]!]}
        roomName="Room"
        document={documentFromText("@a")}
        resolveComposerKeyAction={() => action}
        onCancelReply={() => undefined}
        onSend={textSend(vi.fn())}
        onDocumentChange={textChange(onValueChange)}
      />
    );
    const textarea = container.querySelector(".composer-inline-editor")!;
    setInlineMentionEditorSelection(textarea as HTMLDivElement, 2, 2);
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    changeEditorText(textarea, "newer input" );
    await act(async () => resolveAction("acceptAutocomplete"));

    expect(textarea.textContent).toBe("newer input");
    expect(onValueChange).toHaveBeenCalledTimes(1);
  });

  it("sends the thread value captured when deferred Enter was pressed", async () => {
    let resolveAction!: (action: "send") => void;
    const action = new Promise<"send">((resolve) => {
      resolveAction = resolve;
    });
    const onSend = vi.fn();
    render(
      <ThreadComposer
        canEdit
        document={documentFromText("intent snapshot")}
        draftKey="!room-a:$root-a"
        isSending={false}
        resolveComposerKeyAction={() => action}
        onDocumentChange={textChange(vi.fn())}
        onSend={textSend(onSend)}
      />
    );
    const textarea = screen.getByRole("textbox", { name: /thread/i }) as HTMLDivElement;
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    changeEditorText(textarea, "later input" );
    await act(async () => resolveAction("send"));

    expect(onSend).toHaveBeenCalledWith("intent snapshot");
  });

  it("does not submit while an IME composition is being confirmed with Enter", async () => {
    const onSend = vi.fn();
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);

    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("日本語")}
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={textSend(onSend)}
        onDocumentChange={textChange(() => undefined)}
      />
    );

    const textarea = container.querySelector(".composer-inline-editor");
    expect(textarea).not.toBeNull();

    fireEvent.compositionStart(textarea!);
    fireEvent.keyDown(textarea!, {
      key: "Enter",
      code: "Enter",
      keyCode: 13
    });

    await Promise.resolve();

    expect(onSend).not.toHaveBeenCalled();
    expect(resolveComposerKeyAction).not.toHaveBeenCalled();
  });

  it("does not let composition A's deferred end clear composition B", async () => {
    vi.useFakeTimers();
    const onSend = vi.fn();
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("日本語")}
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={textSend(onSend)}
        onDocumentChange={textChange(() => undefined)}
      />
    );
    const textarea = container.querySelector(".composer-inline-editor")!;

    fireEvent.compositionStart(textarea);
    fireEvent.compositionEnd(textarea);
    fireEvent.compositionStart(textarea);
    vi.runAllTimers();
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    await Promise.resolve();

    expect(resolveComposerKeyAction).not.toHaveBeenCalled();
    expect(onSend).not.toHaveBeenCalled();
  });

  it("resolves one main key intent for rapid Enter presses", async () => {
    let resolveAction!: (action: "send") => void;
    const action = new Promise<"send">((resolve) => {
      resolveAction = resolve;
    });
    const resolveComposerKeyAction = vi.fn(() => action);
    const onSend = vi.fn();
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Room"
        document={documentFromText("once")}
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={textSend(onSend)}
        onDocumentChange={textChange(vi.fn())}
      />
    );
    const textarea = container.querySelector(".composer-inline-editor")!;
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    expect(resolveComposerKeyAction).toHaveBeenCalledTimes(1);
    await act(async () => resolveAction("send"));
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it("finishes old DOM ownership and syncs a switched draft exactly once", async () => {
    vi.useFakeTimers();
    const onSend = vi.fn();
    const onValueChange = vi.fn();
    const resolveComposerKeyAction = vi.fn(async () => "send" as const);
    const props = {
      composerMode: { kind: "plain" as const },
      isSending: false,
      roomName: "Room A",
      document: documentFromText("old draft"),
      draftKey: "room-a",
      resolveComposerKeyAction,
      onCancelReply: () => undefined,
      onSend: textSend(onSend),
      onDocumentChange: textChange(onValueChange)
    };
    const { container, rerender } = render(<Composer {...props} />);
    const textarea = container.querySelector(".composer-inline-editor")!;
    fireEvent.compositionStart(textarea);
    changeEditorText(textarea, "旧変換中" );
    fireEvent.compositionEnd(textarea);

    rerender(
      <Composer
        {...props}
        draftKey="room-b"
        roomName="Room B"
        document={documentFromText("new room draft")}
      />
    );

    expect(textarea.textContent).toBe("new room draft");
    expect([inlineMentionEditorSelection(textarea as HTMLDivElement).start, inlineMentionEditorSelection(textarea as HTMLDivElement).end]).toEqual([14, 14]);
    expect(onValueChange).toHaveBeenCalledTimes(1);

    vi.runAllTimers();
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    await Promise.resolve();
    expect(resolveComposerKeyAction).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onValueChange).toHaveBeenCalledTimes(1);
  });

  it("keeps typed text local and sends it before parent state catches up", () => {
    const onSend = vi.fn();
    const onValueChange = vi.fn();

    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        document={documentFromText("")}
        onCancelReply={() => undefined}
        onSend={textSend(onSend)}
        onDocumentChange={textChange(onValueChange)}
      />
    );

    const textarea = container.querySelector(".composer-inline-editor");
    expect(textarea).not.toBeNull();
    changeEditorText(textarea!, "pasted text that should appear immediately" );

    expect(textarea!.textContent).toBe("pasted text that should appear immediately");
    expect(onValueChange).toHaveBeenCalledWith("pasted text that should appear immediately");

    fireEvent.click(screen.getByLabelText("Send"));

    expect(onSend).toHaveBeenCalledWith("pasted text that should appear immediately");
  });

  it("accepts a suggestion as the only inline mention entity without a duplicate pill", () => {
    const onDocumentChange = vi.fn();
    function Harness() {
      const [document, setDocument] = useState<ComposerDocument>(documentFromText("@a"));
      return (
        <Composer
          composerMode={{ kind: "plain" }}
          document={document}
          isSending={false}
          mentionCandidates={[mentionCandidates[0]!]}
          roomName="Direct room"
          onCancelReply={() => undefined}
          onDocumentChange={(next) => {
            setDocument(next);
            onDocumentChange(next);
          }}
          onSend={textSend(() => undefined)}
        />
      );
    }
    const { container } = render(<Harness />);

    fireEvent.click(screen.getByRole("option", { name: /Alice/ }));

    expect(onDocumentChange.mock.lastCall?.[0].inlines).toMatchObject([
      {
        kind: "mention",
        target: { kind: "user", user_id: "@alice:example.invalid" },
        display_label: "Alice"
      },
      { kind: "text", text: " " }
    ]);
    expect(container.querySelector(".composer-inline-mention")?.textContent).toBe("@Alice");
    expect(container.querySelector(".composer-mention-pills")).toBeNull();
  });

  it("moves the active mention row with arrows and accepts it with Tab", () => {
    const onDocumentChange = vi.fn();

    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={mentionCandidates}
        roomName="Direct room"
        document={documentFromText("@")}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={onDocumentChange}
      />
    );

    const editor = container.querySelector(".composer-inline-editor");
    expect(editor).not.toBeNull();

    fireEvent.keyDown(editor!, { key: "ArrowDown", code: "ArrowDown" });
    expect(
      screen.getByRole("option", { name: "Bob @bob:example.invalid" }).getAttribute("aria-selected")
    ).toBe("true");

    fireEvent.keyDown(editor!, { key: "Tab", code: "Tab" });

    expect(onDocumentChange.mock.lastCall?.[0].inlines).toMatchObject([
      {
        kind: "mention",
        target: {
          kind: "user",
          user_id: "@bob:example.invalid",
          display_label: "Bob"
        }
      },
      { kind: "text", text: " " }
    ]);
  });

  it("requests Rust-owned mention results and renders their canonical order without local filtering", () => {
    const onMentionQueryChange = vi.fn();
    const orderedCandidates = [
      mentionCandidates[1]!,
      mentionCandidates[0]!
    ];
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={orderedCandidates}
        roomName="Direct room"
        document={documentFromText("@alice")}
        onCancelReply={() => undefined}
        onMentionQueryChange={onMentionQueryChange}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );

    expect(onMentionQueryChange).toHaveBeenCalledWith("alice");
    expect(
      screen.getAllByRole("option").map((option) => option.getAttribute("data-mention-key"))
    ).toEqual(orderedCandidates.map((candidate) => candidate.key));
    expect(container.querySelector(".composer-autocomplete-loading")).toBeNull();
  });

  it("keeps known partial mention candidates visible with a loading state", () => {
    render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={[mentionCandidates[0]!]}
        mentionCandidatesLoading
        roomName="Direct room"
        document={documentFromText("@")}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );

    expect(screen.getByRole("option")).toBeTruthy();
    expect(screen.getByText("Loading people…")).toBeTruthy();
  });

  it("closes mention suggestions on Escape until the query changes", async () => {
    const resolveComposerKeyAction = vi.fn(async () => "closeAutocomplete" as const);
    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={mentionCandidates}
        roomName="Direct room"
        document={documentFromText("@a")}
        resolveComposerKeyAction={resolveComposerKeyAction}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );

    const textarea = container.querySelector(".composer-inline-editor");
    expect(textarea).not.toBeNull();
    expect(screen.getByRole("listbox", { name: "Mention suggestions" })).toBeTruthy();

    fireEvent.keyDown(textarea!, { key: "Escape", code: "Escape" });
    await waitFor(() =>
      expect(screen.queryByRole("listbox", { name: "Mention suggestions" })).toBeNull()
    );

    changeEditorText(textarea!, "@al" );
    expect(screen.getByRole("listbox", { name: "Mention suggestions" })).toBeTruthy();
  });

  it("renders users and room notification as sectioned mention suggestions", () => {
    render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        mentionCandidates={[mentionCandidates[2]!]}
        roomName="Direct room"
        document={documentFromText("@room")}
        onCancelReply={() => undefined}
        onSend={textSend(() => undefined)}
        onDocumentChange={textChange(() => undefined)}
      />
    );

    expect(screen.getByText("Room Notification")).toBeTruthy();
    expect(
      screen.getByRole("option", { name: "@room Notify the whole room" }).getAttribute("aria-selected")
    ).toBe("true");
  });

  describe("inline edit emoji surface (#498)", () => {
    beforeEach(() => {
      // The picker persists recent emojis to localStorage; keep each test's
      // picker grid deterministic.
      localStorage.removeItem("koushi-recent-emojis");
    });

    afterEach(() => {
      localStorage.removeItem("koushi-recent-emojis");
    });

    function renderEditComposer(onDocumentChange = vi.fn(), draftKey = "edit:a") {
      const rendered = render(
        <Composer
          editorOnly
          surface="edit"
          composerMode={{ kind: "plain" }}
          isSending={false}
          roomName=""
          document={documentFromText("Hello world")}
          draftKey={draftKey}
          onCancel={() => undefined}
          onCancelReply={() => undefined}
          onSend={textSend(() => undefined)}
          onDocumentChange={onDocumentChange}
        />
      );
      const editor = rendered.container.querySelector(".composer-inline-editor") as HTMLDivElement;
      return { ...rendered, editor, onDocumentChange };
    }

    it("exposes the shared emoji picker on the edit surface without edit-inappropriate controls", () => {
      const { container } = renderEditComposer();

      const emojiButton = screen.getByRole("button", { name: "Emoji" });
      expect(emojiButton).toBeTruthy();
      expect(emojiButton.getAttribute("aria-haspopup")).toBe("dialog");
      expect(emojiButton.getAttribute("aria-expanded")).toBe("false");
      expect(container.querySelector(".composer-edit-aux")).not.toBeNull();
      // Attachment, scheduled-send, and send controls stay out of edits.
      expect(screen.queryByRole("button", { name: "Attach file" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    });

    it("inserts a selected emoji at the caret in one mutation, closes the picker, and restores editor focus", async () => {
      const { editor, onDocumentChange } = renderEditComposer();
      changeEditorText(editor, "Hello world");
      // Caret after "Hello " (position 6).
      setInlineMentionEditorSelection(editor, 6, 6);

      const emojiButton = screen.getByRole("button", { name: "Emoji" });
      fireEvent.click(emojiButton);
      expect(emojiButton.getAttribute("aria-expanded")).toBe("true");
      const picker = screen.getByRole("dialog", { name: "Emoji" });
      fireEvent.click(within(picker).getAllByRole("button", { name: /grinning face$/i })[0]!);

      const nextDocument = onDocumentChange.mock.lastCall?.[0] as ComposerDocument;
      expect(plainBodyFromDocument(nextDocument)).toBe("Hello 😀world");
      expect(screen.queryByRole("dialog", { name: "Emoji" })).toBeNull();
      await waitFor(() => expect(document.activeElement).toBe(editor));
    });

    it("replaces a selected range with the chosen emoji", () => {
      const { editor, onDocumentChange } = renderEditComposer();
      changeEditorText(editor, "Hello world");
      // Select "world" (6..11).
      setInlineMentionEditorSelection(editor, 6, 11);

      fireEvent.click(screen.getByRole("button", { name: "Emoji" }));
      const picker = screen.getByRole("dialog", { name: "Emoji" });
      fireEvent.click(within(picker).getAllByRole("button", { name: /grinning face$/i })[0]!);

      const document = onDocumentChange.mock.lastCall?.[0] as ComposerDocument;
      expect(plainBodyFromDocument(document)).toBe("Hello 😀");
    });

    it("never reuses a selection captured for a previous draft after the draft key changes", () => {
      const onDocumentChange = vi.fn();
      const { container, rerender } = renderEditComposer(onDocumentChange, "edit:a");
      const editor = container.querySelector(".composer-inline-editor") as HTMLDivElement;
      changeEditorText(editor, "old body");
      setInlineMentionEditorSelection(editor, 4, 4);
      fireEvent.click(screen.getByRole("button", { name: "Emoji" }));

      // The same composer is reused for a different edit target while the
      // picker is still open; the old selection must not apply to the new
      // draft.
      rerender(
        <Composer
          editorOnly
          surface="edit"
          composerMode={{ kind: "plain" }}
          isSending={false}
          roomName=""
          document={documentFromText("new body")}
          draftKey="edit:b"
          onCancel={() => undefined}
          onCancelReply={() => undefined}
          onSend={textSend(() => undefined)}
          onDocumentChange={onDocumentChange}
        />
      );
      const newEditor = container.querySelector(".composer-inline-editor") as HTMLDivElement;
      changeEditorText(newEditor, "new body");
      // Caret at the end of the new draft.
      setInlineMentionEditorSelection(newEditor, 8, 8);
      fireEvent.click(
        within(screen.getByRole("dialog", { name: "Emoji" })).getAllByRole("button", {
          name: /grinning face$/i
        })[0]!
      );

      const document = onDocumentChange.mock.lastCall?.[0] as ComposerDocument;
      expect(plainBodyFromDocument(document)).toBe("new body😀");
    });
  });

  describe("unified send key for staged attachments (#send-key-unification)", () => {
    function renderComposerWithResolver(
      resolver: ResolveComposerKeyAction,
      overrides: {
        stagedUploadsReady?: boolean;
        hasStagedUploads?: boolean;
        document?: ComposerDocument;
      } = {}
    ) {
      const onSendSpy = vi.fn();
      const onSend = textSend(onSendSpy);
      const onSendStagedUploads = vi.fn();
      const rendered = render(
        <Composer
          composerMode={{ kind: "plain" }}
          isSending={false}
          hasStagedUploads={overrides.hasStagedUploads ?? Boolean(overrides.stagedUploadsReady)}
          stagedUploadsReady={overrides.stagedUploadsReady ?? false}
          roomName="Direct room"
          document={overrides.document ?? documentFromText("")}
          onCancelReply={() => undefined}
          resolveComposerKeyAction={resolver}
          onSend={onSend}
          onSendStagedUploads={onSendStagedUploads}
          onDocumentChange={textChange(vi.fn())}
        />
      );
      const editor = rendered.container.querySelector(".composer-inline-editor");
      return { ...rendered, editor: editor as HTMLDivElement, onSend: onSendSpy, onSendStagedUploads };
    }

    it("routes the send shortcut to the staged-attachment send when uploads are ready", async () => {
      const resolver = vi.fn(async (_s: unknown, _e: unknown, options: ComposerResolverOptions) =>
        options.send_enabled ? ("send" as const) : ("noop" as const)
      );
      const { editor, onSend, onSendStagedUploads } = renderComposerWithResolver(resolver, {
        stagedUploadsReady: true,
        document: documentFromText("")
      });

      fireEvent.keyDown(editor, { key: "Enter", code: "Enter", keyCode: 13 });

      await waitFor(() => expect(onSendStagedUploads).toHaveBeenCalledTimes(1));
      expect(onSend).not.toHaveBeenCalled();
      expect(resolver).toHaveBeenCalledWith(
        "main",
        expect.anything(),
        expect.objectContaining({ send_enabled: true })
      );
    });

    it("routes the send shortcut to the normal send when there are no staged uploads", async () => {
      const resolver = vi.fn(async (_s: unknown, _e: unknown, options: ComposerResolverOptions) =>
        options.send_enabled ? ("send" as const) : ("noop" as const)
      );
      const { editor, onSend, onSendStagedUploads } = renderComposerWithResolver(resolver, {
        stagedUploadsReady: false,
        document: documentFromText("hello")
      });

      fireEvent.keyDown(editor, { key: "Enter", code: "Enter", keyCode: 13 });

      await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
      expect(onSendStagedUploads).not.toHaveBeenCalled();
    });

    it("keeps the send shortcut disabled for uploads that are not ready", async () => {
      const resolver = vi.fn(async (_s: unknown, _e: unknown, options: ComposerResolverOptions) =>
        options.send_enabled ? ("send" as const) : ("noop" as const)
      );
      const { editor, onSend, onSendStagedUploads } = renderComposerWithResolver(resolver, {
        stagedUploadsReady: false,
        hasStagedUploads: true,
        document: documentFromText("")
      });

      fireEvent.keyDown(editor, { key: "Enter", code: "Enter", keyCode: 13 });

      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(onSend).not.toHaveBeenCalled();
      expect(onSendStagedUploads).not.toHaveBeenCalled();
      expect(resolver).toHaveBeenCalledWith(
        "main",
        expect.anything(),
        expect.objectContaining({ send_enabled: false })
      );
    });

    it("still inserts a newline on Shift+Enter with staged uploads", async () => {
      const resolver = vi.fn(async () => "insertNewline" as const);
      const { container, editor, onSend, onSendStagedUploads } = renderComposerWithResolver(
        resolver,
        {
          stagedUploadsReady: true,
          document: documentFromText("line one")
        }
      );

      fireEvent.keyDown(editor, { key: "Enter", code: "Enter", keyCode: 13, shiftKey: true });

      await waitFor(() =>
        expect(container.querySelector(".composer-inline-editor")?.textContent).toBe("line one\n")
      );
      expect(onSend).not.toHaveBeenCalled();
      expect(onSendStagedUploads).not.toHaveBeenCalled();
    });

    it("gives a non-empty composer body normal-send precedence over staged uploads", async () => {
      const resolver = vi.fn(async (_s: unknown, _e: unknown, options: ComposerResolverOptions) =>
        options.send_enabled ? ("send" as const) : ("noop" as const)
      );
      const { editor, onSend, onSendStagedUploads } = renderComposerWithResolver(resolver, {
        stagedUploadsReady: true,
        document: documentFromText("typed message")
      });

      fireEvent.keyDown(editor, { key: "Enter", code: "Enter", keyCode: 13 });

      await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
      expect(onSendStagedUploads).not.toHaveBeenCalled();
    });

    it("does not route to the staged send when text was typed during key resolution", async () => {
      let resolveAction!: (action: ComposerResolvedAction) => void;
      const resolver = vi.fn(
        () =>
          new Promise<ComposerResolvedAction>((resolve) => {
            resolveAction = resolve;
          })
      );
      const { editor, onSend, onSendStagedUploads } = renderComposerWithResolver(resolver, {
        stagedUploadsReady: true,
        document: documentFromText("")
      });

      fireEvent.keyDown(editor, { key: "Enter", code: "Enter", keyCode: 13 });
      // Text lands while the resolver is still pending.
      changeEditorText(editor, "typed during resolution");
      await act(async () => {
        resolveAction("send");
      });

      // The stale empty-body routing must not fire the staged send (which
      // would clear the freshly typed draft), nor send the captured empty doc.
      expect(onSendStagedUploads).not.toHaveBeenCalled();
      expect(onSend).not.toHaveBeenCalled();
    });

    it("keeps staged readiness out of send_enabled when no staged-send callback exists", async () => {
      const resolver = vi.fn(async (_s: unknown, _e: unknown, options: ComposerResolverOptions) =>
        options.send_enabled ? ("send" as const) : ("noop" as const)
      );
      const onSendSpy = vi.fn();
      const rendered = render(
        <Composer
          composerMode={{ kind: "plain" }}
          isSending={false}
          hasStagedUploads
          stagedUploadsReady
          roomName="Direct room"
          document={documentFromText("")}
          onCancelReply={() => undefined}
          resolveComposerKeyAction={resolver}
          onSend={textSend(onSendSpy)}
          onDocumentChange={textChange(vi.fn())}
        />
      );
      const editor = rendered.container.querySelector(".composer-inline-editor") as HTMLDivElement;

      fireEvent.keyDown(editor, { key: "Enter", code: "Enter", keyCode: 13 });

      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(onSendSpy).not.toHaveBeenCalled();
      expect(resolver).toHaveBeenCalledWith(
        "main",
        expect.anything(),
        expect.objectContaining({ send_enabled: false })
      );
    });
  });

  describe("mention autocomplete keyboard visibility (#480)", () => {
    const manyCandidates: MentionCandidate[] = Array.from({ length: 20 }, (_, index) => ({
      key: `@user-${index}:example.invalid`,
      label: `User ${index}`,
      target: {
        kind: "user" as const,
        user_id: `@user-${index}:example.invalid`,
        display_label: `User ${index}`
      }
    }));

    // Simulate a 100px-tall popup whose options are 40px tall (20 options
    // overflow it). Option rects move up as the popup scrolls, mirroring real
    // layout, so the scrollTop values converge like a real browser's.
    function mockOverflowingPopup(container: HTMLElement) {
      const popup = container.querySelector<HTMLElement>(".composer-autocomplete");
      if (!popup) {
        throw new Error("mention autocomplete popup not rendered");
      }
      const optionHeight = 40;
      const popupHeight = 100;
      Object.defineProperty(popup, "clientHeight", { value: popupHeight, configurable: true });
      popup.getBoundingClientRect = () =>
        ({
          top: 0,
          bottom: popupHeight,
          height: popupHeight
        }) as DOMRect;
      popup.querySelectorAll(".composer-autocomplete-option").forEach((option) => {
        const element = option as HTMLElement;
        // Read the live id: React reuses option nodes across candidate
        // refreshes, so the flat index must be resolved at call time.
        element.getBoundingClientRect = () => {
          const index = Number(element.id.split("-option-").at(-1));
          return {
            top: index * optionHeight - popup.scrollTop,
            height: optionHeight
          } as DOMRect;
        };
      });
      return popup;
    }

    function renderComposerWithMentions(
      surface: "main" | "thread" | "edit",
      candidates = manyCandidates
    ) {
      const rendered = render(
        <Composer
          surface={surface}
          composerMode={{ kind: "plain" }}
          isSending={false}
          mentionCandidates={candidates}
          roomName="Direct room"
          document={documentFromText("@")}
          onCancelReply={() => undefined}
          onSend={textSend(() => undefined)}
          onDocumentChange={textChange(() => undefined)}
        />
      );
      const editor = rendered.container.querySelector(".composer-inline-editor");
      if (!editor) {
        throw new Error("composer editor not rendered");
      }
      return { ...rendered, editor };
    }

    function activeOptionVisible(popup: HTMLElement): boolean {
      const active = popup.querySelector<HTMLElement>('[aria-selected="true"]');
      if (!active) {
        return false;
      }
      const popupRect = popup.getBoundingClientRect();
      const optionRect = active.getBoundingClientRect();
      const optionTop = optionRect.top - popupRect.top + popup.scrollTop;
      const optionBottom = optionTop + optionRect.height;
      return optionTop >= popup.scrollTop - 1 && optionBottom <= popup.scrollTop + popup.clientHeight + 1;
    }

    it.each(["main", "thread", "edit"] as const)(
      "scrolls the %s composer popup to keep the active option visible",
      (surface) => {
        // jsdom omits scrollIntoView; install a stub so a regression that
        // scrolls the outer page fails this spy instead of throwing.
        if (typeof Element.prototype.scrollIntoView !== "function") {
          Element.prototype.scrollIntoView = () => undefined;
        }
        const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");
        const { container, editor } = renderComposerWithMentions(surface);
        const popup = mockOverflowingPopup(container);
        expect(popup.scrollTop).toBe(0);

        // Arrow Down across the lower edge: index 3's option (top 120, bottom
        // 160) cannot fit in the 100px popup, so the popup scrolls to 60.
        for (let step = 0; step < 3; step += 1) {
          fireEvent.keyDown(editor, { key: "ArrowDown", code: "ArrowDown" });
        }
        expect(popup.scrollTop).toBe(60);
        expect(activeOptionVisible(popup)).toBe(true);
        expect(scrollIntoView).not.toHaveBeenCalled();

        // Arrow Down to the last option pins the popup at its maximum scroll.
        for (let step = 0; step < 16; step += 1) {
          fireEvent.keyDown(editor, { key: "ArrowDown", code: "ArrowDown" });
        }
        expect(popup.scrollTop).toBe(700);

        // Wraparound from last back to first scrolls back to the top.
        fireEvent.keyDown(editor, { key: "ArrowDown", code: "ArrowDown" });
        expect(popup.scrollTop).toBe(0);

        // Wraparound from first up to last scrolls to the bottom again.
        fireEvent.keyDown(editor, { key: "ArrowUp", code: "ArrowUp" });
        expect(popup.scrollTop).toBe(700);
        expect(activeOptionVisible(popup)).toBe(true);
        expect(scrollIntoView).not.toHaveBeenCalled();
      }
    );

    it("keeps the active option visible when candidates are refreshed", () => {
      const { container, editor, rerender } = renderComposerWithMentions("main");
      const popup = mockOverflowingPopup(container);
      for (let step = 0; step < 3; step += 1) {
        fireEvent.keyDown(editor, { key: "ArrowDown", code: "ArrowDown" });
      }
      expect(activeOptionVisible(popup)).toBe(true);

      // A refreshed candidate list replaces the options; the active index is
      // clamped and the popup keeps the resulting active option visible.
      const refreshed = manyCandidates.slice(2, 8);
      rerender(
        <Composer
          surface="main"
          composerMode={{ kind: "plain" }}
          isSending={false}
          mentionCandidates={refreshed}
          roomName="Direct room"
          document={documentFromText("@")}
          onCancelReply={() => undefined}
          onSend={textSend(() => undefined)}
          onDocumentChange={textChange(() => undefined)}
        />
      );
      const refreshedPopup = mockOverflowingPopup(container);
      expect(activeOptionVisible(refreshedPopup)).toBe(true);
    });
  });
});
