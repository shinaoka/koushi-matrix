import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { t } from "../i18n/messages";

/** Browser top layer: escapes pane clipping and supplies modal focus containment. */
export function ModalDialog({ title, className = "", dismissible = true, showCloseButton = true, onClose, children }: {
  title: string;
  className?: string;
  dismissible?: boolean;
  showCloseButton?: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current!;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.showModal();
    return () => {
      dialog.close();
      if (previous?.isConnected) previous.focus();
    };
  }, []);
  return (
    <dialog ref={ref} className={`app-modal ${className}`} aria-label={title} aria-modal="true"
      onCancel={(event) => {
        if (event.target !== event.currentTarget) return;
        event.preventDefault();
        if (dismissible) onClose();
      }}
      onKeyDown={(event) => {
        if (event.key !== "Tab" || event.defaultPrevented) return;
        const dialog = event.currentTarget;
        // Native dialogs make the shell inert; keep Tab wrapping in the app
        // as well, instead of letting focus move to browser chrome.
        const focusable = Array.from(dialog.querySelectorAll<HTMLElement>(
          'button, input, select, textarea, a[href], [tabindex]'
        )).filter(element => element.tabIndex >= 0 && !element.matches(":disabled") && element.getClientRects().length > 0);
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault(); last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault(); first?.focus();
        }
      }}>
      <header className="app-modal-header">
        <h2>{title}</h2>
        {showCloseButton ? <button type="button" className="icon-button" aria-label={t("action.close", { title })} disabled={!dismissible} onClick={onClose}>
          <X size={20} aria-hidden="true" />
        </button> : null}
      </header>
      {children}
    </dialog>
  );
}
