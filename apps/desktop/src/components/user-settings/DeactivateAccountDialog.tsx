import { type FormEvent, useEffect, useRef, useState } from "react";

import { t } from "../../i18n/messages";
import { ImeSafeForm, SecureImeTextField } from "../ImeTextControl";
import { ModalDialog } from "../ModalDialog";
import type { AccountManagementState } from "../../domain/types";

/**
 * Issue #1503: Element X-style account deactivation. The dialog names the
 * target account, lists the consequences, requires the password up front,
 * and asks for a final confirmation whose destructive button sits where the
 * first step's Continue button is not, so a double-click cannot pass both.
 */
export function DeactivateAccountDialog({
  userId,
  accountManagement,
  onCancel,
  onDeactivate
}: {
  userId: string;
  accountManagement: AccountManagementState;
  onCancel: () => void;
  onDeactivate: (eraseData: boolean, password: string) => void;
}) {
  const passwordRef = useRef<HTMLInputElement>(null);
  const pendingPassword = useRef("");
  const [step, setStep] = useState<"form" | "confirm">("form");
  const [eraseData, setEraseData] = useState(false);
  const [hasPassword, setHasPassword] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  // Set from the confirm click until the request fails, so the dialog cannot
  // be dismissed or confirmed twice before the core projects `working`.
  const [dispatching, setDispatching] = useState(false);

  const isDeactivation =
    accountManagement.kind !== "idle" && accountManagement.operation === "deactivateAccount";
  const working = submitted && isDeactivation && accountManagement.kind === "working";
  const failed = submitted && isDeactivation && accountManagement.kind === "failed";
  const failure = failed ? accountManagement.failureKind : null;
  const failedRequestId = failed ? accountManagement.request_id : null;

  // Each failed attempt returns to the password step with an empty field.
  useEffect(() => {
    if (failedRequestId !== null) {
      setStep("form");
      setHasPassword(false);
      setDispatching(false);
    }
  }, [failedRequestId]);

  function continueToConfirm(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const password = passwordRef.current?.value ?? "";
    if (!password) {
      return;
    }
    pendingPassword.current = password;
    setStep("confirm");
  }

  function cancel() {
    pendingPassword.current = "";
    onCancel();
  }

  function confirmDeactivation() {
    if (dispatching) {
      return;
    }
    const password = pendingPassword.current;
    pendingPassword.current = "";
    if (!password) {
      setStep("form");
      return;
    }
    setSubmitted(true);
    setDispatching(true);
    onDeactivate(eraseData, password);
  }

  const locked = working || dispatching;

  if (step === "confirm" || locked) {
    return (
      <ModalDialog
        title={t("settings.deactivateAccountFinalConfirmTitle", { userId })}
        className="confirmation-modal"
        showCloseButton={false}
        dismissible={!locked}
        onClose={cancel}
      >
        <div className="confirmation-content" data-testid="deactivate-account-final-confirm">
          <p>{t("settings.deactivateAccountFinalConfirm")}</p>
          {working ? (
            <p className="settings-status-text" role="status">
              {t("settings.deactivateAccountWorking")}
            </p>
          ) : null}
          {/* Destructive action first, Cancel last: the step-one Continue
              button position now holds Cancel. */}
          <div className="dialog-actions">
            <button
              type="button"
              className="dialog-button danger"
              disabled={locked}
              onClick={confirmDeactivation}
              data-testid="deactivate-account-confirm"
            >
              {t("settings.deactivateAccountFinalAction")}
            </button>
            <button
              type="button"
              className="dialog-button"
              disabled={locked}
              onClick={cancel}
              autoFocus
              data-testid="deactivate-account-confirm-cancel"
            >
              {t("action.cancel")}
            </button>
          </div>
        </div>
      </ModalDialog>
    );
  }

  return (
    <ModalDialog
      title={t("settings.deactivateAccount")}
      className="confirmation-modal deactivate-account-modal"
      showCloseButton={false}
      onClose={cancel}
    >
      <ImeSafeForm
        className="confirmation-content"
        onSubmit={continueToConfirm}
        data-testid="deactivate-account-dialog"
      >
        <div className="deactivate-account-target">
          <span>{t("settings.deactivateAccountTarget")}</span>
          <strong data-testid="deactivate-account-target">{userId}</strong>
        </div>
        <p>{t("settings.deactivateAccountIntro")}</p>
        <ul className="deactivate-account-consequences">
          <li>{t("settings.deactivateAccountConsequenceDisable")}</li>
          <li>{t("settings.deactivateAccountConsequenceRooms")}</li>
          <li>{t("settings.deactivateAccountConsequenceIdentity")}</li>
          <li>{t("settings.deactivateAccountConsequenceMessages")}</li>
        </ul>
        <label className="settings-detail-row">
          <input
            type="checkbox"
            checked={eraseData}
            onChange={(event) => setEraseData(event.currentTarget.checked)}
            data-testid="deactivate-erase-checkbox"
          />
          <span>{t("settings.deactivateAccountDeleteMessages")}</span>
        </label>
        {eraseData ? (
          <p className="profile-settings-hint">{t("settings.deactivateAccountDeleteMessagesNotice")}</p>
        ) : null}
        <label className="profile-settings-field">
          <span>{t("settings.deactivateAccountPassword")}</span>
          <SecureImeTextField
            ref={passwordRef}
            autoComplete="current-password"
            autoFocus
            onInput={() => setHasPassword(Boolean(passwordRef.current?.value))}
            data-testid="deactivate-account-password"
          />
        </label>
        {failure ? (
          <p className="settings-status-text" role="alert" data-testid="account-management-error">
            {failure === "forbidden"
              ? t("settings.deactivateAccountWrongPassword")
              : t("settings.accountManagementFailed")}
          </p>
        ) : null}
        <div className="dialog-actions">
          <button type="button" className="dialog-button" onClick={cancel}>
            {t("action.cancel")}
          </button>
          <button
            type="submit"
            className="dialog-button"
            disabled={!hasPassword}
            data-testid="deactivate-account-continue"
          >
            {t("action.continue")}
          </button>
        </div>
      </ImeSafeForm>
    </ModalDialog>
  );
}
