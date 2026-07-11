import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import type { ComposerResolvedAction } from "./types";

export interface CompositionLifecycle {
  start(): number;
  end(epoch?: number): void;
  finish(): void;
  active(): boolean;
  generation(): number;
  dispose(): void;
}

export function createCompositionLifecycle(): CompositionLifecycle {
  let epoch = 0;
  let isActive = false;
  let deferredEnd: ReturnType<typeof setTimeout> | null = null;

  const cancelDeferredEnd = () => {
    if (deferredEnd !== null) {
      clearTimeout(deferredEnd);
      deferredEnd = null;
    }
  };

  return {
    start() {
      cancelDeferredEnd();
      epoch += 1;
      isActive = true;
      return epoch;
    },
    end(capturedEpoch = epoch) {
      cancelDeferredEnd();
      deferredEnd = setTimeout(() => {
        deferredEnd = null;
        if (capturedEpoch === epoch) {
          isActive = false;
        }
      }, 0);
    },
    finish() {
      cancelDeferredEnd();
      epoch += 1;
      isActive = false;
    },
    active() {
      return isActive;
    },
    generation() {
      return epoch;
    },
    dispose() {
      cancelDeferredEnd();
      epoch += 1;
      isActive = false;
    }
  };
}

export interface ComposerKeyIntentSnapshot {
  readonly value: string;
  readonly selectionStart: number;
  readonly selectionEnd: number;
  readonly intentGeneration: number;
  releaseResolution(): void;
  isValidForResult(): boolean;
  isCurrentForMutation(): boolean;
}

export function canApplyResolvedComposerAction(
  intent: ComposerKeyIntentSnapshot,
  action: ComposerResolvedAction
): boolean {
  if (action === "insertNewline" || action === "acceptAutocomplete") {
    return intent.isCurrentForMutation();
  }
  return intent.isValidForResult();
}

export function useComposerKeyIntentSnapshot(lifecycle: CompositionLifecycle) {
  const intentGenerationRef = useRef(0);
  const resolveInFlightRef = useRef<number | null>(null);

  return useCallback((textarea: HTMLTextAreaElement): ComposerKeyIntentSnapshot | null => {
    if (resolveInFlightRef.current !== null) {
      return null;
    }
    intentGenerationRef.current += 1;
    const intentGeneration = intentGenerationRef.current;
    resolveInFlightRef.current = intentGeneration;
    const compositionGeneration = lifecycle.generation();
    const value = textarea.value;
    const selectionStart = textarea.selectionStart;
    const selectionEnd = textarea.selectionEnd;
    const isValidForResult = () =>
      intentGenerationRef.current === intentGeneration &&
      lifecycle.generation() === compositionGeneration;
    return {
      value,
      selectionStart,
      selectionEnd,
      intentGeneration,
      releaseResolution: () => {
        if (resolveInFlightRef.current === intentGeneration) {
          resolveInFlightRef.current = null;
        }
      },
      isValidForResult,
      isCurrentForMutation: () =>
        isValidForResult() &&
        textarea.value === value &&
        textarea.selectionStart === selectionStart &&
        textarea.selectionEnd === selectionEnd
    };
  }, [lifecycle]);
}

export function isComposerImeEnter(
  key: string,
  signals: {
    epochActive: boolean;
    nativeIsComposing: boolean;
    keyCode: number;
  }
): boolean {
  return (
    key === "Enter" &&
    (signals.epochActive || signals.nativeIsComposing || signals.keyCode === 229)
  );
}

export function useCompositionOwnedTextarea(externalValue: string, syncKey: string) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const lifecycleRef = useRef<CompositionLifecycle | null>(null);
  const previousKeyRef = useRef(syncKey);
  if (lifecycleRef.current === null) {
    lifecycleRef.current = createCompositionLifecycle();
  }
  const lifecycle = lifecycleRef.current;

  useLayoutEffect(() => {
    const keyChanged = previousKeyRef.current !== syncKey;
    previousKeyRef.current = syncKey;
    if (keyChanged) {
      lifecycle.finish();
    } else if (lifecycle.active()) {
      return;
    }
    const textarea = textareaRef.current;
    if (textarea && textarea.value !== externalValue) {
      textarea.value = externalValue;
    }
  }, [externalValue, lifecycle, syncKey]);

  useEffect(() => () => lifecycle.dispose(), [lifecycle]);

  return {
    textareaRef,
    lifecycle,
    onCompositionStart: () => lifecycle.start(),
    onCompositionEnd: () => lifecycle.end()
  };
}
