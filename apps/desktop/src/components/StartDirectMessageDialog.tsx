import { t } from "../i18n/messages";
import { ModalDialog } from "./ModalDialog";

/**
 * #1702: asks before a person-targeted start creates a direct chat. Core has
 * already reported that none exists; only "Start chat" asks it to create one.
 * Cancel holds the default focus, so Enter or a stray double click never
 * creates a room.
 */
export function StartDirectMessageDialog({
  name,
  userId,
  isBusy,
  onCancel,
  onConfirm
}: {
  name: string;
  userId: string;
  isBusy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <ModalDialog
      title={t("room.startDirectMessageConfirmTitle")}
      className="confirmation-modal"
      showCloseButton={false}
      dismissible={!isBusy}
      onClose={onCancel}
    >
      <div className="confirmation-content">
        <p dir="auto">{t("room.startDirectMessageConfirmBody", { name, userId })}</p>
        <div className="dialog-actions">
          <button
            type="button"
            className="dialog-button"
            disabled={isBusy}
            autoFocus
            onClick={onCancel}
          >
            {t("action.cancel")}
          </button>
          <button
            type="button"
            className="dialog-button is-primary"
            disabled={isBusy}
            aria-busy={isBusy || undefined}
            onClick={onConfirm}
          >
            {t("room.startDirectMessageConfirmAction")}
          </button>
        </div>
      </div>
    </ModalDialog>
  );
}
