// @vitest-environment jsdom

import { createRef, useState, type FormEventHandler } from "react";
import { act, cleanup, createEvent, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ImeInlineMentionEditor,
  ImeSafeForm,
  inlineMentionEditorSelection,
  setInlineMentionEditorSelection,
  ImeTextArea,
  ImeTextField,
  SecureImeTextField,
  type ImeInlineMentionEditorHandle
} from "./ImeTextControl";
import type { ComposerDocument } from "../domain/types";

const EDITOR_LABEL = "message";

const mentionDocument: ComposerDocument = {
  version: 2,
  inlines: [
    { kind: "text", text: "A" },
    {
      kind: "mention",
      target: { kind: "user", user_id: "@alice:example.invalid", display_label: "Alice" },
      display_label: "Alice"
    },
    { kind: "text", text: "B" }
  ]
};

function ControlledMentionEditor({
  initial = mentionDocument,
  onChange = () => undefined,
  onInput = () => undefined
}: {
  initial?: ComposerDocument;
  onChange?: (document: ComposerDocument) => void;
  onInput?: FormEventHandler<HTMLDivElement>;
}) {
  const [document, setDocument] = useState(initial);
  return (
    <ImeInlineMentionEditor
      aria-label={EDITOR_LABEL}
      document={document}
      syncKey="message-a"
      onInput={onInput}
      onDocumentChange={(next) => {
        setDocument(next);
        onChange(next);
      }}
    />
  );
}

function setSelection(
  startNode: Node,
  startOffset: number,
  endNode = startNode,
  endOffset = startOffset
) {
  const range = document.createRange();
  range.setStart(startNode, startOffset);
  range.setEnd(endNode, endOffset);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

function beforeInput(control: HTMLElement, inputType: string, data: string | null = null) {
  fireEvent(
    control,
    new InputEvent("beforeinput", { bubbles: true, cancelable: true, inputType, data })
  );
}

const fieldLabel = "field";
const secretLabel = "secret";
const formLabel = "form";
const submitLabel = "Submit";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("IME text controls", () => {

describe("mention caret anchors (#875)", () => {
  const bareMentionDocument: ComposerDocument = {
    version: 2,
    inlines: [
      {
        kind: "mention",
        target: { kind: "user", user_id: "@alice:example.invalid", display_label: "Alice" },
        display_label: "Alice"
      }
    ]
  };

  it("renders a zero-width text box on both sides of a mention with no text beside it", () => {
    render(<ControlledMentionEditor initial={bareMentionDocument} />);
    const control = screen.getByRole("textbox", { name: "message" }) as HTMLDivElement;
    const children = Array.from(control.childNodes) as HTMLElement[];
    expect(children).toHaveLength(3);
    expect(children[0].hasAttribute("data-composer-caret-anchor")).toBe(true);
    expect(children[0].textContent).toBe("\u200b");
    expect(children[1].hasAttribute("data-composer-mention")).toBe(true);
    expect(children[2].hasAttribute("data-composer-caret-anchor")).toBe(true);
  });

  // Issue #956: an anchor is a real caret stop for the engine but collapses to
  // the offset beside it, so before the fix an arrow press at a pill boundary
  // consumed a press without moving the document caret. These assertions are on
  // document offsets, which is what the user sees move; the native caret itself
  // is covered by the Playwright regression, since jsdom has no caret motion.
  describe("arrow traversal (#956)", () => {
    const adjacentMentionsDocument: ComposerDocument = {
      version: 2,
      inlines: [
        {
          kind: "mention",
          target: { kind: "user", user_id: "@alice:example.invalid", display_label: "Alice" },
          display_label: "Alice"
        },
        {
          kind: "mention",
          target: { kind: "user", user_id: "@bob:example.invalid", display_label: "Bob" },
          display_label: "Bob"
        }
      ]
    };

    function mentionEditor(initial: ComposerDocument) {
      render(<ControlledMentionEditor initial={initial} />);
      return screen.getByRole("textbox", { name: "message" }) as HTMLDivElement;
    }

    it("crosses two adjacent pills in one press each", () => {
      const control = mentionEditor(adjacentMentionsDocument);
      // Three anchors — leading, shared, trailing — are what produced the three
      // dead presses in the report.
      expect(control.querySelectorAll("[data-composer-caret-anchor]")).toHaveLength(3);

      setInlineMentionEditorSelection(control, 0);
      fireEvent.keyDown(control, { key: "ArrowRight" });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 1, end: 1 });

      fireEvent.keyDown(control, { key: "ArrowRight" });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 2, end: 2 });
    });

    it("does not move past either document edge", () => {
      const control = mentionEditor(adjacentMentionsDocument);

      setInlineMentionEditorSelection(control, 2);
      fireEvent.keyDown(control, { key: "ArrowRight" });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 2, end: 2 });

      setInlineMentionEditorSelection(control, 0);
      fireEvent.keyDown(control, { key: "ArrowLeft" });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 0, end: 0 });
    });

    it("crosses pills backward one press at a time", () => {
      const control = mentionEditor(adjacentMentionsDocument);

      setInlineMentionEditorSelection(control, 2);
      fireEvent.keyDown(control, { key: "ArrowLeft" });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 1, end: 1 });

      fireEvent.keyDown(control, { key: "ArrowLeft" });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 0, end: 0 });
    });

    it("extends the selection over a pill under Shift", () => {
      const control = mentionEditor(adjacentMentionsDocument);

      setInlineMentionEditorSelection(control, 0);
      fireEvent.keyDown(control, { key: "ArrowRight", shiftKey: true });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 0, end: 1 });
    });

    it("leaves a modified arrow to the engine", () => {
      const control = mentionEditor(adjacentMentionsDocument);

      setInlineMentionEditorSelection(control, 0);
      for (const modifier of ["ctrlKey", "altKey", "metaKey"] as const) {
        fireEvent.keyDown(control, { key: "ArrowRight", [modifier]: true });
        expect(inlineMentionEditorSelection(control)).toEqual({ start: 0, end: 0 });
      }
    });

    it("leaves ordinary text beside a pill to the engine", () => {
      const control = mentionEditor(mentionDocument);
      expect(control.querySelectorAll("[data-composer-caret-anchor]")).toHaveLength(0);

      setInlineMentionEditorSelection(control, 0);
      fireEvent.keyDown(control, { key: "ArrowRight" });
      // jsdom moves no caret of its own, so an unchanged offset proves the
      // composer did not take the press over.
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 0, end: 0 });
    });

    it("leaves the arrow keys to the IME while composing", () => {
      const control = mentionEditor(adjacentMentionsDocument);

      setInlineMentionEditorSelection(control, 0);
      fireEvent.compositionStart(control);
      fireEvent.keyDown(control, { key: "ArrowRight" });
      expect(inlineMentionEditorSelection(control)).toEqual({ start: 0, end: 0 });
    });
  });

  it("keeps a mention flanked by text free of caret anchors", () => {
    render(<ControlledMentionEditor />);
    const control = screen.getByRole("textbox", { name: "message" }) as HTMLDivElement;
    expect(control.querySelectorAll("[data-composer-caret-anchor]")).toHaveLength(0);
  });

  it("maps both mention boundaries to a text box outside the pill", () => {
    render(<ControlledMentionEditor initial={bareMentionDocument} />);
    const control = screen.getByRole("textbox", { name: "message" }) as HTMLDivElement;
    const mention = control.querySelector<HTMLElement>("[data-composer-mention]");
    if (!mention) throw new Error("missing mention");

    for (const offset of [0, 1]) {
      setInlineMentionEditorSelection(control, offset);
      const selection = window.getSelection();
      expect(selection?.anchorNode?.nodeType).toBe(Node.TEXT_NODE);
      const anchor = (selection?.anchorNode as Text).parentElement;
      expect(anchor?.hasAttribute("data-composer-caret-anchor")).toBe(true);
      // The browser paints a collapsed parent-level caret against a guess; the
      // anchor keeps it on real text outside the pill.
      expect(selection?.anchorNode === control).toBe(false);
      expect(inlineMentionEditorSelection(control)).toEqual({ start: offset, end: offset });
    }
  });

  it("keeps the anchor character out of the document the composer publishes", () => {
    const onChange = vi.fn();
    render(<ControlledMentionEditor initial={bareMentionDocument} onChange={onChange} />);
    const control = screen.getByRole("textbox", { name: "message" }) as HTMLDivElement;
    expect(control.textContent).toContain("\u200b");

    setInlineMentionEditorSelection(control, 0);
    beforeInput(control, "insertText", "hi");

    const published = onChange.mock.lastCall?.[0] as ComposerDocument;
    expect(published.inlines[0]).toEqual({ kind: "text", text: "hi" });
    expect(published.inlines[1]).toMatchObject({ kind: "mention", display_label: "Alice" });
    expect(JSON.stringify(published)).not.toContain("\u200b");
    expect(inlineMentionEditorSelection(control)).toEqual({ start: 2, end: 2 });
  });
});

describe("trailing newline rendering (#471)", () => {
  it("renders a trailing <br> sentinel when the document ends with a newline", () => {
    render(
      <ControlledMentionEditor
        initial={{ version: 2, inlines: [{ kind: "text", text: "foo\n" }] }}
      />
    );
    const control = screen.getByRole("textbox", { name: "message" });
    const last = control.lastChild;
    expect(last).not.toBeNull();
    expect((last as HTMLElement).tagName).toBe("BR");
    expect((last as HTMLElement).hasAttribute("data-composer-sentinel")).toBe(true);
    // The sentinel never counts toward the model text.
    expect(control.textContent).toBe("foo\n");
  });

  it("does not append a sentinel without a trailing newline", () => {
    render(
      <ControlledMentionEditor initial={{ version: 2, inlines: [{ kind: "text", text: "foo" }] }} />
    );
    const control = screen.getByRole("textbox", { name: "message" });
    expect(control.querySelector("br")).toBeNull();
  });

  it("mid-text newlines render without a sentinel", () => {
    render(
      <ControlledMentionEditor
        initial={{ version: 2, inlines: [{ kind: "text", text: "foo\nbar" }] }}
      />
    );
    const control = screen.getByRole("textbox", { name: "message" });
    expect(control.querySelector("br")).toBeNull();
    expect(control.textContent).toBe("foo\nbar");
  });

  it("keeps the caret at the end of the document after a trailing newline (round-trip)", () => {
    const ref = createRef<ImeInlineMentionEditorHandle>();
    render(
      <ImeInlineMentionEditor
        aria-label={EDITOR_LABEL}
        ref={ref}
        document={{ version: 2, inlines: [{ kind: "text", text: "foo\n" }] }}
        syncKey="message-a"
        onDocumentChange={() => undefined}
      />
    );
    const control = screen.getByRole("textbox", { name: "message" });
    // The sentinel gives the empty final line a paintable box.
    expect(control.querySelector("br[data-composer-sentinel]")).not.toBeNull();
    // documentLength("foo\n") === 4 and the caret must stay there, past the
    // newline (visually the start of line 2), never snapped back to line 1.
    const selection = ref.current?.selection();
    expect(selection?.start).toBe(4);
    expect(selection?.end).toBe(4);
    const text = control.firstChild?.firstChild;
    expect(text?.nodeType).toBe(Node.TEXT_NODE);
    const range = control.ownerDocument.getSelection()?.getRangeAt(0);
    expect(range?.startContainer).toBe(text);
    expect(range?.startOffset).toBe(4);
  });

  it("restoreDocumentSelection moves the caret past the trailing newline", () => {
    const ref = createRef<ImeInlineMentionEditorHandle>();
    function StatefulEditor() {
      const [document, setDocument] = useState<ComposerDocument>({
        version: 2,
        inlines: [{ kind: "text", text: "foo" }]
      });
      return (
        <ImeInlineMentionEditor
          aria-label={EDITOR_LABEL}
          ref={ref}
          document={document}
          syncKey="message-a"
          onDocumentChange={setDocument}
        />
      );
    }
    render(<StatefulEditor />);
    const control = screen.getByRole("textbox", { name: "message" });
    // Simulate Shift+Enter at the end: insert the newline then restore.
    act(() => {
      ref.current?.commit({
        document: { version: 2, inlines: [{ kind: "text", text: "foo\n" }] },
        selection: { start: 4, end: 4 }
      });
    });
    const sentinel = control.querySelector("br[data-composer-sentinel]");
    expect(sentinel).not.toBeNull();
    const text = control.firstChild?.firstChild;
    const range = control.ownerDocument.getSelection()?.getRangeAt(0);
    expect(range?.startContainer).toBe(text);
    expect(range?.startOffset).toBe(4);
    // Reading the caret back still reports the end of the document.
    const selection = ref.current?.selection();
    expect(selection?.start).toBe(4);
  });

  it("maps a caret placed after the sentinel back to the document end (collapsed and range)", () => {
    const ref = createRef<ImeInlineMentionEditorHandle>();
    render(
      <ImeInlineMentionEditor
        aria-label={EDITOR_LABEL}
        ref={ref}
        document={{ version: 2, inlines: [{ kind: "text", text: "foo\n" }] }}
        syncKey="message-a"
        onDocumentChange={() => undefined}
      />
    );
    const control = screen.getByRole("textbox", { name: "message" });
    const sentinel = control.querySelector("br[data-composer-sentinel]");
    expect(sentinel).not.toBeNull();
    // The browser may place the caret after the sentinel when the user clicks
    // the empty final line; that point must read as documentLength (4), never
    // past it.
    setSelection(control, control.childNodes.length);
    const caret = control.ownerDocument.getSelection()?.getRangeAt(0);
    expect(caret?.startContainer).toBe(control);
    expect(caret?.startOffset).toBe(control.childNodes.length);
    const collapsed = ref.current?.selection();
    expect(collapsed?.start).toBe(4);
    expect(collapsed?.end).toBe(4);
    // Range ending after the sentinel.
    setSelection(control, 0, control, control.childNodes.length);
    const rangeSel = ref.current?.selection();
    expect(rangeSel?.start).toBe(0);
    expect(rangeSel?.end).toBe(4);
  });
});

  it("keeps the caret when a same-key parent acknowledges equal editor content", () => {
    const ref = createRef<ImeInlineMentionEditorHandle>();
    let latest: ComposerDocument = { version: 2, inlines: [] };
    const onDocumentChange = vi.fn((next: ComposerDocument) => {
      latest = next;
    });
    const renderEditor = (document: ComposerDocument) => (
      <ImeInlineMentionEditor
        aria-label={EDITOR_LABEL}
        ref={ref}
        document={document}
        syncKey="message-a"
        onDocumentChange={onDocumentChange}
      />
    );
    const { rerender } = render(renderEditor(latest));
    const control = screen.getByRole("textbox", { name: EDITOR_LABEL });

    control.focus();
    setSelection(control, 0);
    beforeInput(control, "insertText", "a");
    rerender(renderEditor(latest));
    expect(control.textContent).toBe("a");
    expect(ref.current?.selection()).toEqual({ start: 1, end: 1 });

    // Tauri/Rust acknowledgement deserializes an equal document into a fresh
    // object. Replacing the contentEditable children for that no-op update
    // drops WebKit's caret to the beginning.
    rerender(renderEditor(structuredClone(latest)));
    expect(ref.current?.selection()).toEqual({ start: 1, end: 1 });

    beforeInput(control, "insertText", "b");
    rerender(renderEditor(latest));
    expect(control.textContent).toBe("ab");
    expect(ref.current?.selection()).toEqual({ start: 2, end: 2 });
  });

  it.each([
    ["text", (props: { value: string; syncKey: string }) => (
      <ImeTextField aria-label={fieldLabel} {...props} />
    )],
    ["search", (props: { value: string; syncKey: string }) => (
      <ImeTextField aria-label={fieldLabel} type="search" {...props} />
    )],
    ["textarea", (props: { value: string; syncKey: string }) => (
      <ImeTextArea aria-label={fieldLabel} {...props} />
    )]
  ] as const)("keeps %s DOM value and selection across stale composition rerenders", (_kind, field) => {
    const { rerender } = render(field({ value: "before", syncKey: "field-a" }));
    const control = screen.getByLabelText("field") as
      | HTMLInputElement
      | HTMLTextAreaElement;

    fireEvent.compositionStart(control);
    fireEvent.change(control, { target: { value: "日本語変換中" } });
    control.setSelectionRange(3, 5);
    rerender(field({ value: "stale external", syncKey: "field-a" }));

    expect(control.value).toBe("日本語変換中");
    expect([control.selectionStart, control.selectionEnd]).toEqual([3, 5]);
  });

  it("keeps a dirty local value until an external acknowledgement arrives", () => {
    const { rerender } = render(
      <ImeTextField aria-label={fieldLabel} value="before" syncKey="field-a" />
    );
    const control = screen.getByRole("textbox", { name: "field" }) as HTMLInputElement;

    fireEvent.change(control, { target: { value: "local" } });
    rerender(<ImeTextField aria-label={fieldLabel} value="before" syncKey="field-a" />);
    expect(control.value).toBe("local");

    rerender(<ImeTextField aria-label={fieldLabel} value="local" syncKey="field-a" />);
    rerender(<ImeTextField aria-label={fieldLabel} value="server" syncKey="field-a" />);
    expect(control.value).toBe("server");
  });

  it("forces the next semantic field value when syncKey changes", () => {
    const { rerender } = render(
      <ImeTextField aria-label={fieldLabel} value="before" syncKey="field-a" />
    );
    const control = screen.getByRole("textbox", { name: "field" }) as HTMLInputElement;
    fireEvent.compositionStart(control);
    fireEvent.change(control, { target: { value: "old composition" } });

    rerender(<ImeTextField aria-label={fieldLabel} value="next" syncKey="field-b" />);

    expect(control.value).toBe("next");
  });

  it("keeps secure values DOM-only behind a forwarded ref", () => {
    const ref = createRef<HTMLInputElement>();
    render(<SecureImeTextField ref={ref} aria-label={secretLabel} autoComplete="off" />);
    const control = screen.getByLabelText("secret") as HTMLInputElement;

    fireEvent.input(control, { target: { value: "private value" } });

    expect(ref.current).toBe(control);
    expect(ref.current?.value).toBe("private value");
  });

  it("renders mention entities as non-editable inline atoms", () => {
    render(<ControlledMentionEditor />);

    const mention = screen.getByText("@Alice");
    expect(mention.getAttribute("contenteditable")).toBe("false");
    expect(mention.getAttribute("role")).toBe("link");
    expect(mention.hasAttribute("data-composer-mention")).toBe(true);
    expect(mention.getAttribute("aria-label")).toBe("Mention: Alice");
  });

  it("removes one emoji grapheme with Backspace", () => {
    const emoji = "👩‍🔬";
    render(
      <ControlledMentionEditor
        initial={{ version: 2, inlines: [{ kind: "text", text: `A${emoji}B` }] }}
      />
    );
    const control = screen.getByRole("textbox", { name: "message" });
    const text = control.firstChild?.firstChild;
    if (!text) throw new Error("missing editor text");
    setSelection(text, 1 + emoji.length);

    beforeInput(control, "deleteContentBackward");

    expect(control.textContent).toBe("AB");
  });

  it.each([
    ["Backspace", "deleteContentBackward", 2],
    ["Delete", "deleteContentForward", 1]
  ] as const)("%s removes the whole adjacent mention and its metadata", (_key, inputType, caret) => {
    const onChange = vi.fn();
    render(<ControlledMentionEditor onChange={onChange} />);
    const control = screen.getByRole("textbox", { name: "message" });
    const [before, _mention, after] = Array.from(control.childNodes);
    if (caret === 2) setSelection(after, 0);
    else setSelection(before, 1);

    beforeInput(control, inputType);

    expect(control.textContent).toBe("AB");
    expect(control.querySelector("[data-composer-mention]")).toBeNull();
    expect(onChange.mock.lastCall?.[0].inlines).toEqual([{ kind: "text", text: "AB" }]);
  });

  it("range deletion and cut remove the selected mention atom", () => {
    const clipboard = { setData: vi.fn() };
    render(<ControlledMentionEditor />);
    const control = screen.getByRole("textbox", { name: "message" });
    let [before, _mention, after] = Array.from(control.childNodes);
    setSelection(before, 1, after, 0);

    fireEvent.cut(control, { clipboardData: clipboard });

    expect(clipboard.setData).toHaveBeenCalledWith("text/plain", "@Alice");
    expect(control.textContent).toBe("AB");
    expect(control.querySelector("[data-composer-mention]")).toBeNull();
  });

  it("undo and redo restore mention text and identity together", () => {
    render(<ControlledMentionEditor />);
    const control = screen.getByRole("textbox", { name: "message" });
    const [_before, _mention, after] = Array.from(control.childNodes);
    setSelection(after, 0);
    beforeInput(control, "deleteContentBackward");
    expect(control.querySelector("[data-composer-mention]")).toBeNull();

    beforeInput(control, "historyUndo");
    expect(control.querySelector("[data-composer-mention]")?.textContent).toBe("@Alice");

    beforeInput(control, "historyRedo");
    expect(control.querySelector("[data-composer-mention]")).toBeNull();
  });

  it("ends composition ownership when the logical editor key changes", () => {
    const onDocumentChange = vi.fn();
    const { rerender } = render(
      <ImeInlineMentionEditor
        aria-label={EDITOR_LABEL}
        document={mentionDocument}
        syncKey="message-a"
        onDocumentChange={onDocumentChange}
      />
    );
    const control = screen.getByRole("textbox", { name: "message" });
    fireEvent.compositionStart(control);
    expect(control.dataset.composing).toBe("true");

    rerender(
      <ImeInlineMentionEditor
        aria-label={EDITOR_LABEL}
        document={{ version: 2, inlines: [{ kind: "text", text: "next" }] }}
        syncKey="message-b"
        onDocumentChange={onDocumentChange}
      />
    );

    expect(control.dataset.composing).toBeUndefined();
    expect(control.textContent).toBe("next");
  });

  it("keeps mention identity while composition updates neighboring text", () => {
    const onChange = vi.fn();
    const onInput = vi.fn();
    render(<ControlledMentionEditor onChange={onChange} onInput={onInput} />);
    const control = screen.getByRole("textbox", { name: "message" });
    const before = control.firstChild;
    if (!before) throw new Error("missing text node");

    fireEvent.compositionStart(control);
    before.textContent = "A日";
    fireEvent.input(control, { inputType: "insertCompositionText", isComposing: true });
    before.textContent = "A日本";
    fireEvent.input(control, { inputType: "insertCompositionText", isComposing: true });
    expect(onChange).not.toHaveBeenCalled();
    expect(onInput).not.toHaveBeenCalled();
    fireEvent.compositionEnd(control);

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.lastCall?.[0]).toMatchObject({
      inlines: [
        { kind: "text", text: "A日本" },
        { kind: "mention", target: { user_id: "@alice:example.invalid" } },
        { kind: "text", text: "B" }
      ]
    });
    beforeInput(control, "historyUndo");
    expect(control.textContent).toBe("A@AliceB");
  });

  it("suppresses IME-confirmation submit without preventing the native key default", () => {
    vi.useFakeTimers();
    const onSubmit = vi.fn((event: React.FormEvent<HTMLFormElement>) => event.preventDefault());
    const onKeyDown = vi.fn();
    render(
      <ImeSafeForm aria-label={formLabel} onSubmit={onSubmit}>
        <ImeTextField aria-label={fieldLabel} onKeyDown={onKeyDown} />
        <button type="submit">{submitLabel}</button>
      </ImeSafeForm>
    );
    const form = screen.getByRole("form", { name: "form" });
    const control = screen.getByRole("textbox", { name: "field" });

    fireEvent.compositionStart(control);
    const imeEnter = createEvent.keyDown(control, {
      key: "Enter",
      code: "Enter",
      keyCode: 229,
      isComposing: true
    });
    fireEvent(control, imeEnter);
    fireEvent.submit(form);

    expect(imeEnter.defaultPrevented).toBe(false);
    expect(onKeyDown).not.toHaveBeenCalled();
    expect(onSubmit).not.toHaveBeenCalled();

    fireEvent.compositionEnd(control);
    vi.runAllTimers();
    fireEvent.keyDown(control, { key: "Enter", code: "Enter", keyCode: 13 });
    fireEvent.submit(form);
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});

// Issue #1010: WebKit spelling corrections carry the word in `dataTransfer`
// with `data === null`, aimed at `getTargetRanges()`. macOS sends them as
// `insertReplacementText`; WebKitGTK 2.52's context menu sends `insertText`.
describe("spelling replacement (#1010)", () => {
  const misspelled: ComposerDocument = {
    version: 2,
    inlines: [{ kind: "text", text: "Check regulaly today." }]
  };

  function textNodeContaining(control: HTMLElement, needle: string): Text {
    const walker = document.createTreeWalker(control, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.textContent?.includes(needle)) return node as Text;
    }
    throw new Error(`no text node contains ${needle}`);
  }

  function replaceText(
    control: HTMLElement,
    {
      data = null,
      inputType = "insertReplacementText",
      transfer,
      target
    }: {
      data?: string | null;
      inputType?: string;
      transfer?: string;
      target?: { node: Node; start: number; end: number };
    }
  ): InputEvent {
    const event = new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      inputType,
      data
    });
    if (transfer !== undefined) {
      Object.defineProperty(event, "dataTransfer", {
        value: { getData: (type: string) => (type === "text/plain" ? transfer : "") }
      });
    }
    if (target) {
      const range = document.createRange();
      range.setStart(target.node, target.start);
      range.setEnd(target.node, target.end);
      Object.defineProperty(event, "getTargetRanges", { value: () => [range] });
    }
    fireEvent(control, event);
    return event;
  }

  function renderMisspelled(initial = misspelled) {
    const onChange = vi.fn();
    const ref = createRef<ImeInlineMentionEditorHandle>();
    function Harness() {
      const [document, setDocument] = useState(initial);
      return (
        <ImeInlineMentionEditor
          aria-label={EDITOR_LABEL}
          ref={ref}
          document={document}
          syncKey="message-a"
          onDocumentChange={(next) => {
            setDocument(next);
            onChange(next);
          }}
        />
      );
    }
    render(<Harness />);
    const control = screen.getByRole("textbox", { name: EDITOR_LABEL }) as HTMLDivElement;
    control.focus();
    return { control, onChange, ref };
  }

  it("replaces the selected word with a correction carried in dataTransfer", () => {
    const { control, ref } = renderMisspelled();
    setInlineMentionEditorSelection(control, 6, 14);

    const event = replaceText(control, { transfer: "regularly" });

    expect(event.defaultPrevented).toBe(true);
    expect(control.textContent).toBe("Check regularly today.");
    expect(ref.current?.selection()).toEqual({ start: 15, end: 15 });
  });

  it("applies a WebKitGTK context-menu correction sent as insertText", () => {
    // Shape recorded from WebKitGTK 2.52.6 choosing "the" for "teh".
    const { control, ref } = renderMisspelled({
      version: 2,
      inlines: [{ kind: "text", text: "Check teh today." }]
    });
    setInlineMentionEditorSelection(control, 6, 9);
    const text = textNodeContaining(control, "teh");

    const event = replaceText(control, {
      inputType: "insertText",
      transfer: "the",
      target: { node: text, start: 6, end: 9 }
    });

    expect(event.defaultPrevented).toBe(true);
    expect(control.textContent).toBe("Check the today.");
    expect(ref.current?.selection()).toEqual({ start: 9, end: 9 });
  });

  it("leaves an insertText without any text to the engine", () => {
    const { control, onChange } = renderMisspelled();
    setInlineMentionEditorSelection(control, 6, 14);

    const event = replaceText(control, { inputType: "insertText" });

    expect(event.defaultPrevented).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
    expect(control.textContent).toBe("Check regulaly today.");
  });

  it("still accepts a correction carried in data", () => {
    const { control } = renderMisspelled();
    setInlineMentionEditorSelection(control, 6, 14);

    replaceText(control, { data: "regularly" });

    expect(control.textContent).toBe("Check regularly today.");
  });

  it("replaces the target range rather than the selection and keeps a later caret in place", () => {
    const { control, ref } = renderMisspelled();
    // macOS autocorrect: the caret has moved past the word it corrects.
    setInlineMentionEditorSelection(control, 21);
    const text = textNodeContaining(control, "regulaly");

    replaceText(control, { transfer: "regularly", target: { node: text, start: 6, end: 14 } });

    expect(control.textContent).toBe("Check regularly today.");
    expect(ref.current?.selection()).toEqual({ start: 22, end: 22 });
  });

  it("leaves a replacement without a payload to the engine instead of deleting the word", () => {
    const { control, onChange } = renderMisspelled();
    setInlineMentionEditorSelection(control, 6, 14);

    const event = replaceText(control, { transfer: "" });

    expect(event.defaultPrevented).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
    expect(control.textContent).toBe("Check regulaly today.");
  });

  it("ignores a target range outside the editor and falls back to the selection", () => {
    const { control } = renderMisspelled();
    setInlineMentionEditorSelection(control, 6, 14);
    const outside = document.createTextNode("elsewhere");
    document.body.append(outside);

    replaceText(control, { transfer: "regularly", target: { node: outside, start: 0, end: 4 } });

    expect(control.textContent).toBe("Check regularly today.");
    outside.remove();
  });

  it("corrects a word beside a mention without touching the mention", () => {
    const { control, onChange } = renderMisspelled({
      version: 2,
      inlines: [
        {
          kind: "mention",
          target: { kind: "user", user_id: "@alice:example.invalid", display_label: "Alice" },
          display_label: "Alice"
        },
        { kind: "text", text: " teh" }
      ]
    });
    const text = textNodeContaining(control, "teh");

    replaceText(control, { transfer: "the", target: { node: text, start: 1, end: 4 } });

    const published = onChange.mock.lastCall?.[0] as ComposerDocument;
    expect(published.inlines).toEqual([
      expect.objectContaining({ kind: "mention", display_label: "Alice" }),
      { kind: "text", text: " the" }
    ]);
  });

  it("undoes a correction back to the misspelled word", () => {
    const { control } = renderMisspelled();
    setInlineMentionEditorSelection(control, 6, 14);
    replaceText(control, { transfer: "regularly" });

    beforeInput(control, "historyUndo");

    expect(control.textContent).toBe("Check regulaly today.");
  });
});
