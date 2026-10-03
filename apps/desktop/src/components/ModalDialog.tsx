import { createContext, useEffect, useLayoutEffect, useRef, type ComponentPropsWithRef, type ReactNode, type RefObject } from "react";
import { X } from "lucide-react";
import { t } from "../i18n/messages";

export const ModalPortalContext = createContext<RefObject<HTMLDialogElement | null> | null>(null);

let modalFocusOrigin: HTMLElement | null = null;

/** Install once at the application root; all listeners share its DOM lifetime.
 * Safari pointer activation does not focus buttons, so activeElement alone
 * cannot identify the opener. Keyboard/programmatic focus updates the same
 * origin, including menu actions launched while another modal is open. */
export function useModalFocusTracking() {
  useEffect(() => {
    const onPointerDown = (event: Event) => {
      // Menu items disappear when they launch a dialog. Retain the menu's
      // persistent opener rather than returning focus to a detached item.
      if (event.target instanceof Element && event.target.closest('[role="menu"]')) return;
      modalFocusOrigin = event.target instanceof Element
        ? event.target.closest<HTMLElement>('button, a[href], input, select, textarea, [tabindex], [contenteditable="true"]')
        : null;
    };
    const onFocus = (event: FocusEvent) => {
      if (event.target instanceof HTMLElement && event.target !== document.body && !event.target.closest('[role="menu"]')) modalFocusOrigin = event.target;
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    // Safari may focus the containing dialog between pointerdown and click.
    // Capture activation again before React mounts a nested modal.
    document.addEventListener("click", onPointerDown, true);
    document.addEventListener("focusin", onFocus, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("click", onPointerDown, true);
      document.removeEventListener("focusin", onFocus, true);
      modalFocusOrigin = null;
    };
  }, []);
}

/** Shared top-layer/focus owner for both framed modals and full-window viewers. */
export function NativeModal({ onDismiss, dismissible = true, initialFocusRef, returnFocusRef, children, ref: forwardedRef, ...props }:
  Omit<ComponentPropsWithRef<"dialog">, "open"> & {
    onDismiss: () => void;
    dismissible?: boolean;
    initialFocusRef?: RefObject<HTMLElement | null>;
    returnFocusRef?: RefObject<HTMLElement | null>;
  }) {
  const ref = useRef<HTMLDialogElement>(null);
  const previous = useRef((modalFocusOrigin?.isConnected ? modalFocusOrigin : null)
    ?? (typeof document !== "undefined" && document.activeElement instanceof HTMLElement ? document.activeElement : null));
  const composing = useRef(false);
  useLayoutEffect(() => {
    const dialog = ref.current!;
    // React's autoFocus runs during commit, before showModal's focus steps.
    const initial = document.activeElement instanceof HTMLElement && dialog.contains(document.activeElement)
      ? document.activeElement : null;
    dialog.showModal();
    (initialFocusRef?.current ?? initial)?.focus();
    return () => {
      dialog.close();
      const target = returnFocusRef?.current ?? previous.current;
      if (target?.isConnected) target.focus();
    };
  }, [initialFocusRef, returnFocusRef]);
  return <ModalPortalContext.Provider value={ref}><dialog {...props} ref={(node) => {
    ref.current = node;
    if (typeof forwardedRef === "function") forwardedRef(node);
    else if (forwardedRef) forwardedRef.current = node;
  }} aria-modal="true"
    onCompositionStart={() => { composing.current = true; }}
    onCompositionEnd={() => { composing.current = false; }}
    onCancel={(event) => {
      if (event.target !== event.currentTarget) return;
      event.preventDefault();
      event.stopPropagation();
      if (dismissible && !composing.current) onDismiss();
    }}
    onKeyDown={(event) => {
      // React portal events also bubble through parents: only this dialog owns
      // the keystroke, never an underlying modal or a document-level shortcut.
      event.stopPropagation();
      if (event.key === "Escape") {
        if (event.defaultPrevented) return;
        event.preventDefault();
        if (dismissible && !composing.current && !event.nativeEvent.isComposing && event.keyCode !== 229) onDismiss();
        return;
      }
      props.onKeyDown?.(event);
      if (event.key !== "Tab" || event.defaultPrevented || composing.current || event.nativeEvent.isComposing || event.keyCode === 229) return;
      const dialog = event.currentTarget;
      const focusable = Array.from(dialog.querySelectorAll<HTMLElement>(
        'button, input, select, textarea, a[href], [tabindex]'
      )).filter(element => element.tabIndex >= 0 && !element.matches(":disabled") && !element.closest("[inert]") && element.getClientRects().length > 0);
      const first = focusable[0];
      if (!first) {
        if (document.activeElement === dialog) event.preventDefault();
      } else {
        // WebKit's platform Tab preference can skip buttons entirely. Own the
        // complete sequence so every action is reachable on every platform.
        const index = focusable.indexOf(document.activeElement as HTMLElement);
        const next = index < 0 ? (event.shiftKey ? focusable.length - 1 : 0)
          : (index + (event.shiftKey ? -1 : 1) + focusable.length) % focusable.length;
        event.preventDefault(); focusable[next]?.focus();
      }
    }}>{children}</dialog></ModalPortalContext.Provider>;
}

/** Browser top layer: escapes pane clipping and supplies modal focus containment. */
export function ModalDialog({ title, className = "", dismissible = true, showCloseButton = true, onClose, children }: {
  title: string;
  className?: string;
  dismissible?: boolean;
  showCloseButton?: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <NativeModal className={`app-modal ${className}`} aria-label={title} dismissible={dismissible} onDismiss={onClose}>
      <header className="app-modal-header">
        <h2>{title}</h2>
        {showCloseButton ? <button type="button" className="icon-button" aria-label={t("action.close", { title })} disabled={!dismissible} onClick={onClose}>
          <X size={20} aria-hidden="true" />
        </button> : null}
      </header>
      {children}
    </NativeModal>
  );
}
