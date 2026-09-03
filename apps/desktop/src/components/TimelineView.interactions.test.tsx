// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { openExternalHttpUrl } from "../backend/linkMediaRuntime";

vi.mock("../backend/linkMediaRuntime", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../backend/linkMediaRuntime")>()),
  openExternalHttpUrl: vi.fn(async () => undefined)
}));

import {
  type CoreEventPayload,
  type TimelineItem,
  type TimelineMessageSource
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import {
  KEY,
  baseTransport,
  fileMessage,
  imageMessage,
  message
} from "./timelineViewTestSupport";
import {
  applyTimelineEvent,
  createTimelineStore,
  type TimelineStoreState
} from "../domain/timelineStore";
import { TimelineStoreContext } from "./timelineStoreContext";
import {
  MessageSourceDialog,
  TimelineView,
  clearTimelineViewportSessionMemoryForTests
} from "./TimelineView";
import type { MentionCandidate } from "../app/uiShared";
import { documentFromText } from "../domain/composerDocument";
import {
  inlineMentionEditorSelection,
  setInlineMentionEditorSelection
} from "./ImeTextControl";

function changeInlineEditorText(editor: HTMLDivElement, text: string) {
  if (editor.dataset.composing === "true") {
    let textNode = editor.querySelector<HTMLElement>("[data-composer-text]");
    if (!textNode) {
      textNode = document.createElement("span");
      textNode.dataset.composerText = "";
      editor.append(textNode);
    }
    textNode.textContent = text;
    fireEvent.input(editor, { inputType: "insertCompositionText", isComposing: true });
    return;
  }
  setInlineMentionEditorSelection(editor, 0, editor.textContent?.length ?? 0);
  fireEvent(
    editor,
    new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      inputType: "insertText",
      data: text
    })
  );
}

function expectLocalizedTooltip(button: HTMLButtonElement, label: string): void {
  fireEvent.focus(button);
  const tooltip = button.parentElement?.querySelector<HTMLElement>('[role="tooltip"]') ?? null;
  expect(tooltip?.textContent).toBe(label);
  expect(button.getAttribute("aria-describedby")).toBe(tooltip?.id);
  fireEvent.blur(button);
}

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("TimelineView", () => {
  it("routes sender profile intent with room and stable user identity only", () => {
    const onOpenSenderProfile = vi.fn();
    const onReply = vi.fn();
    const onOpenThread = vi.fn();
    const onOpenContextMenu = vi.fn();
    const item = {
      ...message("$sender-profile", "Profile target body"),
      sender: "@stable-profile-target:example.invalid",
      sender_label: "Duplicate Name"
    };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [item]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={onReply}
          onOpenThread={onOpenThread}
          onOpenContextMenu={onOpenContextMenu}
          onOpenSenderProfile={onOpenSenderProfile}
        />
      </TimelineStoreContext.Provider>
    );

    fireEvent.click(screen.getByRole("button", { name: "Open profile for Duplicate Name" }));
    expect(onOpenSenderProfile).toHaveBeenCalledTimes(1);
    expect(onOpenSenderProfile).toHaveBeenCalledWith(
      "!room:example.invalid",
      "@stable-profile-target:example.invalid"
    );
    expect(onReply).not.toHaveBeenCalled();
    expect(onOpenThread).not.toHaveBeenCalled();
    expect(onOpenContextMenu).not.toHaveBeenCalled();
  });

  it("keeps edit live conversion DOM value and selection across timeline rerenders", () => {
    const editable = { ...message("$edit-ime", "before"), can_edit: true };
    const makeStore = (item: TimelineItem): TimelineStoreState =>
      applyTimelineEvent(createTimelineStore(), {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      });
    const transport = baseTransport({});
    const view = (store: TimelineStoreState) => (
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    const { rerender } = render(view(makeStore(editable)));

    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
    fireEvent.compositionStart(textarea);
    changeInlineEditorText(textarea, "日本語変換中");
    setInlineMentionEditorSelection(textarea, 3, 5);
    rerender(view(makeStore({ ...editable, body: "stale timeline body", is_edited: true })));

    expect(textarea.textContent).toBe("日本語変換中");
    expect(inlineMentionEditorSelection(textarea)).toEqual({ start: 3, end: 5 });
  });

  it("discards a stale deferred edit newline after newer DOM input", async () => {
    let resolveAction!: (action: "insertNewline") => void;
    const action = new Promise<"insertNewline">((resolve) => {
      resolveAction = resolve;
    });
    const editMessage = vi.fn(async () => undefined);
    const editable = { ...message("$edit-deferred", "captured"), can_edit: true };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [editable]
      }
    });
    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ editMessage })}
          resolveComposerKeyAction={() => action}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
    setInlineMentionEditorSelection(textarea, 8);
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    changeInlineEditorText(textarea, "newer edit input");
    await act(async () => resolveAction("insertNewline"));
    fireEvent.click(screen.getByRole("button", { name: /save edit/i }));

    expect(textarea.textContent).toBe("newer edit input");
    expect(editMessage).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$edit-deferred",
      documentFromText("newer edit input")
    );
  });

  it.each([
    ["text", message("$edit-mention", "old body")],
    ["media caption", { ...imageMessage("$edit-mention"), body: "old body" }]
  ])("opens shared mention autocomplete in %s edit and submits a structured document", async (_surface, item) => {
    const editMessage = vi.fn(async () => undefined);
    const mentionCandidates: MentionCandidate[] = [
      {
        key: "@alice:example.invalid",
        label: "Alice",
        target: {
          kind: "user",
          user_id: "@alice:example.invalid",
          display_label: "Alice"
        }
      }
    ];
    const editable = {
      ...item,
      can_edit: true,
      actions: {
        can_copy: true,
        can_forward: true,
        can_reply: true,
        can_permalink: true,
        can_view_source: true,
        editable_document: documentFromText("old body")
      }
    };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [editable]
      }
    });
    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ editMessage })}
          mentionCandidates={mentionCandidates}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
    changeInlineEditorText(textarea, "@");
    expect(await screen.findByRole("option", { name: "Alice @alice:example.invalid" })).toBeTruthy();
    fireEvent.click(screen.getByRole("option", { name: "Alice @alice:example.invalid" }));
    expect(textarea.textContent).toBe("@Alice ");
    expect(document.querySelector(".composer-mention-pills")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /save edit/i }));
    expect(editMessage).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$edit-mention",
      {
        version: 2,
        inlines: [
          {
            kind: "mention",
            target: {
              kind: "user",
              user_id: "@alice:example.invalid",
              display_label: "Alice"
            },
            display_label: "Alice"
          }
        ]
      }
    );
  });

  describe("inline edit emoji surface (#498)", () => {
    afterEach(() => {
      // Selecting an emoji persists to the shared recent list in
      // localStorage; clear it so later picker tests stay deterministic.
      localStorage.removeItem("koushi-recent-emojis");
    });

    it.each([
      ["text", message("$edit-emoji", "old body")],
      ["media caption", { ...imageMessage("$edit-emoji"), body: "old body" }]
    ])(
      "opens the shared emoji picker in %s edit and saves a structured document with the emoji",
      async (_surface, item) => {
        const editMessage = vi.fn(async () => undefined);
        const editable = {
          ...item,
          can_edit: true,
          actions: {
            can_copy: true,
            can_forward: true,
            can_reply: true,
            can_permalink: true,
            can_view_source: true,
            editable_document: documentFromText("old body")
          }
        };
        const store = applyTimelineEvent(createTimelineStore(), {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [editable]
          }
        });

        render(
          <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
            <TimelineView
              timelineKey={KEY}
              roomId="!room:example.invalid"
              transport={baseTransport({ editMessage })}
              onReply={vi.fn()}
            />
          </TimelineStoreContext.Provider>
        );

        fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
        const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
        changeInlineEditorText(textarea, "old body");
        // Caret after "old " (position 4).
        setInlineMentionEditorSelection(textarea, 4, 4);

        fireEvent.click(screen.getByRole("button", { name: "Emoji" }));
        const picker = await screen.findByRole("dialog", { name: "Emoji" });
        fireEvent.click(within(picker).getAllByRole("button", { name: /grinning face$/i })[0]!);
        expect(textarea.textContent).toBe("old 😀body");
        expect(screen.queryByRole("dialog", { name: "Emoji" })).toBeNull();

        fireEvent.click(screen.getByRole("button", { name: /save edit/i }));
        expect(editMessage).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$edit-emoji",
          documentFromText("old 😀body")
        );
      }
    );

    it("cancel does not submit and reopening edit starts from the authoritative document", () => {
      const editMessage = vi.fn(async () => undefined);
      const editable = {
        ...message("$edit-emoji-cancel", "old body"),
        can_edit: true,
        actions: {
          can_copy: true,
          can_forward: true,
          can_reply: true,
          can_permalink: true,
          can_view_source: true,
          editable_document: documentFromText("old body")
        }
      };
      const store = applyTimelineEvent(createTimelineStore(), {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [editable]
        }
      });

      render(
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({ editMessage })}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      );

      fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
      const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
      changeInlineEditorText(textarea, "old body");
      setInlineMentionEditorSelection(textarea, 4, 4);
      fireEvent.click(screen.getByRole("button", { name: "Emoji" }));
      fireEvent.click(
        within(screen.getByRole("dialog", { name: "Emoji" })).getAllByRole("button", {
          name: /grinning face$/i
        })[0]!
      );
      expect(textarea.textContent).toBe("old 😀body");

      fireEvent.click(screen.getByRole("button", { name: /cancel edit/i }));
      expect(editMessage).not.toHaveBeenCalled();
      expect(screen.queryByRole("textbox", { name: /edit.*body/i })).toBeNull();

      // Reopening starts from the authoritative editable document.
      fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
      const reopened = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
      expect(reopened.textContent).toBe("old body");
    });
  });

  it("keeps an existing mention when its projected label is only an MXID fallback", () => {
    const editMessage = vi.fn(async () => undefined);
    const editable = {
      ...message("$edit-mention-fallback", "hello @Alice"),
      can_edit: true,
      actions: {
        can_copy: true,
        can_forward: true,
        can_reply: true,
        can_permalink: true,
        can_view_source: true,
        editable_document: {
          version: 2 as const,
          inlines: [
            { kind: "text" as const, text: "hello " },
            {
              kind: "mention" as const,
              target: {
                kind: "user" as const,
                user_id: "@alice:example.invalid",
                display_label: "alice:example.invalid"
              },
              display_label: "Alice"
            }
          ]
        }
      }
    };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [editable]
      }
    });
    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ editMessage })}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    fireEvent.click(screen.getByRole("button", { name: /save edit/i }));

    expect(editMessage).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$edit-mention-fallback",
      editable.actions.editable_document
    );
  });

  it("shows a Shift+Enter edit newline immediately and saves that body", async () => {
    const editMessage = vi.fn(async () => undefined);
    const editable = { ...message("$edit-newline", "helloworld"), can_edit: true };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [editable]
      }
    });
    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ editMessage })}
          resolveComposerKeyAction={async () => "insertNewline"}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
    setInlineMentionEditorSelection(textarea, 5);
    fireEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      shiftKey: true
    });

    await waitFor(() => {
      expect(textarea.textContent).toBe("hello\nworld");
      expect(inlineMentionEditorSelection(textarea)).toEqual({ start: 6, end: 6 });
    });

    fireEvent.click(screen.getByRole("button", { name: /save edit/i }));

    expect(editMessage).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$edit-newline",
      documentFromText("hello\nworld")
    );
  });

  it("sends the edit value captured when deferred Enter was pressed", async () => {
    let resolveAction!: (action: "send") => void;
    const action = new Promise<"send">((resolve) => {
      resolveAction = resolve;
    });
    const editMessage = vi.fn(async () => undefined);
    const editable = { ...message("$edit-send-snapshot", "intent snapshot"), can_edit: true };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [editable]
      }
    });
    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ editMessage })}
          resolveComposerKeyAction={() => action}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLDivElement;
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    changeInlineEditorText(textarea, "later edit input");
    await act(async () => resolveAction("send"));

    expect(editMessage).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$edit-send-snapshot",
      documentFromText("intent snapshot")
    );
  });

  it("keeps the reaction emoji picker bound to its message row from the floating layer", async () => {
    const sendReaction = vi.fn(async () => undefined);
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react", "React here")]
      }
    });
    const transport = baseTransport({ sendReaction });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const article = screen.getByText("React here").closest("article");
    expect(article).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    // The panel lives in the body-level floating layer so overflow-clipped
    // panes cannot cut it off, yet the selection still targets this row.
    expect(article!.contains(picker)).toBe(false);
    expect(picker.parentElement).toBe(document.body);

    fireEvent.click(screen.getByRole("button", { name: /grinning face$/i }));

    await waitFor(() => {
      expect(sendReaction).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$react",
        "😀"
      );
    });
  });

  it("dismisses the reaction emoji picker only for presses outside the floating panel", async () => {
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-dismiss", "React and dismiss")]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));
    const picker = await screen.findByRole("dialog", { name: /emoji/i });

    // The panel is not a descendant of the row, so a row-scoped containment
    // check would close it on its own emoji presses.
    fireEvent.mouseDown(within(picker).getByRole("searchbox"));
    expect(screen.queryByRole("dialog", { name: /emoji/i })).not.toBeNull();

    fireEvent.mouseDown(document.body);
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: /emoji/i })).toBeNull();
    });
  });

  it("opens the reaction emoji picker above when the composer-side space is insufficient", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        height = 240;
      } else if (
        this.classList.contains("reaction-control") ||
        this.classList.contains("message-action")
      ) {
        top = 200;
      } else if (this.classList.contains("main-pane")) {
        height = 320;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-near-composer", "React near composer")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.classList.contains("is-above")).toBe(true);
    expect(picker.classList.contains("is-below")).toBe(false);
  });

  it("orders and routes distinct reply actions in a fully actionable room row", () => {
    const onReply = vi.fn();
    const onOpenThread = vi.fn();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$reply-inline", "Reply inline"),
            can_edit: true,
            actions: {
              can_copy: true,
              can_forward: false,
              can_reply: true,
              can_permalink: false,
              can_view_source: false
            }
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={onReply}
          onOpenThread={onOpenThread}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Reply inline").closest("article");
    expect(row).not.toBeNull();

    const actionButtons = Array.from(
      row!.querySelectorAll<HTMLButtonElement>(".message-actions .message-action")
    );
    expect(actionButtons.map((button) => button.getAttribute("aria-label"))).toEqual([
      "Add reaction",
      "Reply to message",
      "Reply in thread",
      "Edit message",
      "Pin message",
      "Message actions"
    ]);

    const replyButton = within(row!).getByRole<HTMLButtonElement>("button", {
      name: "Reply to message"
    });
    const replyInThreadButton = within(row!).getByRole<HTMLButtonElement>("button", {
      name: "Reply in thread"
    });
    const replyIcon = replyButton.querySelector<SVGElement>(
      '[data-message-action-icon="reply"]'
    );
    const replyInThreadIcon = replyInThreadButton.querySelector<SVGElement>(
      '[data-message-action-icon="reply-in-thread"]'
    );
    expect(replyIcon?.getAttribute("aria-hidden")).toBe("true");
    expect(replyInThreadIcon?.getAttribute("aria-hidden")).toBe("true");
    expectLocalizedTooltip(replyButton, "Reply to message");
    expectLocalizedTooltip(replyInThreadButton, "Reply in thread");

    fireEvent.click(replyButton);
    expect(onReply).toHaveBeenCalledWith("!room:example.invalid", "$reply-inline");
    expect(onOpenThread).not.toHaveBeenCalled();
    fireEvent.click(replyInThreadButton);
    expect(onOpenThread).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$reply-inline",
      "newThreadDraft"
    );
    expect(onReply).toHaveBeenCalledTimes(1);

    fireEvent.click(within(row!).getByRole("button", { name: "Message actions" }));
    const menu = within(row!).getByRole("menu", { name: "Message actions" });
    expect(within(menu).queryByRole("menuitem", { name: "Reply to message" })).toBeNull();
  });

  it("preserves reply action order when edit and more actions are absent", () => {
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$reply-inline-minimal", "Reply inline minimal")]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Reply inline minimal").closest("article");
    expect(row).not.toBeNull();
    expect(
      Array.from(
        row!.querySelectorAll<HTMLButtonElement>(".message-actions .message-action")
      ).map((button) => button.getAttribute("aria-label"))
    ).toEqual(["Add reaction", "Reply to message", "Reply in thread", "Pin message"]);
  });

  it("localizes both reply action names and tooltips in Japanese", () => {
    setActiveLocaleProfile("ja", "none");
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$reply-inline-ja", "返信操作")]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("返信操作").closest("article");
    expect(row).not.toBeNull();
    const replyButton = within(row!).getByRole<HTMLButtonElement>("button", {
      name: "メッセージに返信"
    });
    const replyInThreadButton = within(row!).getByRole<HTMLButtonElement>("button", {
      name: "スレッドで返信"
    });
    expectLocalizedTooltip(replyButton, "メッセージに返信");
    expectLocalizedTooltip(replyInThreadButton, "スレッドで返信");
  });

  it("does not expose reply actions for redacted, hidden, or non-replyable rows", () => {
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          { ...message("$reply-redacted", "Redacted reply"), is_redacted: true },
          { ...message("$reply-hidden", "Hidden reply"), is_hidden: true },
          {
            ...message("$reply-bodyless", "Bodyless reply"),
            body: null,
            actions: {
              can_copy: false,
              can_forward: false,
              can_reply: false,
              can_permalink: false,
              can_view_source: false
            }
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    expect(document.querySelector('article[data-content-event-id="$reply-hidden"]')).toBeNull();
    for (const eventId of ["$reply-redacted", "$reply-bodyless"]) {
      const row = document.querySelector<HTMLElement>(
        `article[data-content-event-id="${eventId}"]`
      );
      expect(row).not.toBeNull();
      expect(within(row!).queryByRole("button", { name: "Reply to message" })).toBeNull();
      expect(within(row!).queryByRole("button", { name: "Reply in thread" })).toBeNull();
    }
  });

  it("uses the Rust reply capability for captionless media rows", () => {
    const replyableActions = {
      can_copy: false,
      can_forward: false,
      can_permalink: false,
      can_view_source: false,
      can_reply: true
    };
    const nonReplyableActions = { ...replyableActions, can_reply: false };
    const onReply = vi.fn();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...fileMessage("$captionless-media-reply"),
            body: null,
            actions: replyableActions
          },
          {
            ...fileMessage("$captionless-media-no-reply"),
            body: null,
            actions: nonReplyableActions
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={onReply}
        />
      </TimelineStoreContext.Provider>
    );

    const replyableRow = document.querySelector<HTMLElement>(
      'article[data-content-event-id="$captionless-media-reply"]'
    );
    const nonReplyableRow = document.querySelector<HTMLElement>(
      'article[data-content-event-id="$captionless-media-no-reply"]'
    );
    expect(replyableRow).not.toBeNull();
    expect(nonReplyableRow).not.toBeNull();

    fireEvent.click(within(replyableRow!).getByRole("button", { name: "Reply to message" }));
    expect(onReply).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$captionless-media-reply"
    );
    expect(
      within(nonReplyableRow!).queryByRole("button", { name: "Reply to message" })
    ).toBeNull();
  });

  it("autosaves sender aliases from the message action menu", () => {
    const onSetLocalUserAlias = vi.fn();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$alias", "Alias me")]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onSetLocalUserAlias={onSetLocalUserAlias}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Alias me").closest("article");
    expect(row).not.toBeNull();

    fireEvent.click(within(row!).getByRole("button", { name: "Message actions" }));
    fireEvent.click(
      within(row!).getByRole("menuitem", { name: "Set alias for Unknown user" })
    );

    fireEvent.change(screen.getByRole("textbox", { name: "Alias" }), {
      target: { value: "Builder Bob" }
    });

    expect(screen.queryByRole("button", { name: "Save alias" })).toBeNull();
    expect(onSetLocalUserAlias).toHaveBeenCalledWith(
      "@bob:example.invalid",
      "Builder Bob"
    );
  });

  it("navigates the message action menu with arrow keys", () => {
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$arrows", "Arrow me"),
            can_edit: true,
            actions: {
              can_copy: true,
              can_forward: true,
              can_reply: true,
              can_permalink: true,
              can_view_source: true
            }
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Arrow me").closest("article");
    expect(row).not.toBeNull();

    fireEvent.click(within(row!).getByRole("button", { name: "Message actions" }));
    const menu = within(row!).getByRole("menu");
    const items = within(row!).getAllByRole("menuitem");
    expect(items.length).toBeGreaterThanOrEqual(2);

    // The first item is focused when the menu opens; arrows rove and wrap.
    expect(document.activeElement).toBe(items[0]);
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(items[1]);
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(items[2 % items.length]);
    fireEvent.keyDown(menu, { key: "Home" });
    expect(document.activeElement).toBe(items[0]);
    fireEvent.keyDown(menu, { key: "End" });
    expect(document.activeElement).toBe(items[items.length - 1]);
  });

  it("shrinks the reaction emoji picker to the visible space instead of clipping it", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        top = 120;
        height = 260;
      } else if (
        this.classList.contains("reaction-control") ||
        this.classList.contains("message-action")
      ) {
        top = 320;
      } else if (this.classList.contains("main-pane")) {
        top = 100;
        height = 500;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-tight-space", "React with tight space")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.style.getPropertyValue("block-size")).toBe("194px");
  });

  it("keeps the reaction emoji picker at its preferred block size when there is room", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        top = 80;
        height = 720;
      } else if (
        this.classList.contains("reaction-control") ||
        this.classList.contains("message-action")
      ) {
        top = 160;
      } else if (this.classList.contains("main-pane")) {
        top = 60;
        height = 760;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-roomy-space", "React with roomy space")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.classList.contains("is-below")).toBe(true);
    expect(picker.style.getPropertyValue("block-size")).toBe("520px");
  });

  it("updates the reaction emoji picker size when the visible space changes", async () => {
    let reactionControlTop = 320;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        top = 120;
        height = 260;
      } else if (
        this.classList.contains("reaction-control") ||
        this.classList.contains("message-action")
      ) {
        top = reactionControlTop;
      } else if (this.classList.contains("main-pane")) {
        top = 100;
        height = 500;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-resized-space", "React after resize")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.style.getPropertyValue("block-size")).toBe("194px");

    reactionControlTop = 150;
    fireEvent(window, new Event("resize"));

    await waitFor(() => {
      expect(picker.classList.contains("is-below")).toBe(true);
      expect(picker.style.getPropertyValue("block-size")).toBe("200px");
    });
  });

  it("renders a Rust-positioned failed gap between known rows and retries non-destructively", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const repairTimeline = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      repairTimeline
    });
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        continuity={{
          kind: "failedIncomplete",
          generation: 3,
          gap_count: 1,
          batches_processed: 2,
          failure_kind: "sdk"
        }}
      />
    );
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$older:example.invalid", "Older"), message("$newer:example.invalid", "Newer")]
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          GapPositionsUpdated: {
            key: KEY,
            actor_generation: 0,
            generation: 3,
            positions: [
              { id: { topology_revision: "7", ordinal: 0 }, before_item_index: 1 }
            ]
          }
        }
      });
    });

    const frames = await screen.findAllByRole("article");
    const gap = await screen.findByTestId("timeline-gap-row");
    expect(frames[0]?.textContent).toContain("Older");
    expect(frames[1]?.textContent).toContain("Newer");
    expect(gap.parentElement?.previousElementSibling?.textContent).toContain("Older");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(repairTimeline).toHaveBeenCalledWith("!room:example.invalid");
  });

  it("shows visible copy controls in the message source dialog", () => {
    const source: TimelineMessageSource = {
      event_id: "$source:example.invalid",
      sender: "@alice:example.invalid",
      timestamp_ms: 1_800_000_000_000,
      body: "source body",
      in_reply_to_event_id: null,
      thread_root: null,
      is_redacted: false,
      is_edited: false,
      has_media: false,
      megolm_session_fingerprint: "AbCdEfGhIjKl",
      megolm_message_index: 0,
      megolm_session_rotation_reason: "expiredTime",
      original_json: {
        type: "m.room.message",
        content: { body: "source body", msgtype: "m.text" }
      }
    };

    render(<MessageSourceDialog source={source} onClose={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Copy event ID" }).textContent).toContain(
      "Copy event ID"
    );
    expect(
      screen.getByRole("button", { name: "Copy original event source" }).textContent
    ).toContain("Copy original event source");
    expect(
      screen.getByRole("button", { name: "Copy Megolm session fingerprint" }).textContent
    ).toContain("Copy");
    expect(screen.getByText("Megolm message index")).toBeTruthy();
    expect(screen.getByText("0")).toBeTruthy();
    expect(screen.getByText("Time limit reached")).toBeTruthy();
  });

  it("omits Megolm rotation attribution when Rust supplies no local reason", () => {
    const source: TimelineMessageSource = {
      event_id: "$source-peer:example.invalid",
      sender: "@bob:example.invalid",
      timestamp_ms: 1_800_000_000_000,
      body: "source body",
      in_reply_to_event_id: null,
      thread_root: null,
      is_redacted: false,
      is_edited: false,
      has_media: false,
      megolm_session_fingerprint: "AbCdEfGhIjKl"
    };

    render(<MessageSourceDialog source={source} onClose={vi.fn()} />);

    expect(screen.queryByText("Session change reason")).toBeNull();
  });

  it("shows an honest unavailable Megolm rotation reason", () => {
    const source: TimelineMessageSource = {
      event_id: "$source-unavailable:example.invalid",
      sender: "@alice:example.invalid",
      timestamp_ms: 1_800_000_000_000,
      body: "source body",
      in_reply_to_event_id: null,
      thread_root: null,
      is_redacted: false,
      is_edited: false,
      has_media: false,
      megolm_session_fingerprint: "AbCdEfGhIjKl",
      megolm_session_rotation_reason: "notRetained"
    };

    render(<MessageSourceDialog source={source} onClose={vi.fn()} />);

    expect(screen.getByText("Reason unavailable")).toBeTruthy();
  });

  it("renders plain-text URLs as anchors from Rust-projected link ranges", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const text = "Check https://example.com/page and https://example.com/page out";
    const item: TimelineItem = {
      ...message("$url:example.invalid", text),
      link_ranges: [
        {
          url: "https://example.com/page",
          start_utf16: 6,
          end_utf16: 30
        },
        {
          url: "https://example.com/page",
          start_utf16: 35,
          end_utf16: 59
        }
      ]
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const links = await screen.findAllByRole("link", { name: "https://example.com/page" });
    expect(links).toHaveLength(2);
    for (const link of links) {
      expect(link.getAttribute("href")).toBe("https://example.com/page");
      expect(link.getAttribute("target")).toBe("_blank");
    }

    fireEvent.click(links[0]);
    await waitFor(() => {
      expect(openExternalHttpUrl).toHaveBeenCalledWith("https://example.com/page");
    });
  });

  it("routes a plain-text matrix.to room link into the app instead of the browser", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const permalink = "https://matrix.to/#/%23room%3Aexample.invalid";
    const text = `Join ${permalink}`;
    const item: TimelineItem = {
      ...message("$matrixto-plain:example.invalid", text),
      link_ranges: [
        { url: permalink, start_utf16: "Join ".length, end_utf16: text.length }
      ]
    };
    const onOpenMatrixTarget = vi.fn();

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onOpenMatrixTarget={onOpenMatrixTarget}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const link = await screen.findByRole("link", { name: permalink });
    fireEvent.click(link);
    await waitFor(() => {
      expect(onOpenMatrixTarget).toHaveBeenCalledWith({
        kind: "room",
        roomIdOrAlias: "#room:example.invalid",
        viaServers: []
      });
    });
    // A Matrix target is in-app navigation, so it must never reach the browser.
    expect(openExternalHttpUrl).not.toHaveBeenCalledWith(permalink);
  });

  it("routes a formatted matrix.to anchor into the app, keeping its via servers", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const permalink =
      "https://matrix.to/#/%23room%3Aexample.invalid?via=first.invalid&via=second.invalid";
    const item: TimelineItem = {
      ...message("$matrixto-formatted:example.invalid", "#room:example.invalid"),
      formatted: {
        html: `<a href="${permalink}">#room:example.invalid</a>`,
        plain_text: "#room:example.invalid",
        code_blocks: []
      }
    };
    const onOpenMatrixTarget = vi.fn();

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onOpenMatrixTarget={onOpenMatrixTarget}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const link = await screen.findByRole("link", { name: "#room:example.invalid" });
    fireEvent.click(link);
    await waitFor(() => {
      expect(onOpenMatrixTarget).toHaveBeenCalledWith({
        kind: "room",
        roomIdOrAlias: "#room:example.invalid",
        viaServers: ["first.invalid", "second.invalid"]
      });
    });
    expect(openExternalHttpUrl).not.toHaveBeenCalledWith(permalink);
  });
});
