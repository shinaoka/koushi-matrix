import { useId } from "react";
import { t } from "../i18n/messages";

/**
 * The explicit confirmation for replacing a recovery key (#1997). The Secure
 * Backup gate and Settings start the same `resetRecoveryKey` intent, so they
 * share one warning: the previous recovery key and security phrase stop working
 * on the account and in every other Matrix client.
 */
export function RecoveryKeyResetConfirmation({
  onConfirm,
  onCancel,
  disabled = false
}: {
  onConfirm: () => void;
  onCancel: () => void;
  disabled?: boolean;
}) {
  const titleId = useId();
  return (
    <div className="trust-verification-dialog" role="region" aria-labelledby={titleId}>
      <h2 id={titleId}>{t("gate.secureBackupResetRecoveryKeyTitle")}</h2>
      <p>{t("gate.secureBackupResetRecoveryKeyWarning")}</p>
      <div className="dialog-actions">
        <button
          className="dialog-button danger"
          disabled={disabled}
          type="button"
          onClick={onConfirm}
        >
          {t("gate.secureBackupResetRecoveryKeyConfirm")}
        </button>
        <button className="dialog-button" type="button" onClick={onCancel}>
          {t("action.cancel")}
        </button>
      </div>
    </div>
  );
}
