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

export type MacEmacsAction =
  | "moveForward"
  | "moveBackward"
  | "moveUp"
  | "moveDown"
  | "killToEol"
  | "yank";

export interface MacEmacsEffect {
  /** Replacement textarea value; absent when only selection changed. */
  newValue?: string;
  /** Cursor position to apply (via requestAnimationFrame when newValue is present). */
  newSelectionPos: number;
  /** Updated kill-ring content; absent when unchanged. */
  newKillRing?: string;
}

// Evaluated once at module load; stable for the lifetime of the WebView.
export const IS_MAC_PLATFORM: boolean =
  typeof navigator !== "undefined" &&
  /mac/i.test(
    (
      (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData
        ?.platform ?? navigator.platform
    )
  );

/**
 * Returns the Emacs-style text-editing action for a Ctrl+key event on macOS,
 * or null if the event is not a macOS Emacs binding.
 * Must only be called when IS_MAC_PLATFORM is true and the event is not IME-composing.
 */
export function macEmacsActionFromEvent(
  event: ComposerKeyboardEventFacts
): MacEmacsAction | null {
  if (!event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) {
    return null;
  }
  switch (event.key) {
    case "f":
      return "moveForward";
    case "b":
      return "moveBackward";
    case "p":
      return "moveUp";
    case "n":
      return "moveDown";
    case "k":
      return "killToEol";
    case "y":
      return "yank";
    default:
      return null;
  }
}

/**
 * Pure function: computes the effect of a macOS Emacs text-editing action given
 * the current textarea state.  Returns null when there is nothing to do
 * (e.g. yank with an empty kill ring, or kill at the very end of the string).
 */
export function applyMacEmacsAction(
  action: MacEmacsAction,
  value: string,
  selectionStart: number,
  selectionEnd: number,
  killRing: string
): MacEmacsEffect | null {
  switch (action) {
    case "moveForward": {
      return { newSelectionPos: Math.min(selectionStart + 1, value.length) };
    }
    case "moveBackward": {
      return { newSelectionPos: Math.max(selectionStart - 1, 0) };
    }
    case "moveUp": {
      const lineStart = value.lastIndexOf("\n", selectionStart - 1) + 1;
      const col = selectionStart - lineStart;
      const prevLineEnd = lineStart - 1;
      if (prevLineEnd < 0) {
        return { newSelectionPos: 0 };
      }
      const prevLineStart = value.lastIndexOf("\n", prevLineEnd - 1) + 1;
      return { newSelectionPos: Math.min(prevLineStart + col, prevLineEnd) };
    }
    case "moveDown": {
      const lineStart = value.lastIndexOf("\n", selectionStart - 1) + 1;
      const col = selectionStart - lineStart;
      const nextNl = value.indexOf("\n", selectionStart);
      if (nextNl === -1) {
        return { newSelectionPos: value.length };
      }
      const nextLineStart = nextNl + 1;
      const nextNl2 = value.indexOf("\n", nextLineStart);
      const nextLineEnd = nextNl2 === -1 ? value.length : nextNl2;
      return { newSelectionPos: Math.min(nextLineStart + col, nextLineEnd) };
    }
    case "killToEol": {
      const eolIdx = value.indexOf("\n", selectionStart);
      const atEol =
        eolIdx === selectionStart || (eolIdx === -1 && selectionStart === value.length);
      if (atEol) {
        if (eolIdx === -1) {
          // Already at end of string — nothing to kill.
          return null;
        }
        // Kill the newline character itself.
        return {
          newValue: value.slice(0, selectionStart) + value.slice(selectionStart + 1),
          newSelectionPos: selectionStart,
          newKillRing: "\n"
        };
      }
      const endPos = eolIdx === -1 ? value.length : eolIdx;
      return {
        newValue: value.slice(0, selectionStart) + value.slice(endPos),
        newSelectionPos: selectionStart,
        newKillRing: value.slice(selectionStart, endPos)
      };
    }
    case "yank": {
      if (!killRing) {
        return null;
      }
      const newValue =
        value.slice(0, selectionStart) + killRing + value.slice(selectionEnd);
      return {
        newValue,
        newSelectionPos: selectionStart + killRing.length
      };
    }
  }
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
