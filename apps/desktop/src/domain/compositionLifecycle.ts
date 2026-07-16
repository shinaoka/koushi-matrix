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

export type TextControlElement = HTMLInputElement | HTMLTextAreaElement;

export type CompositionOwnedValueDecision =
  | { kind: "acknowledged" }
  | { kind: "ignoreComposition" }
  | { kind: "ignoreStale" }
  | { kind: "uncontrolled" }
  | { kind: "write"; value: string };

export interface CompositionOwnedValueState {
  currentKey(): string;
  recordLocalValue(value: string): void;
  observeExternal(
    externalValue: string | undefined,
    syncKey: string,
    compositionActive: boolean
  ): CompositionOwnedValueDecision;
}

export function createCompositionOwnedValueState(
  initialExternalValue: string | undefined,
  initialKey: string
): CompositionOwnedValueState {
  let key = initialKey;
  let localValue = initialExternalValue;
  let dirty = false;

  return {
    currentKey: () => key,
    recordLocalValue(value) {
      localValue = value;
      dirty = true;
    },
    observeExternal(externalValue, syncKey, compositionActive) {
      if (syncKey !== key) {
        key = syncKey;
        localValue = externalValue;
        dirty = false;
        return externalValue === undefined
          ? { kind: "uncontrolled" }
          : { kind: "write", value: externalValue };
      }
      if (externalValue === undefined) {
        return { kind: "uncontrolled" };
      }
      if (dirty && externalValue === localValue) {
        dirty = false;
        return { kind: "acknowledged" };
      }
      if (compositionActive) {
        return { kind: "ignoreComposition" };
      }
      if (dirty) {
        return { kind: "ignoreStale" };
      }
      localValue = externalValue;
      return { kind: "write", value: externalValue };
    }
  };
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

export function useCompositionOwnedTextControl<T extends TextControlElement>(
  externalValue: string | undefined,
  syncKey: string
) {
  const controlRef = useRef<T>(null);
  const lifecycleRef = useRef<CompositionLifecycle | null>(null);
  const valueStateRef = useRef<CompositionOwnedValueState | null>(null);
  if (lifecycleRef.current === null) {
    lifecycleRef.current = createCompositionLifecycle();
  }
  if (valueStateRef.current === null) {
    valueStateRef.current = createCompositionOwnedValueState(externalValue, syncKey);
  }
  const lifecycle = lifecycleRef.current;
  const valueState = valueStateRef.current;

  useLayoutEffect(() => {
    const keyChanged = valueState.currentKey() !== syncKey;
    if (keyChanged) {
      lifecycle.finish();
    }
    const decision = valueState.observeExternal(externalValue, syncKey, lifecycle.active());
    const control = controlRef.current;
    if (control && decision.kind === "write" && control.value !== decision.value) {
      control.value = decision.value;
    }
  }, [externalValue, lifecycle, syncKey, valueState]);

  useEffect(() => () => lifecycle.dispose(), [lifecycle]);

  return {
    controlRef,
    lifecycle,
    recordLocalValue: (value: string) => valueState.recordLocalValue(value),
    onCompositionStart: () => lifecycle.start(),
    onCompositionEnd: () => lifecycle.end()
  };
}

export function useCompositionOwnedTextarea(externalValue: string, syncKey: string) {
  const control = useCompositionOwnedTextControl<HTMLTextAreaElement>(externalValue, syncKey);
  return {
    ...control,
    textareaRef: control.controlRef
  };
}
