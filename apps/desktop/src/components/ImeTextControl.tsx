import {
  createContext,
  forwardRef,
  useCallback,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  type ChangeEventHandler,
  type CompositionEventHandler,
  type FormHTMLAttributes,
  type ForwardedRef,
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
