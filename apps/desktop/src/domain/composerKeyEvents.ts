import type { ComposerKeyEvent, ComposerSelection } from "./types";

export interface ComposerKeyboardEventFacts {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  nativeEvent?: {
    isComposing?: boolean;
  };
}

export interface TextInsertionResult {
  value: string;
  cursor: number;
}

export function shouldResolveComposerKeyEvent(event: ComposerKeyboardEventFacts): boolean {
  return event.key === "Enter" || event.key === "Escape";
}

export function shouldLetNativeImeHandleComposerKeyEvent(event: ComposerKeyEvent): boolean {
  return event.is_composing;
}

export function composerKeyEventFromDom(
  event: ComposerKeyboardEventFacts,
  selection: ComposerSelection | null = null
): ComposerKeyEvent {
  return {
    key: event.key === "Enter" ? "enter" : event.key === "Escape" ? "escape" : "other",
    modifiers: {
      ctrl: event.ctrlKey,
      meta: event.metaKey,
      shift: event.shiftKey,
      alt: event.altKey
    },
    is_composing: Boolean(event.nativeEvent?.isComposing),
    selection
  };
}

export function insertNewlineAtSelection(
  value: string,
  selectionStart: number,
  selectionEnd: number
): TextInsertionResult {
  const start = Math.max(0, Math.min(selectionStart, value.length));
  const end = Math.max(start, Math.min(selectionEnd, value.length));
  return {
    value: `${value.slice(0, start)}\n${value.slice(end)}`,
    cursor: start + 1
  };
}
