import {
  createContext,
  forwardRef,
  useCallback,
  useContext,
  useEffect,
  useId,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  type ChangeEventHandler,
  type ClipboardEventHandler,
  type CompositionEventHandler,
  type FormHTMLAttributes,
  type ForwardedRef,
  type HTMLAttributes,
  type InputHTMLAttributes,
  type KeyboardEventHandler,
  type Ref,
  type TextareaHTMLAttributes
} from "react";

import {
  isComposerImeEnter,
  useCompositionOwnedTextControl,
  type CompositionLifecycle,
  type TextControlElement
} from "../domain/compositionLifecycle";
import {
  commitDocument,
  copyDocumentRange,
  createDocumentHistory,
  deleteDocumentBackward,
  deleteDocumentForward,
  documentLength,
  normalizeDocument,
  pasteDocumentText,
  redoDocument,
  undoDocument,
  type DocumentHistory,
  type DocumentMutation,
  type DocumentSelection
} from "../domain/composerDocument";
import type { ComposerDocument, ComposerInline } from "../domain/types";
import { t } from "../i18n/messages";

interface ImeSubmitFence {
  consume(): boolean;
  mark(): void;
}

const ImeSubmitFenceContext = createContext<ImeSubmitFence | null>(null);

export type ImeSafeFormProps = FormHTMLAttributes<HTMLFormElement>;

export function ImeSafeForm({ children, onSubmit, ...props }: ImeSafeFormProps) {
  const pendingRef = useRef(false);
  const clearTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fence = useMemo<ImeSubmitFence>(
    () => ({
      consume() {
        if (!pendingRef.current) {
          return false;
        }
        pendingRef.current = false;
        if (clearTimerRef.current !== null) {
          clearTimeout(clearTimerRef.current);
          clearTimerRef.current = null;
        }
        return true;
      },
      mark() {
        pendingRef.current = true;
        if (clearTimerRef.current !== null) {
          clearTimeout(clearTimerRef.current);
        }
        clearTimerRef.current = setTimeout(() => {
          pendingRef.current = false;
          clearTimerRef.current = null;
        }, 0);
      }
    }),
    []
  );

  useEffect(
    () => () => {
      if (clearTimerRef.current !== null) {
        clearTimeout(clearTimerRef.current);
      }
    },
    []
  );

  const handleSubmit: NonNullable<FormHTMLAttributes<HTMLFormElement>["onSubmit"]> = (event) => {
    if (fence.consume()) {
      event.preventDefault();
      return;
    }
    onSubmit?.(event);
  };

  return (
    <ImeSubmitFenceContext.Provider value={fence}>
      <form {...props} onSubmit={handleSubmit}>
        {children}
      </form>
    </ImeSubmitFenceContext.Provider>
  );
}

interface ImeControlOwnership<T extends TextControlElement> {
  controlRef: { current: T | null };
  lifecycle: CompositionLifecycle;
  onCompositionEnd(): void;
  onCompositionStart(): number;
  recordLocalValue(value: string): void;
}

interface ImeControlHandlers<T extends TextControlElement> {
  onChange?: ChangeEventHandler<T>;
  onCompositionEnd?: CompositionEventHandler<T>;
  onCompositionStart?: CompositionEventHandler<T>;
  onKeyDown?: KeyboardEventHandler<T>;
}

function assignRef<T>(ref: Ref<T> | undefined, value: T | null) {
  if (typeof ref === "function") {
    ref(value);
  } else if (ref) {
    ref.current = value;
  }
}

function useImeControlBindings<T extends TextControlElement>(
  ownership: ImeControlOwnership<T>,
  forwardedRef: ForwardedRef<T>,
  handlers: ImeControlHandlers<T>
) {
  const submitFence = useContext(ImeSubmitFenceContext);
  const bindRef = useCallback(
    (node: T | null) => {
      ownership.controlRef.current = node;
      assignRef(forwardedRef, node);
    },
    [forwardedRef, ownership.controlRef]
  );
  const onChange: ChangeEventHandler<T> = (event) => {
    ownership.recordLocalValue(event.currentTarget.value);
    handlers.onChange?.(event);
  };
  const onCompositionStart: CompositionEventHandler<T> = (event) => {
    ownership.onCompositionStart();
    handlers.onCompositionStart?.(event);
  };
  const onCompositionEnd: CompositionEventHandler<T> = (event) => {
    ownership.recordLocalValue(event.currentTarget.value);
    ownership.onCompositionEnd();
    handlers.onCompositionEnd?.(event);
  };
  const onKeyDown: KeyboardEventHandler<T> = (event) => {
    if (
      isComposerImeEnter(event.key, {
        epochActive: ownership.lifecycle.active(),
        nativeIsComposing: event.nativeEvent.isComposing,
        keyCode: event.keyCode
      })
    ) {
      submitFence?.mark();
      return;
    }
    handlers.onKeyDown?.(event);
  };

  return { bindRef, onChange, onCompositionEnd, onCompositionStart, onKeyDown };
}

type ImeInputType = "email" | "search" | "tel" | "text" | "url";

export interface ImeTextFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "defaultValue" | "type" | "value"> {
  syncKey?: string;
  type?: ImeInputType;
  value?: string;
}

export const ImeTextField = forwardRef<HTMLInputElement, ImeTextFieldProps>(
  function ImeTextField(
    {
      onChange,
      onCompositionEnd,
      onCompositionStart,
      onKeyDown,
      syncKey,
      type = "text",
      value,
      ...props
    },
    forwardedRef
  ) {
    const generatedKey = useId();
    const ownership = useCompositionOwnedTextControl<HTMLInputElement>(
      value,
      syncKey ?? generatedKey
    );
    const bindings = useImeControlBindings(ownership, forwardedRef, {
      onChange,
      onCompositionEnd,
      onCompositionStart,
      onKeyDown
    });
    return (
      <input
        {...props}
        ref={bindings.bindRef}
        type={type}
        defaultValue={value}
        onChange={bindings.onChange}
        onCompositionStart={bindings.onCompositionStart}
        onCompositionEnd={bindings.onCompositionEnd}
        onKeyDown={bindings.onKeyDown}
      />
    );
  }
);

export interface SecureImeTextFieldProps
  extends Omit<
    InputHTMLAttributes<HTMLInputElement>,
    "defaultValue" | "type" | "value"
  > {
  syncKey?: string;
}

export const SecureImeTextField = forwardRef<HTMLInputElement, SecureImeTextFieldProps>(
  function SecureImeTextField(
    { onChange, onCompositionEnd, onCompositionStart, onKeyDown, syncKey, ...props },
    forwardedRef
  ) {
    const generatedKey = useId();
    const ownership = useCompositionOwnedTextControl<HTMLInputElement>(
      undefined,
      syncKey ?? generatedKey
    );
    const bindings = useImeControlBindings(ownership, forwardedRef, {
      onChange,
      onCompositionEnd,
      onCompositionStart,
      onKeyDown
    });
    return (
      <input
        {...props}
        ref={bindings.bindRef}
        type="password"
        onChange={bindings.onChange}
        onCompositionStart={bindings.onCompositionStart}
        onCompositionEnd={bindings.onCompositionEnd}
        onKeyDown={bindings.onKeyDown}
      />
    );
  }
);

export interface ImeInlineMentionEditorProps
  extends Omit<HTMLAttributes<HTMLDivElement>, "children" | "contentEditable" | "onChange"> {
  document: ComposerDocument;
  editable?: boolean;
  onDocumentChange(document: ComposerDocument): void;
  onSelectionChange?(selection: DocumentSelection): void;
  syncKey: string;
}

export interface ImeInlineMentionEditorHandle {
  commit(mutation: DocumentMutation): void;
  focus(): void;
  isComposing(): boolean;
  selection(): DocumentSelection;
  setSelection(selection: DocumentSelection): void;
}

export const ImeInlineMentionEditor = forwardRef<
  ImeInlineMentionEditorHandle,
  ImeInlineMentionEditorProps
>(
  function ImeInlineMentionEditor(
    {
      document,
      editable = true,
      onDocumentChange,
      onCompositionEnd,
      onCompositionStart,
      onInput,
      onKeyDown,
      onPaste,
      onSelectionChange,
      syncKey,
      ...props
    },
    forwardedRef
  ) {
    const controlRef = useRef<HTMLDivElement | null>(null);
    const documentRef = useRef(document);
    const historyRef = useRef<DocumentHistory>(createDocumentHistory(document));
    const historyKeyRef = useRef(syncKey);
    const composingRef = useRef(false);
    const pendingSelectionRef = useRef<DocumentSelection | null>(null);
    const renderedKeyRef = useRef<string | null>(null);
    const submitFence = useContext(ImeSubmitFenceContext);

    if (historyKeyRef.current !== syncKey) {
      historyKeyRef.current = syncKey;
      historyRef.current = createDocumentHistory(document);
    } else if (!composingRef.current && !documentsEqual(historyRef.current.present, document)) {
      historyRef.current = createDocumentHistory(document);
    }
    documentRef.current = document;

    const bindRef = useCallback(
      (node: HTMLDivElement | null) => {
        controlRef.current = node;
      },
      []
    );

    useLayoutEffect(() => {
      const control = controlRef.current;
      if (!control) return;
      const keyChanged = renderedKeyRef.current !== syncKey;
      if (keyChanged) {
        composingRef.current = false;
        delete control.dataset.composing;
      }
      if (keyChanged || !composingRef.current) {
        renderEditorDocument(control, document);
        renderedKeyRef.current = syncKey;
      }
      const selection = keyChanged
        ? { start: documentLength(document), end: documentLength(document) }
        : pendingSelectionRef.current;
      if (!selection || composingRef.current) return;
      pendingSelectionRef.current = null;
      restoreDocumentSelection(control, selection);
    }, [document, syncKey]);

    const publish = useCallback(
      (mutation: DocumentMutation) => {
        const changed = !documentsEqual(documentRef.current, mutation.document);
        historyRef.current = commitDocument(historyRef.current, mutation.document);
        documentRef.current = mutation.document;
        pendingSelectionRef.current = mutation.selection;
        onSelectionChange?.(mutation.selection);
        if (changed) onDocumentChange(mutation.document);
      },
      [onDocumentChange, onSelectionChange]
    );

    const publishHistory = useCallback(
      (history: DocumentHistory) => {
        if (history === historyRef.current) return;
        historyRef.current = history;
        documentRef.current = history.present;
        const caret = documentLength(history.present);
        pendingSelectionRef.current = { start: caret, end: caret };
        onSelectionChange?.({ start: caret, end: caret });
        onDocumentChange(history.present);
      },
      [onDocumentChange, onSelectionChange]
    );

    const selection = useCallback(() => {
      const control = controlRef.current;
      return control ? documentSelectionFromDom(control) : { start: 0, end: 0 };
    }, []);

    useImperativeHandle(
      forwardedRef,
      () => ({
        commit: publish,
        focus: () => controlRef.current?.focus(),
        isComposing: () => composingRef.current,
        selection,
        setSelection: (nextSelection) => {
          const control = controlRef.current;
          if (control) restoreDocumentSelection(control, nextSelection);
        }
      }),
      [publish, selection]
    );

    const publishDom = useCallback(
      (commitHistory = true) => {
        const control = controlRef.current;
        if (!control) return;
        const next = documentFromEditorDom(control, documentRef.current);
        const nextSelection = documentSelectionFromDom(control);
        if (commitHistory) {
          publish({ document: next, selection: nextSelection });
          return;
        }
        documentRef.current = next;
        onSelectionChange?.(nextSelection);
        onDocumentChange(next);
      },
      [onDocumentChange, onSelectionChange, publish]
    );

    const handleBeforeInput = useCallback(
      (event: InputEvent) => {
        if (composingRef.current || event.isComposing) return;
        const range = selection();
        let mutation: DocumentMutation | null = null;
        switch (event.inputType) {
          case "deleteContentBackward":
            mutation = deleteDocumentBackward(documentRef.current, range.start, range.end);
            break;
          case "deleteContentForward":
            mutation = deleteDocumentForward(documentRef.current, range.start, range.end);
            break;
          case "insertText":
          case "insertReplacementText":
            mutation = pasteDocumentText(
              documentRef.current,
              range.start,
              range.end,
              event.data ?? ""
            );
            break;
          case "insertLineBreak":
          case "insertParagraph":
            mutation = pasteDocumentText(documentRef.current, range.start, range.end, "\n");
            break;
          case "historyUndo":
            event.preventDefault();
            publishHistory(undoDocument(historyRef.current));
            return;
          case "historyRedo":
            event.preventDefault();
            publishHistory(redoDocument(historyRef.current));
            return;
          default:
            if (range.start !== range.end && event.inputType.startsWith("delete")) {
              mutation = deleteDocumentBackward(documentRef.current, range.start, range.end);
            }
        }
        if (!mutation) return;
        event.preventDefault();
        publish(mutation);
      },
      [publish, publishHistory, selection]
    );

    useEffect(() => {
      const control = controlRef.current;
      if (!control) return;
      control.addEventListener("beforeinput", handleBeforeInput);
      return () => control.removeEventListener("beforeinput", handleBeforeInput);
    }, [handleBeforeInput]);

    const handleCopy: ClipboardEventHandler<HTMLDivElement> = (event) => {
      const range = selection();
      if (range.start === range.end) return;
      event.clipboardData.setData(
        "text/plain",
        copyDocumentRange(documentRef.current, range.start, range.end)
      );
      event.preventDefault();
    };
    const handleCut: ClipboardEventHandler<HTMLDivElement> = (event) => {
      const range = selection();
      if (range.start === range.end) return;
      event.clipboardData.setData(
        "text/plain",
        copyDocumentRange(documentRef.current, range.start, range.end)
      );
      event.preventDefault();
      publish(deleteDocumentBackward(documentRef.current, range.start, range.end));
    };
    const handlePaste: ClipboardEventHandler<HTMLDivElement> = (event) => {
      onPaste?.(event);
      if (event.defaultPrevented) return;
      const range = selection();
      event.preventDefault();
      publish(
        pasteDocumentText(
          documentRef.current,
          range.start,
          range.end,
          event.clipboardData.getData("text/plain")
        )
      );
    };

    return (
      <div
        {...props}
        ref={bindRef}
        role="textbox"
        aria-multiline="true"
        aria-disabled={!editable || undefined}
        contentEditable={editable}
        suppressContentEditableWarning
        onCompositionStart={(event) => {
          composingRef.current = true;
          event.currentTarget.dataset.composing = "true";
          onCompositionStart?.(event);
        }}
        onCompositionEnd={(event) => {
          composingRef.current = false;
          delete event.currentTarget.dataset.composing;
          publishDom();
          onCompositionEnd?.(event);
        }}
        onCopy={handleCopy}
        onCut={handleCut}
        onInput={(event) => {
          if (composingRef.current || event.nativeEvent.isComposing) return;
          publishDom();
          onInput?.(event);
        }}
        onKeyDown={(event) => {
          if (
            isComposerImeEnter(event.key, {
              epochActive: composingRef.current,
              nativeIsComposing: event.nativeEvent.isComposing,
              keyCode: event.keyCode
            })
          ) {
            submitFence?.mark();
            return;
          }
          onKeyDown?.(event);
        }}
        onPaste={handlePaste}
        onSelect={() => onSelectionChange?.(selection())}
        onKeyUp={() => onSelectionChange?.(selection())}
        onMouseUp={() => onSelectionChange?.(selection())}
      />
    );
  }
);

function renderEditorDocument(control: HTMLDivElement, document: ComposerDocument) {
  const nodes = document.inlines.map((inline, index) => {
    const span = control.ownerDocument.createElement("span");
    if (inline.kind === "text") {
      span.dataset.composerText = "";
      span.textContent = inline.text;
    } else {
      span.className = "composer-inline-mention";
      span.setAttribute("contenteditable", "false");
      span.setAttribute("role", "link");
      span.dataset.composerMention = String(index);
      span.setAttribute("aria-label", t("composer.inlineMention", { label: inline.display_label }));
      span.textContent = `@${inline.display_label}`;
    }
    return span;
  });
  // Issue #471: under `white-space: pre-wrap` a trailing newline as the last
  // character of the block creates no final line box — the composer neither
  // grows nor paints the caret on the new line. Append a sentinel <br> that
  // the DOM readers ignore; it never counts toward document offsets.
  if (documentEndsWithNewline(document)) {
    const sentinel = control.ownerDocument.createElement("br");
    sentinel.dataset.composerSentinel = "";
    nodes.push(sentinel);
  }
  control.replaceChildren(...nodes);
}

function documentEndsWithNewline(document: ComposerDocument): boolean {
  const last = document.inlines.at(-1);
  return last?.kind === "text" && last.text.endsWith("\n");
}

function isSentinelBr(node: Node): boolean {
  return (
    node instanceof HTMLElement &&
    node.tagName === "BR" &&
    node.hasAttribute("data-composer-sentinel")
  );
}

export function inlineMentionEditorSelection(control: HTMLDivElement): DocumentSelection {
  return documentSelectionFromDom(control);
}

export function setInlineMentionEditorSelection(
  control: HTMLDivElement,
  start: number,
  end = start
) {
  restoreDocumentSelection(control, { start, end });
}

function documentSelectionFromDom(control: HTMLDivElement): DocumentSelection {
  const selection = control.ownerDocument.getSelection();
  if (!selection || selection.rangeCount === 0) return { start: 0, end: 0 };
  const range = selection.getRangeAt(0);
  if (!control.contains(range.startContainer) || !control.contains(range.endContainer)) {
    const end = documentLength(documentFromEditorDom(control, { version: 2, inlines: [] }));
    return { start: end, end };
  }
  const start = documentOffsetFromDomPoint(control, range.startContainer, range.startOffset);
  const end = documentOffsetFromDomPoint(control, range.endContainer, range.endOffset);
  return { start: Math.min(start, end), end: Math.max(start, end) };
}

function documentOffsetFromDomPoint(
  control: HTMLDivElement,
  container: Node,
  offset: number
): number {
  if (container === control) {
    // Issue #471: the trailing newline lives inside the last text span, so
    // the sentinel <br> contributes zero and the after-sentinel point maps
    // to the same document offset as the end of that span (the empty final
    // line has no document width). No special case is needed.
    return Array.from(control.childNodes)
      .slice(0, offset)
      .reduce((total, child) => total + editorNodeLength(child), 0);
  }
  const child = Array.from(control.childNodes).find(
    (candidate) => candidate === container || candidate.contains?.(container)
  );
  if (!child) return 0;
  const before = Array.from(control.childNodes)
    .slice(0, Array.from(control.childNodes).indexOf(child))
    .reduce((total, node) => total + editorNodeLength(node), 0);
  if (child instanceof HTMLElement && child.hasAttribute("data-composer-mention")) {
    return before + (offset > 0 ? 1 : 0);
  }
  const textLength = child.textContent?.length ?? 0;
  if (container.nodeType === Node.TEXT_NODE) return before + Math.min(textLength, offset);
  return before + (offset > 0 ? textLength : 0);
}

function editorNodeLength(node: Node): number {
  return node instanceof HTMLElement && node.hasAttribute("data-composer-mention")
    ? 1
    : (node.textContent?.length ?? 0);
}

function documentFromEditorDom(
  control: HTMLDivElement,
  current: ComposerDocument
): ComposerDocument {
  const inlines: ComposerInline[] = [];
  for (const node of control.childNodes) {
    if (isSentinelBr(node)) {
      // Issue #471: the trailing-newline sentinel is presentation-only and
      // never becomes document content.
      continue;
    }
    if (node instanceof HTMLElement && node.hasAttribute("data-composer-mention")) {
      const index = Number(node.dataset.composerMention);
      const mention = current.inlines[index];
      if (mention?.kind === "mention") inlines.push(mention);
      continue;
    }
    const text = node.textContent ?? "";
    if (text) inlines.push({ kind: "text", text });
  }
  return normalizeDocument({ version: 2, inlines });
}

function restoreDocumentSelection(control: HTMLDivElement, selection: DocumentSelection) {
  const start = domPointFromDocumentOffset(control, selection.start);
  const end = domPointFromDocumentOffset(control, selection.end);
  const range = control.ownerDocument.createRange();
  range.setStart(start.node, start.offset);
  range.setEnd(end.node, end.offset);
  const domSelection = control.ownerDocument.getSelection();
  domSelection?.removeAllRanges();
  domSelection?.addRange(range);
}

function domPointFromDocumentOffset(control: HTMLDivElement, rawOffset: number) {
  let remaining = Math.max(0, rawOffset);
  for (const child of control.childNodes) {
    const length = editorNodeLength(child);
    if (remaining <= length) {
      if (child instanceof HTMLElement && child.hasAttribute("data-composer-mention")) {
        const index = Array.from(control.childNodes).indexOf(child);
        return { node: control as Node, offset: index + (remaining === 0 ? 0 : 1) };
      }
      const text = child.firstChild;
      if (text?.nodeType === Node.TEXT_NODE) {
        return { node: text, offset: Math.min(remaining, text.textContent?.length ?? 0) };
      }
      return { node: child, offset: 0 };
    }
    remaining -= length;
  }
  return { node: control as Node, offset: control.childNodes.length };
}

function documentsEqual(left: ComposerDocument, right: ComposerDocument) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export interface ImeTextAreaProps
  extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "defaultValue" | "value"> {
  syncKey?: string;
  value?: string;
}

export const ImeTextArea = forwardRef<HTMLTextAreaElement, ImeTextAreaProps>(
  function ImeTextArea(
    {
      onChange,
      onCompositionEnd,
      onCompositionStart,
      onKeyDown,
      syncKey,
      value,
      ...props
    },
    forwardedRef
  ) {
    const generatedKey = useId();
    const ownership = useCompositionOwnedTextControl<HTMLTextAreaElement>(
      value,
      syncKey ?? generatedKey
    );
    return (
      <ImeOwnedTextArea
        {...props}
        ref={forwardedRef}
        ownership={ownership}
        value={value}
        onChange={onChange}
        onCompositionStart={onCompositionStart}
        onCompositionEnd={onCompositionEnd}
        onKeyDown={onKeyDown}
      />
    );
  }
);

export interface ImeOwnedTextAreaProps
  extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "defaultValue" | "value"> {
  ownership: ImeControlOwnership<HTMLTextAreaElement>;
  value?: string;
}

export const ImeOwnedTextArea = forwardRef<HTMLTextAreaElement, ImeOwnedTextAreaProps>(
  function ImeOwnedTextArea(
    {
      onChange,
      onCompositionEnd,
      onCompositionStart,
      onKeyDown,
      ownership,
      value,
      ...props
    },
    forwardedRef
  ) {
    const bindings = useImeControlBindings(ownership, forwardedRef, {
      onChange,
      onCompositionEnd,
      onCompositionStart,
      onKeyDown
    });
    return (
      <textarea
        {...props}
        ref={bindings.bindRef}
        defaultValue={value}
        onChange={bindings.onChange}
        onCompositionStart={bindings.onCompositionStart}
        onCompositionEnd={bindings.onCompositionEnd}
        onKeyDown={bindings.onKeyDown}
      />
    );
  }
);
