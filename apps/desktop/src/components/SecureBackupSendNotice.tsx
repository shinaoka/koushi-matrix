import { t } from "../i18n/messages";

/**
 * #2086: the send-blocked notice is the place the block is, so it carries the
 * way out: re-check Secure Backup now, or open the settings page that shows the
 * same gate state. Rust owns admission; these actions only ask it to look
 * again, and only a confirmed ready inspection opens sending.
 */
export function SecureBackupSendNotice({
  retrying = false,
  retryable = true,
  onRetry,
  onOpenSettings
}: {
  retrying?: boolean;
  /** False when the recorded cause is not one this session can retry. */
  retryable?: boolean;
  onRetry: () => void;
  onOpenSettings: () => void;
}) {
  return (
    <div className="composer-read-only-notice-row">
      {/* `.composer-read-only-notice` is the live region the read-only shell
          specs read as their contract, so it keeps that class and `role` and
          holds only the reason. The actions are its siblings: a status message
          must not contain the buttons it announced. */}
      <div className="composer-read-only-notice" role="status">
        {t("composer.sendBlockedSecureBackup")}
      </div>
      <div className="composer-read-only-notice-actions">
        {retryable ? (
          <button
            type="button"
            className="composer-read-only-notice-action"
            onClick={onRetry}
            disabled={retrying}
          >
            {t("composer.sendBlockedSecureBackupRecheck")}
          </button>
        ) : null}
        <button
          type="button"
          className="composer-read-only-notice-link"
          onClick={onOpenSettings}
        >
          {t("composer.sendBlockedSecureBackupSettings")}
        </button>
      </div>
    </div>
  );
}
