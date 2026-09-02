import { type FormEvent, useRef, useState } from "react";
import {
  Check,
  KeyRound,
  RotateCcw,
  ShieldAlert,
  ShieldCheck,
  ShieldQuestion,
  X
} from "lucide-react";

import { t } from "../../i18n/messages";
import {
  currentSessionStatusDetails,
  currentSessionStatusFactsRemainAuthoritative
} from "../../domain/currentSessionStatus";
import { ImeSafeForm, SecureImeTextField } from "../ImeTextControl";
import {
  TrustActionButton,
  TrustStatusRow,
  type TrustTone,
  failureKindLabel
} from "./SettingsStatusPrimitives";
import type {
  CrossSigningStatus,
  CurrentSessionStatusState,
  E2eeTrustState,
  IdentityResetState,
  KeyBackupStatus,
  VerificationFlowState
} from "../../domain/types";

export function TrustSection({
  trust,
  currentSessionStatus,
  onBootstrapCrossSigning,
  onEnableKeyBackup,
  onAcceptVerification,
  onConfirmSasVerification,
  onCancelVerification,
  onResetIdentity,
  onCancelIdentityReset,
  onSubmitIdentityResetPassword,
  onSubmitIdentityResetOAuth
}: {
  trust: E2eeTrustState;
  currentSessionStatus: CurrentSessionStatusState;
  onBootstrapCrossSigning: () => void;
  onEnableKeyBackup: () => void;
  onAcceptVerification: (flowId: number) => void;
  onConfirmSasVerification: (flowId: number) => void;
  onCancelVerification: (flowId: number) => void;
  onResetIdentity: () => void;
  onCancelIdentityReset: (flowId: number) => void;
  onSubmitIdentityResetPassword: (flowId: number, password: string) => void;
  onSubmitIdentityResetOAuth: (flowId: number) => void;
}) {
  const currentSessionDetails = currentSessionStatusDetails(currentSessionStatus);
  const overall = trustOverallStatus(trust, currentSessionStatus);
  const crossSigningEstablished = currentSessionDetails?.is_cross_signed_by_owner === true;
  const keyBackupEstablished = currentSessionDetails?.key_backup === "ready";

  return (
    <section className="settings-section trust-section" aria-label={t("trust.encryption")}>
      <div className="settings-section-heading">
        <h3>{t("trust.encryption")}</h3>
        <span className={`trust-status-chip ${overall.tone}`}>{overall.label}</span>
      </div>

      <VerificationDialog
        verification={trust.verification}
        onAccept={onAcceptVerification}
        onCancel={onCancelVerification}
        onConfirm={onConfirmSasVerification}
      />

      <div className="trust-status-list">
        <TrustStatusRow
          icon={<ShieldCheck size={16} />}
          label={t("trust.crossSigning")}
          value={
            currentSessionDetails
              ? currentSessionDetails.is_cross_signed_by_owner
                ? t("sessionStatus.crossSigned")
                : t("sessionStatus.notCrossSigned")
              : crossSigningStatusLabel(trust.cross_signing)
          }
          tone={
            currentSessionDetails
              ? currentSessionDetails.is_cross_signed_by_owner
                ? "good"
                : "warning"
              : crossSigningTone(trust.cross_signing)
          }
          action={
            !crossSigningEstablished && crossSigningActionAvailable(trust.cross_signing) ? (
              <TrustActionButton
                icon={<ShieldCheck size={14} />}
                label={t("trust.setupCrossSigning")}
                onClick={onBootstrapCrossSigning}
              />
            ) : null
          }
        />
        <TrustStatusRow
          icon={<KeyRound size={16} />}
          label={t("trust.keyBackup")}
          value={
            currentSessionDetails
              ? currentSessionKeyBackupLabel(currentSessionDetails.key_backup)
              : keyBackupStatusLabel(trust.key_backup)
          }
          tone={
            currentSessionDetails
              ? currentSessionDetails.key_backup === "ready"
                ? "good"
                : currentSessionDetails.key_backup === "disabled"
                  ? "warning"
                  : "neutral"
              : keyBackupTone(trust.key_backup)
          }
          action={
            !keyBackupEstablished && keyBackupActionAvailable(trust.key_backup) ? (
              <TrustActionButton
                icon={<KeyRound size={14} />}
                label={t("trust.enableKeyBackup")}
                onClick={onEnableKeyBackup}
              />
            ) : null
          }
        />
        <TrustStatusRow
          icon={<RotateCcw size={16} />}
          label={t("trust.identityReset")}
          value={identityResetStatusLabel(trust.identity_reset)}
          tone={identityResetTone(trust.identity_reset)}
          action={
            trust.identity_reset.kind === "resetting" ? null : (
              <TrustActionButton
                icon={<RotateCcw size={14} />}
                label={t("trust.resetIdentity")}
                onClick={onResetIdentity}
              />
            )
          }
        />
      </div>

      <IdentityResetAuthControls
        state={trust.identity_reset}
        onCancelIdentityReset={onCancelIdentityReset}
        onSubmitIdentityResetOAuth={onSubmitIdentityResetOAuth}
        onSubmitIdentityResetPassword={onSubmitIdentityResetPassword}
      />
    </section>
  );
}

function VerificationDialog({
  verification,
  onAccept,
  onCancel,
  onConfirm
}: {
  verification: VerificationFlowState;
  onAccept: (flowId: number) => void;
  onCancel: (flowId: number) => void;
  onConfirm: (flowId: number) => void;
}) {
  if (verification.kind === "idle") {
    return null;
  }

  const titleId = `trust-verification-${verification.request_id}`;
  const flowId = verification.request_id;
  const statusLabel = verificationStatusLabel(verification);

  return (
    <article
      className={`trust-verification-dialog ${verification.kind}`}
      role="dialog"
      aria-labelledby={titleId}
    >
      <div className="trust-verification-heading">
        <ShieldQuestion size={17} aria-hidden="true" />
        <div>
          <h4 id={titleId}>{t("trust.verification")}</h4>
          <p>{statusLabel}</p>
        </div>
      </div>

      {verification.kind === "sasPresented" || verification.kind === "confirming" ? (
        <ol className="trust-sas-list" aria-label={t("trust.sasEmojiList")}>
          {verification.emojis.map((emoji, index) => (
            <li
              className="trust-sas-item"
              key={`${emoji.symbol}-${index}`}
              aria-label={t("trust.sasEmoji", { index: index + 1 })}
            >
              {emoji.symbol}
            </li>
          ))}
        </ol>
      ) : null}

      {verification.kind === "requested" ? (
        <div className="trust-dialog-actions">
          <TrustActionButton
            icon={<Check size={14} />}
            label={t("trust.acceptVerification")}
            onClick={() => onAccept(flowId)}
          />
          <TrustActionButton
            icon={<X size={14} />}
            label={t("trust.declineVerification")}
            variant="secondary"
            onClick={() => onCancel(flowId)}
          />
        </div>
      ) : null}

      {verification.kind === "sasPresented" ? (
        <div className="trust-dialog-actions">
          <TrustActionButton
            icon={<Check size={14} />}
            label={t("trust.confirmSas")}
            onClick={() => onConfirm(flowId)}
          />
          <TrustActionButton
            icon={<X size={14} />}
            label={t("trust.declineVerification")}
            variant="secondary"
            onClick={() => onCancel(flowId)}
          />
        </div>
      ) : null}

      {verification.kind === "accepted" ||
      verification.kind === "confirming" ||
      verification.kind === "failed" ? (
        <div className="trust-dialog-actions">
          <TrustActionButton
            icon={<X size={14} />}
            label={t("trust.closeVerification")}
            variant="secondary"
            onClick={() => onCancel(flowId)}
          />
        </div>
      ) : null}
    </article>
  );
}

function IdentityResetAuthControls({
  state,
  onCancelIdentityReset,
  onSubmitIdentityResetPassword,
  onSubmitIdentityResetOAuth
}: {
  state: IdentityResetState;
  onCancelIdentityReset: (flowId: number) => void;
  onSubmitIdentityResetPassword: (flowId: number, password: string) => void;
  onSubmitIdentityResetOAuth: (flowId: number) => void;
}) {
  const passwordInput = useRef<HTMLInputElement>(null);
  const [passwordFilled, setPasswordFilled] = useState(false);

  if (state.kind !== "awaitingAuth") {
    return null;
  }

  const flowId = state.request_id;

  if (state.auth_type === "oauth") {
    return (
      <div className="trust-auth-row">
        <TrustActionButton
          icon={<X size={14} />}
          label={t("trust.cancelIdentityReset")}
          onClick={() => onCancelIdentityReset(flowId)}
        />
        <TrustActionButton
          icon={<Check size={14} />}
          label={t("trust.continueIdentityReset")}
          onClick={() => onSubmitIdentityResetOAuth(flowId)}
        />
      </div>
    );
  }

  if (state.auth_type !== "uiaa") {
    return (
      <div className="trust-auth-row" role="status">
        <ShieldAlert size={15} aria-hidden="true" />
        <span>{t("trust.identityResetAuthUnknown")}</span>
        <TrustActionButton
          icon={<X size={14} />}
          label={t("trust.cancelIdentityReset")}
          onClick={() => onCancelIdentityReset(flowId)}
        />
      </div>
    );
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const password = passwordInput.current?.value ?? "";
    if (!password) {
      return;
    }
    onSubmitIdentityResetPassword(flowId, password);
    if (passwordInput.current) {
      passwordInput.current.value = "";
    }
    setPasswordFilled(false);
  }

  return (
    <ImeSafeForm className="trust-auth-row" onSubmit={submit}>
      <label className="trust-password-field">
        <span>{t("trust.identityResetPassword")}</span>
        <SecureImeTextField
          autoComplete="current-password"
          ref={passwordInput}
          onInput={(event) => setPasswordFilled(event.currentTarget.value.length > 0)}
        />
      </label>
      <button className="trust-action-button primary" type="submit" disabled={!passwordFilled}>
        <Check size={14} />
        <span>{t("trust.continueIdentityReset")}</span>
      </button>
      <button
        className="trust-action-button"
        type="button"
        onClick={() => onCancelIdentityReset(flowId)}
      >
        <X size={14} />
        <span>{t("trust.cancelIdentityReset")}</span>
      </button>
    </ImeSafeForm>
  );
}

function trustOverallStatus(
  trust: E2eeTrustState,
  currentSessionStatus: CurrentSessionStatusState
): { label: string; tone: TrustTone } {
  const details = currentSessionStatusDetails(currentSessionStatus);
  const detailsRemainAuthoritative =
    currentSessionStatusFactsRemainAuthoritative(currentSessionStatus);
  if (details && detailsRemainAuthoritative) {
    return details.verification === "verified"
      ? { label: t("sessionStatus.verified"), tone: "good" }
      : { label: t("trust.statusNeedsAttention"), tone: "warning" };
  }
  if (
    trust.verification.kind === "failed" ||
    trust.cross_signing.kind === "failed" ||
    trust.key_backup.kind === "failed" ||
    trust.identity_reset.kind === "failed"
  ) {
    return { label: t("trust.statusFailed"), tone: "danger" };
  }

  if (
    trust.verification.kind === "requested" ||
    trust.verification.kind === "accepted" ||
    trust.verification.kind === "sasPresented" ||
    trust.verification.kind === "confirming" ||
    trust.cross_signing.kind === "bootstrapping" ||
    trust.key_backup.kind === "enabling" ||
    trust.key_backup.kind === "restoring" ||
    trust.identity_reset.kind === "resetting" ||
    trust.identity_reset.kind === "awaitingAuth"
  ) {
    return { label: t("trust.statusInProgress"), tone: "progress" };
  }

  if (
    trust.cross_signing.kind === "trusted" &&
    trust.key_backup.kind === "enabled" &&
    trust.devices.length > 0 &&
    trust.devices.every((device) => device.trust_level === "verified")
  ) {
    return { label: t("trust.statusTrusted"), tone: "good" };
  }

  if (
    trust.cross_signing.kind === "unknown" &&
    trust.key_backup.kind === "unknown" &&
    trust.devices.length === 0
  ) {
    return { label: t("trust.statusUnknown"), tone: "neutral" };
  }

  return { label: t("trust.statusNeedsAttention"), tone: "warning" };
}

function crossSigningStatusLabel(status: CrossSigningStatus): string {
  switch (status.kind) {
    case "unknown":
      return t("trust.statusUnknown");
    case "missing":
      return t("trust.statusMissing");
    case "bootstrapping":
      return t("trust.statusBootstrapping");
    case "trusted":
      return t("trust.statusTrusted");
    case "notTrusted":
      return t("trust.statusNotTrusted");
    case "failed":
      return t("trust.statusFailedReason", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

function keyBackupStatusLabel(status: KeyBackupStatus): string {
  switch (status.kind) {
    case "unknown":
      return t("trust.statusUnknown");
    case "disabled":
      return t("trust.statusDisabled");
    case "enabling":
      return t("trust.statusEnabling");
    case "enabled":
      return t("trust.statusEnabled");
    case "restoring":
      return status.total_rooms === null
        ? t("trust.statusRestoringBackupOpen", {
            restored: status.restored_rooms
          })
        : t("trust.statusRestoringBackup", {
            restored: status.restored_rooms,
            total: status.total_rooms
          });
    case "failed":
      return t("trust.statusFailedReason", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

function identityResetStatusLabel(status: IdentityResetState): string {
  switch (status.kind) {
    case "idle":
      return t("trust.statusIdle");
    case "resetting":
      return t("trust.statusResetting");
    case "awaitingAuth":
      return t("trust.statusAwaitingAuth");
    case "failed":
      return t("trust.statusFailedReason", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

function verificationStatusLabel(status: VerificationFlowState): string {
  switch (status.kind) {
    case "idle":
      return t("trust.statusIdle");
    case "requested":
      return t("trust.statusVerificationRequested");
    case "accepted":
      return t("trust.statusVerificationAccepted");
    case "sasPresented":
      return t("trust.statusSasPresented");
    case "confirming":
      return t("trust.statusConfirming");
    case "done":
      return t("trust.statusVerified");
    case "failed":
      return t("trust.statusFailedReason", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

function currentSessionKeyBackupLabel(state: "ready" | "disabled" | "unknown"): string {
  switch (state) {
    case "ready":
      return t("sessionStatus.backupReady");
    case "disabled":
      return t("sessionStatus.backupDisabled");
    case "unknown":
      return t("sessionStatus.unknown");
  }
}

function crossSigningTone(status: CrossSigningStatus): TrustTone {
  switch (status.kind) {
    case "trusted":
      return "good";
    case "bootstrapping":
      return "progress";
    case "failed":
      return "danger";
    case "unknown":
      return "neutral";
    case "missing":
    case "notTrusted":
      return "warning";
  }
}

function keyBackupTone(status: KeyBackupStatus): TrustTone {
  switch (status.kind) {
    case "enabled":
      return "good";
    case "enabling":
    case "restoring":
      return "progress";
    case "failed":
      return "danger";
    case "unknown":
      return "neutral";
    case "disabled":
      return "warning";
  }
}

function identityResetTone(status: IdentityResetState): TrustTone {
  switch (status.kind) {
    case "idle":
      return "neutral";
    case "resetting":
    case "awaitingAuth":
      return "progress";
    case "failed":
      return "danger";
  }
}

function crossSigningActionAvailable(status: CrossSigningStatus): boolean {
  return (
    status.kind === "unknown" ||
    status.kind === "missing" ||
    status.kind === "notTrusted" ||
    status.kind === "failed"
  );
}

function keyBackupActionAvailable(status: KeyBackupStatus): boolean {
  return status.kind === "unknown" || status.kind === "disabled" || status.kind === "failed";
}
