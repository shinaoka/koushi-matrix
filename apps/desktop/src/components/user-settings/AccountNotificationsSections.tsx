import { type FormEvent, useEffect, useState } from "react";
import { AtSign, ExternalLink, Mail, MessageSquare, RefreshCcw, UserPlus, Users } from "lucide-react";

import { t, type MessageId } from "../../i18n/messages";
import { ImeSafeForm, ImeTextField } from "../ImeTextControl";
import { AccountManagementUiaForm } from "./AccountManagementUiaForm";
import type {
  AccountNotificationsFailureKind,
  AccountNotificationsOperation,
  AccountNotificationsState,
  NotificationCategory,
  NotificationCategoryState,
  NotificationCategoryStates
} from "../../domain/types";

/**
 * Typed commands for the account-level notification settings (#981).
 * Every value shown here comes from the Rust snapshot; these callbacks only
 * dispatch user intent.
 */
export interface AccountNotificationActions {
  load: () => void;
  setCategory: (category: NotificationCategory, enabled: boolean) => void;
  setAccountPush: (enabled: boolean) => void;
  requestEmailToken: (address: string) => void;
  resendEmailToken: () => void;
  confirmEmail: () => void;
  submitUia: (flowId: number, password: string) => void;
  cancelEmail: () => void;
  enableEmail: (address: string) => void;
  disableEmail: () => void;
}

export const noopAccountNotificationActions: AccountNotificationActions = {
  load: () => undefined,
  setCategory: () => undefined,
  setAccountPush: () => undefined,
  requestEmailToken: () => undefined,
  resendEmailToken: () => undefined,
  confirmEmail: () => undefined,
  submitUia: () => undefined,
  cancelEmail: () => undefined,
  enableEmail: () => undefined,
  disableEmail: () => undefined
};

const categoryRows: {
  category: NotificationCategory;
  field: keyof NotificationCategoryStates;
  label: MessageId;
  icon: typeof Users;
}[] = [
  {
    category: "directMessages",
    field: "direct_messages",
    label: "settings.notificationCategoryDirectMessages",
    icon: MessageSquare
  },
  {
    category: "groupMessages",
    field: "group_messages",
    label: "settings.notificationCategoryGroupMessages",
    icon: Users
  },
  {
    category: "mentionsAndReplies",
    field: "mentions_and_replies",
    label: "settings.notificationCategoryMentions",
    icon: AtSign
  },
  {
    category: "invites",
    field: "invites",
    label: "settings.notificationCategoryInvites",
    icon: UserPlus
  }
];

const failureMessages: Record<AccountNotificationsFailureKind, MessageId> = {
  unsupported: "settings.notificationFailureUnsupported",
  emailInUse: "settings.notificationFailureEmailInUse",
  emailDenied: "settings.notificationFailureEmailDenied",
  invalidEmail: "settings.notificationFailureInvalidEmail",
  emailNotVerified: "settings.notificationFailureEmailNotVerified",
  emailNotRegistered: "settings.notificationFailureEmailNotRegistered",
  authRejected: "settings.notificationFailureAuthRejected",
  forbidden: "settings.notificationFailureForbidden",
  rateLimited: "settings.notificationFailureRateLimited",
  network: "settings.notificationFailureNetwork",
  server: "settings.notificationFailureServer",
  sessionRequired: "settings.notificationFailureServer"
};

function isEmailOperation(operation: AccountNotificationsOperation): boolean {
  return operation.kind !== "setCategory" && operation.kind !== "setAccountPush";
}

function isBusy(state: AccountNotificationsState): boolean {
  return state.operation.kind === "working" || state.operation.kind === "awaitingUia";
}

/** Load progress/failure for the account notification sections, shown once. */
export function AccountNotificationsLoadStatus({
  state,
  onRetry
}: {
  state: AccountNotificationsState;
  onRetry: () => void;
}) {
  if (state.load.kind === "failed") {
    return (
      <div className="session-actions" data-testid="account-notifications-load-failed">
        <p className="settings-status-text">
          {t(failureMessages[state.load.failureKind])}
        </p>
        <button className="trust-action-button secondary" type="button" onClick={onRetry}>
          <RefreshCcw size={14} aria-hidden="true" />
          <span>{t("settings.notificationRetry")}</span>
        </button>
      </div>
    );
  }
  if (!state.snapshot) {
    return <p className="settings-status-text">{t("settings.notificationLoading")}</p>;
  }
  return null;
}

/** Shared ON/OFF rules for app and email notifications (standard push rules). */
export function NotificationCategoriesSection({
  state,
  actions
}: {
  state: AccountNotificationsState;
  actions: AccountNotificationActions;
}) {
  const snapshot = state.snapshot;
  const busy = isBusy(state);
  const failedCategory =
    state.operation.kind === "failed" && !isEmailOperation(state.operation.operation)
      ? state.operation.failureKind
      : null;
  return (
    <section
      className="settings-section"
      aria-label={t("settings.notificationCategories")}
      data-testid="notification-categories"
    >
      <div className="settings-section-heading">
        <div>
          <h3>{t("settings.notificationCategories")}</h3>
          <p>{t("settings.notificationCategoriesDescription")}</p>
        </div>
      </div>
      {snapshot && !snapshot.account_push_enabled ? (
        <div className="session-actions" data-testid="account-push-disabled">
          <p className="settings-status-text">{t("settings.notificationAccountMuted")}</p>
          <button
            className="trust-action-button secondary"
            type="button"
            disabled={busy}
            onClick={() => actions.setAccountPush(true)}
          >
            {t("settings.notificationAccountUnmute")}
          </button>
        </div>
      ) : null}
      {snapshot ? (
        <div className="settings-toggle-list">
          {categoryRows.map((row) => (
            <CategoryToggle
              key={row.category}
              label={t(row.label)}
              icon={row.icon}
              value={snapshot.categories[row.field]}
              disabled={busy}
              pending={
                state.operation.kind === "working" &&
                state.operation.operation.kind === "setCategory" &&
                state.operation.operation.category === row.category
              }
              onToggle={(enabled) => actions.setCategory(row.category, enabled)}
            />
          ))}
        </div>
      ) : null}
      {snapshot && snapshot.categories.group_messages !== "on" ? (
        <p className="settings-status-text" data-testid="encrypted-group-caveat">
          {snapshot.encrypted_event_push
            ? t("settings.notificationEncryptedEventPushCaveat")
            : t("settings.notificationEncryptedMentionsCaveat")}
        </p>
      ) : null}
      {failedCategory ? (
        <p className="settings-status-text" data-testid="notification-category-error">
          {t(failureMessages[failedCategory])}
        </p>
      ) : null}
    </section>
  );
}

function CategoryToggle({
  label,
  icon: Icon,
  value,
  disabled,
  pending,
  onToggle
}: {
  label: string;
  icon: typeof Users;
  value: NotificationCategoryState;
  disabled: boolean;
  pending: boolean;
  onToggle: (enabled: boolean) => void;
}) {
  // A mixed category (set differently by another client) is shown as not
  // fully ON; toggling it applies ON to every rule of the category.
  const checked = value === "on";
  const unavailable = value === "unavailable";
  return (
    <button
      className="settings-toggle-row"
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      data-state={value}
      disabled={disabled || unavailable}
      onClick={() => onToggle(!checked)}
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <Icon size={15} aria-hidden="true" />
          <span>{label}</span>
        </span>
        {value === "mixed" ? (
          <span className="settings-toggle-description">
            {t("settings.notificationCategoryMixed")}
          </span>
        ) : null}
        {unavailable ? (
          <span className="settings-toggle-description">
            {t("settings.notificationCategoryUnavailable")}
          </span>
        ) : null}
        {pending ? (
          <span className="settings-toggle-description">{t("settings.saving")}</span>
        ) : null}
      </span>
      <span className="settings-switch-track" aria-hidden="true">
        <span className="settings-switch-thumb" />
      </span>
    </button>
  );
}

/** Email address registration/verification and the email pusher switch. */
export function EmailNotificationsSection({
  state,
  actions,
  syncKey,
  accountManagementAvailable,
  onManageAccount
}: {
  state: AccountNotificationsState;
  actions: AccountNotificationActions;
  syncKey: string;
  accountManagementAvailable: boolean;
  onManageAccount: () => void;
}) {
  const snapshot = state.snapshot;
  const busy = isBusy(state);
  const verified = snapshot?.emails ?? [];
  const active = verified.filter((email) => email.notifications_active);
  const emailOn = Boolean(
    snapshot && (active.length > 0 || snapshot.unverified_email_pusher_count > 0)
  );
  const [selectedAddress, setSelectedAddress] = useState<string | null>(null);
  const [showAddForm, setShowAddForm] = useState(false);
  const [draft, setDraft] = useState("");

  // The selection is a presentation choice among Rust-provided verified
  // addresses; it falls back to the active target, then the first address.
  const target =
    verified.find((email) => email.address === selectedAddress)?.address ??
    active[0]?.address ??
    verified[0]?.address ??
    null;

  const pending = state.pending_email;
  useEffect(() => {
    if (pending) {
      setShowAddForm(false);
      setDraft("");
    }
  }, [pending?.address]);

  const emailFailure =
    state.operation.kind === "failed" && isEmailOperation(state.operation.operation)
      ? state.operation.failureKind
      : null;
  const awaitingUia =
    state.operation.kind === "awaitingUia" && state.operation.operation.kind === "confirmEmail"
      ? state.operation
      : null;

  function submitAddress(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const address = draft.trim();
    if (!address || busy) {
      return;
    }
    actions.requestEmailToken(address);
  }

  const management = snapshot?.email_management ?? "available";

  return (
    <section
      className="settings-section"
      aria-label={t("settings.emailNotifications")}
      data-testid="email-notifications"
    >
      <div className="settings-section-heading">
        <div>
          <h3>{t("settings.emailNotifications")}</h3>
          <p>{t("settings.emailNotificationsDescription")}</p>
        </div>
      </div>
      {snapshot ? (
        <div className="settings-toggle-list">
          <button
            className="settings-toggle-row"
            type="button"
            role="switch"
            aria-checked={emailOn}
            aria-label={t("settings.emailNotificationsToggle")}
            data-testid="email-notifications-switch"
            disabled={busy || (!emailOn && !target)}
            onClick={() => {
              if (emailOn) {
                actions.disableEmail();
              } else if (target) {
                actions.enableEmail(target);
              }
            }}
          >
            <span className="settings-toggle-copy">
              <span className="settings-toggle-label">
                <Mail size={15} aria-hidden="true" />
                <span>{t("settings.emailNotificationsToggle")}</span>
              </span>
              <span className="settings-toggle-description">
                {emailOn
                  ? active.length > 0 && verified.length > 1
                    ? t("settings.emailNotificationsOn")
                    : active.length > 0
                    ? t("settings.emailNotificationsSendingTo", {
                        address: active.map((email) => email.address).join(", ")
                      })
                    : t("settings.emailNotificationsUnverifiedTarget")
                  : target
                    ? t("settings.emailNotificationsOff")
                    : t("settings.emailNotificationsNeedsEmail")}
              </span>
            </span>
            <span className="settings-switch-track" aria-hidden="true">
              <span className="settings-switch-thumb" />
            </span>
          </button>
        </div>
      ) : null}

      {snapshot && snapshot.unverified_email_pusher_count > 0 ? (
        <p className="settings-status-text" data-testid="email-unverified-pushers">
          {t("settings.emailNotificationsUnverifiedPushers", {
            count: snapshot.unverified_email_pusher_count
          })}
        </p>
      ) : null}

      {snapshot && verified.length > 0 ? (
        <div className="settings-detail-list" data-testid="notification-email-list">
          {verified.map((email) => (
            <div className="settings-detail-row" key={email.address}>
              <span>{email.address}</span>
              <span>{t("settings.emailVerified")}</span>
            </div>
          ))}
        </div>
      ) : null}

      {snapshot && verified.length > 1 ? (
        <label className="profile-settings-field">
          <span>{t("settings.emailNotificationsTarget")}</span>
          <select
            value={target ?? ""}
            disabled={busy}
            data-testid="email-notifications-target"
            onChange={(event) => {
              const address = event.currentTarget.value;
              setSelectedAddress(address);
              // While ON, choosing another verified address moves the single
              // target (Rust adds the new pusher, then removes the old one).
              if (emailOn && address) {
                actions.enableEmail(address);
              }
            }}
          >
            {verified.map((email) => (
              <option key={email.address} value={email.address}>
                {email.address}
              </option>
            ))}
          </select>
        </label>
      ) : null}

      {snapshot && management === "delegatedToAccountManagement" ? (
        <div className="manage-account-row" data-testid="email-managed-externally">
          <p className="profile-settings-hint">{t("settings.emailManagedByAccount")}</p>
          {accountManagementAvailable ? (
            <button className="trust-action-button" type="button" onClick={onManageAccount}>
              <ExternalLink size={14} aria-hidden="true" />
              <span>{t("settings.manageAccount")}</span>
            </button>
          ) : null}
        </div>
      ) : null}

      {snapshot && management === "unsupported" ? (
        <p className="settings-status-text" data-testid="email-unsupported">
          {t("settings.emailUnsupported")}
        </p>
      ) : null}

      {snapshot && management === "available" && pending ? (
        <div className="settings-form" data-testid="email-pending">
          <p className="settings-status-text">
            {t("settings.emailPending", { address: pending.address })}
          </p>
          {pending.resend_count > 0 ? (
            <p className="settings-status-text">{t("settings.emailResent")}</p>
          ) : null}
          {awaitingUia ? (
            <AccountManagementUiaForm flowId={awaitingUia.flow_id} onSubmit={actions.submitUia} />
          ) : (
            <div className="session-actions">
              <button
                className="trust-action-button secondary"
                type="button"
                disabled={busy}
                onClick={actions.cancelEmail}
              >
                {t("action.cancel")}
              </button>
              <button
                className="trust-action-button secondary"
                type="button"
                disabled={busy}
                onClick={actions.resendEmailToken}
                data-testid="email-resend"
              >
                {t("settings.emailResend")}
              </button>
              <button
                className="trust-action-button primary"
                type="button"
                disabled={busy}
                onClick={actions.confirmEmail}
                data-testid="email-confirm"
              >
                {t("settings.emailConfirm")}
              </button>
            </div>
          )}
        </div>
      ) : null}

      {snapshot && management === "available" && !pending && !showAddForm ? (
        <div className="session-actions">
          <button
            className="trust-action-button secondary"
            type="button"
            disabled={busy}
            onClick={() => setShowAddForm(true)}
            data-testid="email-add"
          >
            {verified.length > 0 ? t("settings.emailChange") : t("settings.emailAdd")}
          </button>
        </div>
      ) : null}

      {snapshot && management === "available" && !pending && showAddForm ? (
        <ImeSafeForm className="profile-settings-form" onSubmit={submitAddress}>
          <label className="profile-settings-field">
            <span>{t("settings.emailAddressLabel")}</span>
            <ImeTextField
              type="email"
              autoComplete="email"
              value={draft}
              syncKey={`${syncKey}:notification-email`}
              disabled={busy}
              onChange={(event) => setDraft(event.currentTarget.value)}
              data-testid="email-address-input"
            />
          </label>
          <p className="profile-settings-hint">{t("settings.emailAddHint")}</p>
          <div className="session-actions">
            <button
              className="trust-action-button secondary"
              type="button"
              onClick={() => {
                setShowAddForm(false);
                setDraft("");
              }}
            >
              {t("action.cancel")}
            </button>
            <button
              className="trust-action-button primary"
              type="submit"
              disabled={busy || draft.trim().length === 0}
              data-testid="email-send-verification"
            >
              {t("settings.emailSendVerification")}
            </button>
          </div>
        </ImeSafeForm>
      ) : null}

      {state.operation.kind === "working" && isEmailOperation(state.operation.operation) ? (
        <p className="settings-status-text">{t("settings.saving")}</p>
      ) : null}
      {emailFailure ? (
        <p className="settings-status-text" data-testid="email-notifications-error">
          {t(failureMessages[emailFailure])}
        </p>
      ) : null}
    </section>
  );
}
