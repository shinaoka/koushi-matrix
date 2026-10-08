import { type FormEvent, type ReactNode, useRef, useState } from "react";
import { NativeModal } from "../ModalDialog";
import {
  Download,
  KeyRound,
  RefreshCcw,
  RotateCcw,
  ShieldAlert,
  ShieldCheck,
  ShieldQuestion,
  ShieldX,
  Upload
} from "lucide-react";

import { t } from "../../i18n/messages";
import { ImeSafeForm, SecureImeTextField } from "../ImeTextControl";
import { RecoveryKeyReveal } from "../RecoveryKeyReveal";
import {
  DetailRow,
  TrustActionButton,
  TrustStatusRow,
  failureKindLabel,
  type TrustTone
} from "./SettingsStatusPrimitives";
import type {
  DisplayPlatform,
  E2eeTrustState,
  LocalEncryptionState,
  RecoveryKeyDeliveryState,
  RoomKeyExportState,
  RoomKeyImportState,
  SecureBackupGateState,
  SecureBackupPassphraseChangeState,
  SecureBackupSetupIntent,
  SecureBackupSetupState
} from "../../domain/types";

export function SecuritySection({
  keyManagement,
  localEncryption,
  secureBackupGate,
  platform,
  onExportRoomKeys,
  onImportRoomKeys,
  onChooseRoomKeyExportDestination,
  onChooseRoomKeyImportSource,
  onBootstrapSecureBackup,
  onChangeSecureBackupPassphrase,
  onSaveSecureBackupRecoveryKey,
  onConfirmSecureBackupRecoveryKeySaved = () => undefined,
  onOpenRecovery,
  onProbeLocalEncryption,
  onResetLocalData
}: {
  keyManagement: E2eeTrustState["key_management"];
  localEncryption: LocalEncryptionState;
  /** Account-level secure-backup gate; the row's account truth (#1201). */
  secureBackupGate?: SecureBackupGateState;
  platform: DisplayPlatform;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onChooseRoomKeyExportDestination: () => Promise<string | null>;
  onChooseRoomKeyImportSource: () => Promise<string | null>;
  onBootstrapSecureBackup: (passphrase: string | null, intent: SecureBackupSetupIntent) => void;
  onChangeSecureBackupPassphrase: (oldSecret: string, newPassphrase: string) => void;
  /** Optional "Save to file…" for a revealed recovery key (#927). */
  onSaveSecureBackupRecoveryKey?: (revealRequestId: number) => Promise<void>;
  /** Explicit "I saved the recovery key" confirmation (#927). */
  onConfirmSecureBackupRecoveryKeySaved?: (revealRequestId: number) => void | Promise<void>;
  onOpenRecovery: () => void;
  onProbeLocalEncryption: () => void;
  onResetLocalData: () => void;
}) {
  const status = localEncryptionStatus(localEncryption);
  const roomKeyPassphraseRef = useRef<HTMLInputElement>(null);
  const secureBackupPassphraseRef = useRef<HTMLInputElement>(null);
  const oldSecureBackupSecretRef = useRef<HTMLInputElement>(null);
  const newSecureBackupPassphraseRef = useRef<HTMLInputElement>(null);
  const [roomKeyPassphraseRequest, setRoomKeyPassphraseRequest] =
    useState<RoomKeyPassphraseRequest | null>(null);
  async function chooseRoomKeyExport(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const destinationPath = await onChooseRoomKeyExportDestination();
    if (!destinationPath) {
      return;
    }
    setRoomKeyPassphraseRequest({ kind: "export", path: destinationPath });
  }

  async function chooseRoomKeyImport(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const sourcePath = await onChooseRoomKeyImportSource();
    if (!sourcePath) {
      return;
    }
    setRoomKeyPassphraseRequest({ kind: "import", path: sourcePath });
  }

  function submitRoomKeyPassphrase(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const passphrase = roomKeyPassphraseRef.current?.value ?? "";
    if (!roomKeyPassphraseRequest || !passphrase) {
      return;
    }
    if (roomKeyPassphraseRequest.kind === "export") {
      onExportRoomKeys(roomKeyPassphraseRequest.path, passphrase);
    } else {
      onImportRoomKeys(roomKeyPassphraseRequest.path, passphrase);
    }
    if (roomKeyPassphraseRef.current) {
      roomKeyPassphraseRef.current.value = "";
    }
    setRoomKeyPassphraseRequest(null);
  }

  function closeRoomKeyPassphraseDialog() {
    if (roomKeyPassphraseRef.current) {
      roomKeyPassphraseRef.current.value = "";
    }
    setRoomKeyPassphraseRequest(null);
  }

  // #927: the revealed key is rendered straight from the Rust snapshot.
  const secureBackupSetup = keyManagement.secure_backup_setup;
  const passphraseChange = keyManagement.passphrase_change;
  function recoveryKeyReveal(reveal: {
    request_id: number;
    recovery_key: string;
    delivery: RecoveryKeyDeliveryState;
    confirmation_failed?: boolean;
  }) {
    return (
      <RecoveryKeyReveal
        key={reveal.request_id}
        recoveryKey={reveal.recovery_key}
        delivery={reveal.delivery}
        confirmationFailed={reveal.confirmation_failed ?? false}
        onSaveToFile={
          onSaveSecureBackupRecoveryKey
            ? () => onSaveSecureBackupRecoveryKey(reveal.request_id)
            : undefined
        }
        onConfirmSaved={async () => {
          await onConfirmSecureBackupRecoveryKeySaved(reveal.request_id);
        }}
      />
    );
  }

  // #927: setup and passphrase change reveal the key on screen; choosing a
  // file is an optional follow-up, never a precondition.
  function submitSecureBackupSetup(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const passphrase = secureBackupPassphraseRef.current?.value ?? "";
    onBootstrapSecureBackup(passphrase.length > 0 ? passphrase : null, { kind: "initialSetup" });
    if (secureBackupPassphraseRef.current) {
      secureBackupPassphraseRef.current.value = "";
    }
  }

  function submitSecureBackupPassphraseChange(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const oldSecret = oldSecureBackupSecretRef.current?.value ?? "";
    const newPassphrase = newSecureBackupPassphraseRef.current?.value ?? "";
    if (!oldSecret || !newPassphrase) {
      return;
    }
    onChangeSecureBackupPassphrase(oldSecret, newPassphrase);
    if (oldSecureBackupSecretRef.current) {
      oldSecureBackupSecretRef.current.value = "";
    }
    if (newSecureBackupPassphraseRef.current) {
      newSecureBackupPassphraseRef.current.value = "";
    }
  }

  const secureBackupStatus = (
    <KeyManagementStatus
      label={t("settings.secureBackup")}
      value={secureBackupStatusLabel(secureBackupSetup, secureBackupGate)}
      testId="secure-backup-state"
    />
  );
  // #1201: Core admits the setup request only on a gate that asks for it, so the
  // form appears only where it would be accepted, plus while one is in flight.
  const showSecureBackupSetup =
    secureBackupGate === undefined ||
    secureBackupGate.kind === "setupRequired" ||
    secureBackupSetup.kind === "settingUp" ||
    secureBackupSetup.kind === "failed";

  return (
    <>
      <div className="settings-detail-list">
        <DetailRow
          label={t("settings.credentialStore")}
          value={credentialStoreLabel(platform)}
        />
        <DetailRow label={t("settings.searchIndex")} value={t("settings.searchIndex")} />
      </div>
      <div className="trust-status-list">
        <TrustStatusRow
          icon={status.icon}
          label={t("settings.localEncryption")}
          value={status.label}
          tone={status.tone}
          action={
            <TrustActionButton
              icon={<RefreshCcw size={14} />}
              label={t("settings.checkLocalEncryption")}
              disabled={localEncryption.kind === "probing"}
              onClick={onProbeLocalEncryption}
            />
          }
        />
        <TrustStatusRow
          icon={<RotateCcw size={16} />}
          label={t("settings.localData")}
          value={t("settings.localDataResetAvailable")}
          tone={localEncryption.kind === "resetting" ? "progress" : "danger"}
          action={
            <>
              <TrustActionButton
                icon={<KeyRound size={14} />}
                label={t("settings.openRecovery")}
                variant="secondary"
                onClick={onOpenRecovery}
              />
              <TrustActionButton
                icon={<RotateCcw size={14} />}
                label={t("settings.resetLocalData")}
                disabled={localEncryption.kind === "resetting"}
                onClick={onResetLocalData}
              />
            </>
          }
        />
      </div>
      <section className="settings-section" aria-label={t("settings.keyManagement")}>
        <h4 className="settings-subheading">{t("settings.keyManagement")}</h4>
        <div className="settings-control-stack">
          <ImeSafeForm
            aria-label={t("settings.roomKeyExport")}
            className="profile-settings-form"
            onSubmit={(event) => {
              void chooseRoomKeyExport(event);
            }}
          >
            <KeyManagementStatus
              label={t("settings.roomKeyExport")}
              value={roomKeyExportStatusLabel(keyManagement.room_key_export)}
              testId="room-key-export-state"
            />
            <p className="profile-settings-hint">{t("settings.chooseRoomKeyExportFile")}</p>
            <div className="profile-settings-actions">
              <button
                className="trust-action-button primary"
                type="submit"
                disabled={keyManagement.room_key_export.kind === "exporting"}
              >
                <Download size={14} />
                <span>{t("settings.exportRoomKeys")}</span>
              </button>
            </div>
          </ImeSafeForm>

          <ImeSafeForm
            aria-label={t("settings.roomKeyImport")}
            className="profile-settings-form"
            onSubmit={(event) => {
              void chooseRoomKeyImport(event);
            }}
          >
            <KeyManagementStatus
              label={t("settings.roomKeyImport")}
              value={roomKeyImportStatusLabel(keyManagement.room_key_import)}
              testId="room-key-import-state"
            />
            <p className="profile-settings-hint">{t("settings.chooseRoomKeyImportFile")}</p>
            <div className="profile-settings-actions">
              <button
                className="trust-action-button primary"
                type="submit"
                disabled={keyManagement.room_key_import.kind === "importing"}
              >
                <Upload size={14} />
                <span>{t("settings.importRoomKeys")}</span>
              </button>
            </div>
          </ImeSafeForm>

          {secureBackupSetup.kind === "recoveryKeyReady" ? (
            <div className="profile-settings-form">
              {secureBackupStatus}
              {recoveryKeyReveal(secureBackupSetup)}
            </div>
          ) : showSecureBackupSetup ? (
          <ImeSafeForm
            aria-label={t("settings.secureBackup")}
            className="profile-settings-form"
            onSubmit={submitSecureBackupSetup}
          >
            {secureBackupStatus}
            <label className="profile-settings-field">
              <span>{t("settings.secureBackupPassphrase")}</span>
              <SecureImeTextField
                ref={secureBackupPassphraseRef}
                autoComplete="new-password"
              />
            </label>
            <div className="profile-settings-actions">
              <button className="trust-action-button primary" type="submit">
                <KeyRound size={14} />
                <span>{t("settings.setupSecureBackup")}</span>
              </button>
            </div>
          </ImeSafeForm>
          ) : (
            <div className="profile-settings-form">{secureBackupStatus}</div>
          )}

          {passphraseChange.kind === "changed" ? (
            <div className="profile-settings-form">
              <KeyManagementStatus
                label={t("settings.changeSecureBackupPassphrase")}
                value={secureBackupPassphraseChangeStatusLabel(passphraseChange)}
                testId="secure-backup-passphrase-change-state"
              />
              {recoveryKeyReveal(passphraseChange)}
            </div>
          ) : (
          <ImeSafeForm
            aria-label={t("settings.changeSecureBackupPassphrase")}
            className="profile-settings-form"
            onSubmit={submitSecureBackupPassphraseChange}
          >
            <KeyManagementStatus
              label={t("settings.changeSecureBackupPassphrase")}
              value={secureBackupPassphraseChangeStatusLabel(keyManagement.passphrase_change)}
              testId="secure-backup-passphrase-change-state"
            />
            <label className="profile-settings-field">
              <span>{t("settings.oldSecureBackupSecret")}</span>
              <SecureImeTextField
                ref={oldSecureBackupSecretRef}
                autoComplete="current-password"
              />
            </label>
            <label className="profile-settings-field">
              <span>{t("settings.newSecureBackupPassphrase")}</span>
              <SecureImeTextField
                ref={newSecureBackupPassphraseRef}
                autoComplete="new-password"
              />
            </label>
            <div className="profile-settings-actions">
              <button className="trust-action-button primary" type="submit">
                <RefreshCcw size={14} />
                <span>{t("settings.updateSecureBackupPassphrase")}</span>
              </button>
            </div>
          </ImeSafeForm>
          )}
        </div>
      </section>
      {roomKeyPassphraseRequest ? (
        <NativeModal className="dialog-overlay" aria-labelledby="room-key-passphrase-title" onDismiss={closeRoomKeyPassphraseDialog}>
          <ImeSafeForm
            className="dialog-box"
            onSubmit={submitRoomKeyPassphrase}
          >
            <h3 className="dialog-title" id="room-key-passphrase-title">
              {t("settings.roomKeyPassphrase")}
            </h3>
            <p className="profile-settings-hint">
              {roomKeyPassphraseRequest.kind === "export"
                ? t("settings.roomKeyPassphrasePromptExport")
                : t("settings.roomKeyPassphrasePromptImport")}
            </p>
            <SecureImeTextField
              className="dialog-input"
              ref={roomKeyPassphraseRef}
              autoComplete="new-password"
              aria-label={t("settings.roomKeyPassphrase")}
            />
            <div className="dialog-actions">
              <button className="dialog-button" type="button" onClick={closeRoomKeyPassphraseDialog}>
                {t("action.cancel")}
              </button>
              <button className="dialog-button is-primary" type="submit">
                {roomKeyPassphraseRequest.kind === "export"
                  ? t("settings.exportRoomKeys")
                  : t("settings.importRoomKeys")}
              </button>
            </div>
          </ImeSafeForm>
        </NativeModal>
      ) : null}
    </>
  );
}

type RoomKeyPassphraseRequest =
  | { kind: "export"; path: string }
  | { kind: "import"; path: string };

function KeyManagementStatus({
  label,
  value,
  testId
}: {
  label: string;
  value: string;
  testId: string;
}) {
  return (
    <div className="settings-detail-row">
      <span>{label}</span>
      <small data-testid={testId}>{value}</small>
    </div>
  );
}

function credentialStoreLabel(platform: DisplayPlatform): string {
  switch (platform) {
    case "macos":
      return t("settings.credentialStoreMacos");
    case "windows":
      return t("settings.credentialStoreWindows");
    case "linux":
      return t("settings.credentialStoreLinux");
  }
}

function localEncryptionStatus(state: LocalEncryptionState): {
  label: string;
  tone: TrustTone;
  icon: ReactNode;
} {
  switch (state.kind) {
    case "healthy":
      return {
        label: t("settings.localEncryptionHealthy"),
        tone: "good",
        icon: <ShieldCheck size={16} />
      };
    case "probing":
      return {
        label: t("settings.localEncryptionChecking"),
        tone: "progress",
        icon: <RefreshCcw size={16} />
      };
    case "unavailable":
      return {
        label: t("settings.localEncryptionUnavailable"),
        tone: "danger",
        icon: <ShieldX size={16} />
      };
    case "lockedOrInaccessible":
      return {
        label: t("settings.localEncryptionLocked"),
        tone: "warning",
        icon: <ShieldAlert size={16} />
      };
    case "missingCredential":
      return {
        label: t("settings.localEncryptionMissing"),
        tone: "danger",
        icon: <ShieldX size={16} />
      };
    case "resetRequired":
      return {
        label: t("settings.localEncryptionResetRequired"),
        tone: "danger",
        icon: <ShieldX size={16} />
      };
    case "resetting":
      return {
        label: t("settings.localEncryptionResetting"),
        tone: "progress",
        icon: <RefreshCcw size={16} />
      };
    case "unknown":
      return {
        label: t("settings.localEncryptionUnknown"),
        tone: "neutral",
        icon: <ShieldQuestion size={16} />
      };
  }
}

function roomKeyExportStatusLabel(status: RoomKeyExportState): string {
  switch (status.kind) {
    case "idle":
      return t("settings.roomKeyExportIdle");
    case "exporting":
      return t("settings.roomKeyExporting");
    case "exported":
      return status.exported_sessions === null
        ? t("settings.roomKeyExportedUnknown")
        : t("settings.roomKeyExportedCount", { count: status.exported_sessions });
    case "failed":
      return t("settings.roomKeyExportFailed", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

function roomKeyImportStatusLabel(status: RoomKeyImportState): string {
  switch (status.kind) {
    case "idle":
      return t("settings.roomKeyImportIdle");
    case "importing":
      return t("settings.roomKeyImporting");
    case "imported":
      return t("settings.roomKeyImportedCount", {
        imported: status.imported_count,
        total: status.total_count
      });
    case "failed":
      return t("settings.roomKeyImportFailed", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

function secureBackupStatusLabel(
  setup: SecureBackupSetupState,
  gate: SecureBackupGateState | undefined
): string {
  switch (setup.kind) {
    case "settingUp":
      return t("settings.secureBackupSettingUp");
    case "recoveryKeyReady":
      return recoveryKeyDeliveryLabel(setup.delivery);
    case "enabled":
      return t("settings.secureBackupEnabled");
    case "failed":
      return t("settings.secureBackupFailed", {
        reason: failureKindLabel(setup.failureKind)
      });
    case "idle":
      // #1201: this session's setup request says nothing about an account that
      // already has secure backup, so the account gate owns the idle label.
      return gate === undefined
        ? t("settings.secureBackupIdle")
        : secureBackupGateStatusLabel(gate);
  }
}

function secureBackupGateStatusLabel(gate: SecureBackupGateState): string {
  switch (gate.kind) {
    case "ready":
    case "uploadingExistingKeys":
    case "degradedRetrying":
      return t("settings.secureBackupEnabled");
    case "checking":
    case "creatingBackup":
      return t("settings.secureBackupSettingUp");
    default:
      return t("settings.secureBackupIdle");
  }
}

function secureBackupPassphraseChangeStatusLabel(
  status: SecureBackupPassphraseChangeState
): string {
  switch (status.kind) {
    case "idle":
      return t("settings.passphraseChangeIdle");
    case "changing":
      return t("settings.passphraseChangeChanging");
    case "changed":
      return status.delivery.kind === "written"
        ? t("settings.passphraseChangeRecoveryKeySaved")
        : t("settings.passphraseChangeChanged");
    case "failed":
      return t("settings.passphraseChangeFailed", {
        reason: failureKindLabel(status.failureKind)
      });
  }
}

function recoveryKeyDeliveryLabel(delivery: RecoveryKeyDeliveryState): string {
  switch (delivery.kind) {
    case "written":
      return t("settings.recoveryKeySaved");
    case "notWritten":
    case "writeFailed":
      return t("settings.recoveryKeyReady");
  }
}
