import { type FormEvent, useEffect, useRef, useState } from "react";
import { ExternalLink, KeyRound, ShieldAlert } from "lucide-react";

import { toExternalHttpUrl } from "../../domain/externalLinks";
import { t } from "../../i18n/messages";
import { ImeSafeForm, SecureImeTextField } from "../ImeTextControl";
import { AccountManagementUiaForm } from "./AccountManagementUiaForm";
import type {
  AccountManagementCapabilities,
  AccountManagementState,
  SavedSessionInfo
} from "../../domain/types";

export function AccountManagementSection({
  accountManagement,
  accountManagementCapabilities,
  accountManagementUrl,
  currentSession,
  onLoadAccountManagementCapabilities,
  onChangePassword,
  onDeactivateAccount,
  onManageAccount,
  onSubmitAccountManagementUia
}: {
  accountManagement: AccountManagementState;
  accountManagementCapabilities: AccountManagementCapabilities;
  accountManagementUrl: string | null;
  currentSession: SavedSessionInfo | null;
  onLoadAccountManagementCapabilities: () => void;
  onChangePassword: (newPassword: string) => void;
  onDeactivateAccount: (eraseData: boolean) => void;
  onManageAccount: () => void;
  onSubmitAccountManagementUia: (flowId: number, password: string) => void;
}) {
  useEffect(() => {
    if (currentSession && accountManagementCapabilities.change_password.kind === "unknown") {
      onLoadAccountManagementCapabilities();
    }
  }, [currentSession, accountManagementCapabilities.change_password.kind, onLoadAccountManagementCapabilities]);

  const safeAccountManagementUrl = toExternalHttpUrl(accountManagementUrl);
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [showDeactivate, setShowDeactivate] = useState(false);
  const newPasswordRef = useRef<HTMLInputElement>(null);
  const confirmPasswordRef = useRef<HTMLInputElement>(null);
  const [passwordFieldsComplete, setPasswordFieldsComplete] = useState(false);
  const [eraseData, setEraseData] = useState(false);
  const [mismatch, setMismatch] = useState(false);

  const activeOperation =
    accountManagement.kind === "working" ||
    accountManagement.kind === "awaitingUia" ||
    accountManagement.kind === "succeeded" ||
    accountManagement.kind === "failed"
      ? accountManagement.operation
      : null;

  const isChangePassword = activeOperation === "changePassword";
  const isDeactivate = activeOperation === "deactivateAccount";

  const changePasswordEnabled =
    accountManagementCapabilities.change_password.kind === "enabled";

  function submitChangePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const newPassword = newPasswordRef.current?.value ?? "";
    const confirmPassword = confirmPasswordRef.current?.value ?? "";
    if (newPassword !== confirmPassword) {
      setMismatch(true);
      return;
    }
    setMismatch(false);
    onChangePassword(newPassword);
  }

  function updatePasswordFieldsComplete() {
    setPasswordFieldsComplete(
      Boolean(newPasswordRef.current?.value && confirmPasswordRef.current?.value)
    );
  }

  function submitDeactivate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onDeactivateAccount(eraseData);
  }

  function resetForms() {
    if (newPasswordRef.current) {
      newPasswordRef.current.value = "";
    }
    if (confirmPasswordRef.current) {
      confirmPasswordRef.current.value = "";
    }
    setShowChangePassword(false);
    setShowDeactivate(false);
    setPasswordFieldsComplete(false);
    setEraseData(false);
    setMismatch(false);
  }

  return (
    <section className="settings-section" aria-label={t("settings.accountManagement")}>
      <div className="settings-section-heading">
        <h3>{t("settings.accountManagement")}</h3>
      </div>

      {safeAccountManagementUrl ? (
        <div className="manage-account-row">
          <button
            className="trust-action-button"
            type="button"
            onClick={onManageAccount}
            data-testid="manage-account-button"
          >
            <ExternalLink size={14} aria-hidden="true" />
            <span>{t("settings.manageAccount")}</span>
          </button>
          <p className="profile-settings-hint">{t("settings.manageAccountHint")}</p>
        </div>
      ) : null}

      {accountManagement.kind === "awaitingUia" && (isChangePassword || isDeactivate) ? (
        <AccountManagementUiaForm
          flowId={accountManagement.flow_id}
          onSubmit={onSubmitAccountManagementUia}
        />
      ) : null}

      {accountManagement.kind === "succeeded" && isChangePassword ? (
        <p className="settings-status-text" data-testid="change-password-success">
          {t("settings.passwordChanged")}
        </p>
      ) : null}

      {accountManagement.kind === "succeeded" && isDeactivate ? (
        <p className="settings-status-text" data-testid="deactivate-success">
          {t("settings.accountDeactivated")}
        </p>
      ) : null}

      {accountManagement.kind === "failed" && (isChangePassword || isDeactivate) ? (
        <p className="settings-status-text" data-testid="account-management-error">
          {t("settings.accountManagementFailed")}
        </p>
      ) : null}

      {!showChangePassword && !showDeactivate ? (
        <div className="session-actions">
          <button
            className="trust-action-button secondary"
            type="button"
            disabled={!changePasswordEnabled || accountManagement.kind === "working"}
            onClick={() => setShowChangePassword(true)}
            data-testid="change-password-button"
          >
            <KeyRound size={14} />
            <span>{t("settings.changePassword")}</span>
          </button>
          <button
            className="trust-action-button danger"
            type="button"
            disabled={accountManagement.kind === "working"}
            onClick={() => setShowDeactivate(true)}
            data-testid="deactivate-account-button"
          >
            <ShieldAlert size={14} />
            <span>{t("settings.deactivateAccount")}</span>
          </button>
        </div>
      ) : null}

      {showChangePassword ? (
        <ImeSafeForm className="profile-settings-form" onSubmit={submitChangePassword}>
          <label className="profile-settings-field">
            <span>{t("settings.changePasswordLabel")}</span>
            <SecureImeTextField
              ref={newPasswordRef}
              autoComplete="new-password"
              onInput={updatePasswordFieldsComplete}
              data-testid="change-password-input"
            />
          </label>
          <label className="profile-settings-field">
            <span>{t("settings.changePasswordConfirm")}</span>
            <SecureImeTextField
              ref={confirmPasswordRef}
              autoComplete="new-password"
              onInput={updatePasswordFieldsComplete}
              data-testid="change-password-confirm-input"
            />
          </label>
          {mismatch ? (
            <p className="settings-status-text">{t("settings.changePasswordMismatch")}</p>
          ) : null}
          <div className="session-actions">
            <button
              className="trust-action-button secondary"
              type="button"
              onClick={() => {
                resetForms();
              }}
            >
              {t("action.cancel")}
            </button>
            <button
              className="trust-action-button primary"
              type="submit"
              disabled={!passwordFieldsComplete || accountManagement.kind === "working"}
              data-testid="change-password-submit"
            >
              {t("settings.changePassword")}
            </button>
          </div>
        </ImeSafeForm>
      ) : null}

      {showDeactivate ? (
        <ImeSafeForm className="settings-form" onSubmit={submitDeactivate}>
          <p className="settings-status-text">{t("settings.deactivateAccountConfirm")}</p>
          <label className="settings-detail-row">
            <input
              type="checkbox"
              checked={eraseData}
              onChange={(event) => setEraseData(event.currentTarget.checked)}
              data-testid="deactivate-erase-checkbox"
            />
            <span>{t("settings.deactivateAccountErase")}</span>
          </label>
          <div className="session-actions">
            <button
              className="trust-action-button secondary"
              type="button"
              onClick={() => {
                resetForms();
              }}
            >
              {t("action.cancel")}
            </button>
            <button
              className="trust-action-button danger"
              type="submit"
              disabled={accountManagement.kind === "working"}
              data-testid="deactivate-account-submit"
            >
              {t("settings.deactivateAccount")}
            </button>
          </div>
        </ImeSafeForm>
      ) : null}
    </section>
  );
}
