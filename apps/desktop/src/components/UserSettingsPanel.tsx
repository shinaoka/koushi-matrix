import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import {
  Bell,
  Code2,
  Check,
  Download,
  Edit3,
  EyeOff,
  Image,
  KeyRound,
  Keyboard,
  Link,
  RefreshCcw,
  RotateCcw,
  ShieldAlert,
  ShieldCheck,
  ShieldQuestion,
  ShieldX,
  SlidersHorizontal,
  Smartphone,
  Upload,
  UserRound,
  X
} from "lucide-react";

import { t } from "../i18n/messages";
import type {
  AccountManagementCapabilities,
  AccountManagementState,
  CrossSigningStatus,
  DeviceSessionListState,
  DeviceSessionSummary,
  DeviceTrustLevel,
  DisplaySettings,
  E2eeTrustState,
  EmojiPreference,
  DisplayPlatform,
  FontPreference,
  ImageUploadCompressionMode,
  IdentityResetState,
  KeyBackupStatus,
  LocalEncryptionState,
  MediaSettings,
  NotificationSettings,
  RecoveryKeyDeliveryState,
  SavedSessionInfo,
  SettingsPatch,
  SettingsState,
  ProfileState,
  RoomKeyExportState,
  RoomKeyImportState,
  SecureBackupPassphraseChangeState,
  SecureBackupSetupState,
  ThemePreference,
  TrustOperationFailureKind,
  VerificationFlowState
} from "../domain/types";

export function UserSettingsPanel({
  currentSession,
  savedSessions,
  settings,
  profile,
  e2eeTrust,
  localEncryption,
  platform,
  deviceSessions,
  accountManagement,
  accountManagementCapabilities,
  onOpenKeyboardSettings,
  onUpdateSettings,
  onSetDisplayName,
  onSetAvatar,
  onBootstrapCrossSigning,
  onEnableKeyBackup,
  onExportRoomKeys,
  onImportRoomKeys,
  onBootstrapSecureBackup,
  onChangeSecureBackupPassphrase,
  onAcceptVerification,
  onConfirmSasVerification,
  onCancelVerification,
  onResetIdentity,
  onSubmitIdentityResetPassword,
  onSubmitIdentityResetOAuth,
  onProbeLocalEncryption,
  onResetLocalData,
  onOpenRecovery,
  onSwitchAccount,
  onQueryDevices,
  onRenameDevice,
  onDeleteDevices,
  onLoadAccountManagementCapabilities,
  onChangePassword,
  onDeactivateAccount,
  onSubmitAccountManagementUia
}: {
  currentSession: SavedSessionInfo | null;
  savedSessions: SavedSessionInfo[];
  settings: SettingsState;
  profile: ProfileState;
  e2eeTrust: E2eeTrustState;
  localEncryption: LocalEncryptionState;
  platform: DisplayPlatform;
  deviceSessions: DeviceSessionListState;
  accountManagement: AccountManagementState;
  accountManagementCapabilities: AccountManagementCapabilities;
  onOpenKeyboardSettings: () => void;
  onUpdateSettings: (patch: SettingsPatch) => void;
  onSetDisplayName: (displayName: string | null) => void;
  onSetAvatar: (file: File) => void;
  onBootstrapCrossSigning: () => void;
  onEnableKeyBackup: () => void;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onBootstrapSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onChangeSecureBackupPassphrase: (
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onAcceptVerification: (flowId: number) => void;
  onConfirmSasVerification: (flowId: number) => void;
  onCancelVerification: (flowId: number) => void;
  onResetIdentity: () => void;
  onSubmitIdentityResetPassword: (flowId: number, password: string) => void;
  onSubmitIdentityResetOAuth: (flowId: number) => void;
  onProbeLocalEncryption: () => void;
  onResetLocalData: () => void;
  onOpenRecovery: () => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
  onQueryDevices: () => void;
  onRenameDevice: (deviceOrdinal: number, displayName: string) => void;
  onDeleteDevices: (deviceOrdinals: number[]) => void;
  onLoadAccountManagementCapabilities: () => void;
  onChangePassword: (newPassword: string) => void;
  onDeactivateAccount: (eraseData: boolean) => void;
  onSubmitAccountManagementUia: (flowId: number, password: string) => void;
}) {
  useEffect(() => {
    if (deviceSessions.kind === "idle" && currentSession) {
      onQueryDevices();
    }
  }, [deviceSessions.kind, currentSession, onQueryDevices]);
  const selectedTheme = settings.values.appearance.theme;
  const selectedFont = settings.values.typography.font;
  const selectedEmoji = settings.values.typography.emoji;
  const selectedNotifications = settings.values.notifications;
  const selectedDisplay = settings.values.display;
  const selectedMedia = settings.values.media;
  const isSaving = settings.persistence.kind === "saving";
  const [displayNameDraft, setDisplayNameDraft] = useState(profile.own.display_name ?? "");
  const avatarInputRef = useRef<HTMLInputElement | null>(null);
  const profileBusy = profile.update.kind !== "idle";
  const displayNameBusy = profile.update.kind === "settingDisplayName";
  const avatarBusy = profile.update.kind === "settingAvatar";
  const profileAvatarUrl = avatarSourceUrl(profile.own.avatar);
  const profileInitial = profile.own.display_name?.charAt(0).toUpperCase()
    || accountInitial(currentSession?.user_id ?? "");

  useEffect(() => {
    setDisplayNameDraft(profile.own.display_name ?? "");
  }, [profile.own.display_name]);

  function submitDisplayName(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (profileBusy) {
      return;
    }
    const trimmed = displayNameDraft.trim();
    onSetDisplayName(trimmed.length > 0 ? trimmed : null);
  }

  function selectAvatarFile(file: File | null) {
    if (!file || avatarBusy) {
      return;
    }
    onSetAvatar(file);
  }

  return (
    <section className="settings-panel user-settings-panel" aria-labelledby="user-settings-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="user-settings-title">{t("panel.userSettings")}</h2>
          <p dir="auto">{currentSession?.user_id ?? t("settings.matrixAccount")}</p>
        </div>
      </header>

      <div className="settings-list">
        <button className="settings-list-item" type="button">
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <UserRound size={16} />
            </span>
            <span>{t("settings.general")}</span>
          </span>
        </button>
        <button className="settings-list-item" type="button">
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <ShieldCheck size={16} />
            </span>
            <span>{t("settings.securityPrivacy")}</span>
          </span>
        </button>
        <button className="settings-list-item" type="button">
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Smartphone size={16} />
            </span>
            <span>{t("settings.sessions")}</span>
          </span>
        </button>
        <button className="settings-list-item" type="button" onClick={onOpenKeyboardSettings}>
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Keyboard size={16} />
            </span>
            <span>{t("settings.keyboard")}</span>
          </span>
        </button>
        <button className="settings-list-item" type="button">
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <SlidersHorizontal size={16} />
            </span>
            <span>{t("settings.preferences")}</span>
          </span>
        </button>
      </div>

      <section className="settings-section" aria-label={t("settings.profile")}>
        <h3>{t("settings.profile")}</h3>
        <div className="profile-settings">
          <div className="profile-settings-avatar" aria-hidden="true">
            {profileAvatarUrl ? (
              <img src={profileAvatarUrl} />
            ) : (
              <span>{profileInitial}</span>
            )}
          </div>
          <form className="profile-settings-form" onSubmit={submitDisplayName}>
            <label className="profile-settings-field">
              <span>{t("settings.profileDisplayName")}</span>
              <input
                type="text"
                value={displayNameDraft}
                placeholder={t("settings.profileDisplayNamePlaceholder")}
                disabled={profileBusy}
                onChange={(event) => setDisplayNameDraft(event.currentTarget.value)}
              />
            </label>
            <div className="profile-settings-actions">
              <button
                className="profile-settings-action"
                type="submit"
                disabled={profileBusy}
              >
                <Check size={14} />
                <span>
                  {displayNameBusy ? t("settings.profileSavingDisplayName") : t("settings.profileUpdate")}
                </span>
              </button>
              <input
                ref={avatarInputRef}
                className="sr-only"
                type="file"
                accept="image/png,image/jpeg,image/webp,image/gif"
                onChange={(event) => {
                  selectAvatarFile(event.currentTarget.files?.[0] ?? null);
                  event.currentTarget.value = "";
                }}
              />
              <button
                className="profile-settings-action"
                type="button"
                disabled={profileBusy}
                onClick={() => avatarInputRef.current?.click()}
              >
                <Image size={14} />
                <span>
                  {avatarBusy ? t("settings.profileSavingAvatar") : t("settings.profileUploadAvatar")}
                </span>
              </button>
            </div>
          </form>
        </div>
      </section>

      <section className="settings-section" aria-label={t("settings.session")}>
        <h3>{t("settings.session")}</h3>
        <div className="settings-detail-list">
          <DetailRow label={t("settings.homeserver")} value={currentSession?.homeserver ?? t("settings.notRestored")} />
          <DetailRow label={t("settings.userId")} value={currentSession?.user_id ?? t("settings.notRestored")} />
          <DetailRow label={t("settings.device")} value={currentSession?.device_id ?? t("settings.notRestored")} />
          <DetailRow label={t("settings.localStoreLabel")} value={t("settings.localStore")} />
        </div>
      </section>

      <section className="settings-section" aria-label={t("settings.appearance")}>
        <div className="settings-section-heading">
          <h3>{t("settings.appearance")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="segmented-control" role="group" aria-label={t("settings.theme")}>
          <ThemeButton
            label={t("settings.themeSystem")}
            selected={selectedTheme === "system"}
            value="system"
            onSelect={onUpdateSettings}
          />
          <ThemeButton
            label={t("settings.themeLight")}
            selected={selectedTheme === "light"}
            value="light"
            onSelect={onUpdateSettings}
          />
          <ThemeButton
            label={t("settings.themeDark")}
            selected={selectedTheme === "dark"}
            value="dark"
            onSelect={onUpdateSettings}
          />
        </div>
        <h4 className="settings-subheading">{t("settings.typography")}</h4>
        <div className="settings-control-stack">
          <div className="settings-control-row">
            <span>{t("settings.uiFont")}</span>
            <div className="segmented-control" role="group" aria-label={t("settings.uiFont")}>
              <FontButton
                label={t("settings.fontSystem")}
                selected={selectedFont === "system"}
                value="system"
                currentEmoji={selectedEmoji}
                onSelect={onUpdateSettings}
              />
              <FontButton
                label={t("settings.fontInter")}
                selected={selectedFont === "inter"}
                value="inter"
                currentEmoji={selectedEmoji}
                onSelect={onUpdateSettings}
              />
            </div>
          </div>
          <div className="settings-control-row">
            <span>{t("settings.emojiFont")}</span>
            <div className="segmented-control" role="group" aria-label={t("settings.emojiFont")}>
              <EmojiButton
                label={t("settings.fontSystem")}
                selected={selectedEmoji === "system"}
                value="system"
                currentFont={selectedFont}
                onSelect={onUpdateSettings}
              />
              <EmojiButton
                label={t("settings.twemojiColr")}
                selected={selectedEmoji === "twemojiColr"}
                value="twemojiColr"
                currentFont={selectedFont}
                onSelect={onUpdateSettings}
              />
            </div>
          </div>
        </div>
      </section>

      <section className="settings-section" aria-label={t("settings.display")}>
        <div className="settings-section-heading">
          <h3>{t("settings.display")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="settings-toggle-list">
          <DisplayToggle
            label={t("settings.codeBlockWrap")}
            settingKey="code_block_wrap"
            icon="code"
            current={selectedDisplay}
            onSelect={onUpdateSettings}
          />
          <DisplayToggle
            label={t("settings.urlPreviews")}
            description={t("settings.urlPreviewsDescription")}
            settingKey="url_previews_enabled"
            icon="link"
            current={selectedDisplay}
            onSelect={onUpdateSettings}
          />
          <DisplayToggle
            label={t("settings.hideRedacted")}
            settingKey="hide_redacted"
            icon="hideRedacted"
            current={selectedDisplay}
            onSelect={onUpdateSettings}
          />
        </div>
      </section>

      <section className="settings-section" aria-label={t("settings.media")}>
        <div className="settings-section-heading">
          <h3>{t("settings.media")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="settings-control-row">
          <span>{t("settings.compressImages")}</span>
          <div className="segmented-control" role="group" aria-label={t("settings.compressImages")}>
            <ImageCompressionButton
              label={t("settings.compressImagesAlways")}
              selected={selectedMedia.image_upload_compression === "always"}
              value="always"
              current={selectedMedia}
              onSelect={onUpdateSettings}
            />
            <ImageCompressionButton
              label={t("settings.compressImagesAsk")}
              selected={selectedMedia.image_upload_compression === "ask"}
              value="ask"
              current={selectedMedia}
              onSelect={onUpdateSettings}
            />
            <ImageCompressionButton
              label={t("settings.compressImagesNever")}
              selected={selectedMedia.image_upload_compression === "never"}
              value="never"
              current={selectedMedia}
              onSelect={onUpdateSettings}
            />
          </div>
        </div>
      </section>

      <section className="settings-section" aria-label={t("settings.notifications")}>
        <div className="settings-section-heading">
          <h3>{t("settings.notifications")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="settings-toggle-list">
          <NotificationToggle
            label={t("settings.notificationDesktop")}
            settingKey="desktop_notifications"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
          />
          <NotificationToggle
            label={t("settings.notificationSound")}
            settingKey="sound"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
          />
          <NotificationToggle
            label={t("settings.notificationBadges")}
            settingKey="badges"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
          />
          <NotificationToggle
            label={t("settings.sendReadReceipts")}
            settingKey="send_read_receipts"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
          />
          <NotificationToggle
            label={t("settings.sendTypingNotifications")}
            settingKey="send_typing_notifications"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
          />
        </div>
      </section>

      <section className="settings-section" aria-label={t("settings.security")}>
        <h3>{t("settings.security")}</h3>
        <SecuritySection
          keyManagement={e2eeTrust.key_management}
          localEncryption={localEncryption}
          platform={platform}
          onBootstrapSecureBackup={onBootstrapSecureBackup}
          onChangeSecureBackupPassphrase={onChangeSecureBackupPassphrase}
          onExportRoomKeys={onExportRoomKeys}
          onImportRoomKeys={onImportRoomKeys}
          onOpenRecovery={onOpenRecovery}
          onProbeLocalEncryption={onProbeLocalEncryption}
          onResetLocalData={onResetLocalData}
        />
      </section>

      <SessionsSection
        deviceSessions={deviceSessions}
        accountManagement={accountManagement}
        onQueryDevices={onQueryDevices}
        onRenameDevice={onRenameDevice}
        onDeleteDevices={onDeleteDevices}
        onSubmitAccountManagementUia={onSubmitAccountManagementUia}
      />

      <AccountManagementSection
        accountManagement={accountManagement}
        accountManagementCapabilities={accountManagementCapabilities}
        currentSession={currentSession}
        onLoadAccountManagementCapabilities={onLoadAccountManagementCapabilities}
        onChangePassword={onChangePassword}
        onDeactivateAccount={onDeactivateAccount}
        onSubmitAccountManagementUia={onSubmitAccountManagementUia}
      />

      <TrustSection
        trust={e2eeTrust}
        onAcceptVerification={onAcceptVerification}
        onBootstrapCrossSigning={onBootstrapCrossSigning}
        onCancelVerification={onCancelVerification}
        onConfirmSasVerification={onConfirmSasVerification}
        onEnableKeyBackup={onEnableKeyBackup}
        onResetIdentity={onResetIdentity}
        onSubmitIdentityResetOAuth={onSubmitIdentityResetOAuth}
        onSubmitIdentityResetPassword={onSubmitIdentityResetPassword}
      />

      <section className="account-switcher" aria-label={t("settings.accountSwitcher")}>
        <h3>{t("settings.accounts")}</h3>
        <div className="account-switcher-list">
          {savedSessions.map((session) => {
            const isCurrent = sessionMatches(currentSession, session);
            return (
              <article className="account-switcher-row" key={sessionKey(session)}>
                <div className="account-switcher-avatar" aria-hidden="true">
                  {accountInitial(session.user_id)}
                </div>
                <div className="account-switcher-main">
                  <div className="account-switcher-user" dir="auto">{session.user_id}</div>
                  <div className="account-switcher-meta" dir="auto">
                    {session.homeserver} / {session.device_id}
                  </div>
                </div>
                <button
                  className="account-switcher-action"
                  type="button"
                  disabled={isCurrent}
                  onClick={() => onSwitchAccount(session)}
                >
                  <RefreshCcw size={14} />
                  <span>{isCurrent ? t("settings.current") : t("settings.switch")}</span>
                </button>
              </article>
            );
          })}
        </div>
      </section>
    </section>
  );
}

function SecuritySection({
  keyManagement,
  localEncryption,
  platform,
  onExportRoomKeys,
  onImportRoomKeys,
  onBootstrapSecureBackup,
  onChangeSecureBackupPassphrase,
  onOpenRecovery,
  onProbeLocalEncryption,
  onResetLocalData
}: {
  keyManagement: E2eeTrustState["key_management"];
  localEncryption: LocalEncryptionState;
  platform: DisplayPlatform;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onBootstrapSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onChangeSecureBackupPassphrase: (
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onOpenRecovery: () => void;
  onProbeLocalEncryption: () => void;
  onResetLocalData: () => void;
}) {
  const status = localEncryptionStatus(localEncryption);
  const exportDestinationRef = useRef<HTMLInputElement>(null);
  const exportPassphraseRef = useRef<HTMLInputElement>(null);
  const importSourceRef = useRef<HTMLInputElement>(null);
  const importPassphraseRef = useRef<HTMLInputElement>(null);
  const secureBackupPassphraseRef = useRef<HTMLInputElement>(null);
  const secureBackupRecoveryPathRef = useRef<HTMLInputElement>(null);
  const oldSecureBackupSecretRef = useRef<HTMLInputElement>(null);
  const newSecureBackupPassphraseRef = useRef<HTMLInputElement>(null);
  const passphraseChangeRecoveryPathRef = useRef<HTMLInputElement>(null);
  const canReset =
    localEncryption.kind === "missingCredential" ||
    localEncryption.kind === "resetRequired" ||
    localEncryption.kind === "resetting";

  function submitRoomKeyExport(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const destinationPath = exportDestinationRef.current?.value.trim() ?? "";
    const passphrase = exportPassphraseRef.current?.value ?? "";
    if (!destinationPath || !passphrase) {
      return;
    }
    onExportRoomKeys(destinationPath, passphrase);
    if (exportPassphraseRef.current) {
      exportPassphraseRef.current.value = "";
    }
  }

  function submitRoomKeyImport(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const sourcePath = importSourceRef.current?.value.trim() ?? "";
    const passphrase = importPassphraseRef.current?.value ?? "";
    if (!sourcePath || !passphrase) {
      return;
    }
    onImportRoomKeys(sourcePath, passphrase);
    if (importPassphraseRef.current) {
      importPassphraseRef.current.value = "";
    }
  }

  function submitSecureBackupSetup(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const passphrase = secureBackupPassphraseRef.current?.value ?? "";
    const recoveryPath = secureBackupRecoveryPathRef.current?.value.trim() ?? "";
    onBootstrapSecureBackup(passphrase.length > 0 ? passphrase : null, recoveryPath || null);
    if (secureBackupPassphraseRef.current) {
      secureBackupPassphraseRef.current.value = "";
    }
  }

  function submitSecureBackupPassphraseChange(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const oldSecret = oldSecureBackupSecretRef.current?.value ?? "";
    const newPassphrase = newSecureBackupPassphraseRef.current?.value ?? "";
    const recoveryPath = passphraseChangeRecoveryPathRef.current?.value.trim() ?? "";
    if (!oldSecret || !newPassphrase) {
      return;
    }
    onChangeSecureBackupPassphrase(oldSecret, newPassphrase, recoveryPath || null);
    if (oldSecureBackupSecretRef.current) {
      oldSecureBackupSecretRef.current.value = "";
    }
    if (newSecureBackupPassphraseRef.current) {
      newSecureBackupPassphraseRef.current.value = "";
    }
  }

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
        {canReset ? (
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
        ) : null}
      </div>
      <section className="settings-section" aria-label={t("settings.keyManagement")}>
        <h4 className="settings-subheading">{t("settings.keyManagement")}</h4>
        <div className="settings-control-stack">
          <form
            aria-label={t("settings.roomKeyExport")}
            className="profile-settings-form"
            onSubmit={submitRoomKeyExport}
          >
            <KeyManagementStatus
              label={t("settings.roomKeyExport")}
              value={roomKeyExportStatusLabel(keyManagement.room_key_export)}
              testId="room-key-export-state"
            />
            <label className="profile-settings-field">
              <span>{t("settings.roomKeyExportDestination")}</span>
              <input ref={exportDestinationRef} autoComplete="off" type="text" />
            </label>
            <label className="profile-settings-field">
              <span>{t("settings.roomKeyPassphrase")}</span>
              <input ref={exportPassphraseRef} autoComplete="new-password" type="password" />
            </label>
            <div className="profile-settings-actions">
              <button className="trust-action-button primary" type="submit">
                <Download size={14} />
                <span>{t("settings.exportRoomKeys")}</span>
              </button>
            </div>
          </form>

          <form
            aria-label={t("settings.roomKeyImport")}
            className="profile-settings-form"
            onSubmit={submitRoomKeyImport}
          >
            <KeyManagementStatus
              label={t("settings.roomKeyImport")}
              value={roomKeyImportStatusLabel(keyManagement.room_key_import)}
              testId="room-key-import-state"
            />
            <label className="profile-settings-field">
              <span>{t("settings.roomKeyImportSource")}</span>
              <input ref={importSourceRef} autoComplete="off" type="text" />
            </label>
            <label className="profile-settings-field">
              <span>{t("settings.roomKeyPassphrase")}</span>
              <input ref={importPassphraseRef} autoComplete="new-password" type="password" />
            </label>
            <div className="profile-settings-actions">
              <button className="trust-action-button primary" type="submit">
                <Upload size={14} />
                <span>{t("settings.importRoomKeys")}</span>
              </button>
            </div>
          </form>

          <form
            aria-label={t("settings.secureBackup")}
            className="profile-settings-form"
            onSubmit={submitSecureBackupSetup}
          >
            <KeyManagementStatus
              label={t("settings.secureBackup")}
              value={secureBackupSetupStatusLabel(keyManagement.secure_backup_setup)}
              testId="secure-backup-state"
            />
            <label className="profile-settings-field">
              <span>{t("settings.secureBackupPassphrase")}</span>
              <input
                ref={secureBackupPassphraseRef}
                autoComplete="new-password"
                type="password"
              />
            </label>
            <label className="profile-settings-field">
              <span>{t("settings.recoveryKeyDestination")}</span>
              <input ref={secureBackupRecoveryPathRef} autoComplete="off" type="text" />
            </label>
            <div className="profile-settings-actions">
              <button className="trust-action-button primary" type="submit">
                <KeyRound size={14} />
                <span>{t("settings.setupSecureBackup")}</span>
              </button>
            </div>
          </form>

          <form
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
              <input
                ref={oldSecureBackupSecretRef}
                autoComplete="current-password"
                type="password"
              />
            </label>
            <label className="profile-settings-field">
              <span>{t("settings.newSecureBackupPassphrase")}</span>
              <input
                ref={newSecureBackupPassphraseRef}
                autoComplete="new-password"
                type="password"
              />
            </label>
            <label className="profile-settings-field">
              <span>{t("settings.recoveryKeyDestination")}</span>
              <input ref={passphraseChangeRecoveryPathRef} autoComplete="off" type="text" />
            </label>
            <div className="profile-settings-actions">
              <button className="trust-action-button primary" type="submit">
                <RefreshCcw size={14} />
                <span>{t("settings.updateSecureBackupPassphrase")}</span>
              </button>
            </div>
          </form>
        </div>
      </section>
    </>
  );
}

function SessionsSection({
  deviceSessions,
  accountManagement,
  onQueryDevices,
  onRenameDevice,
  onDeleteDevices,
  onSubmitAccountManagementUia
}: {
  deviceSessions: DeviceSessionListState;
  accountManagement: AccountManagementState;
  onQueryDevices: () => void;
  onRenameDevice: (deviceOrdinal: number, displayName: string) => void;
  onDeleteDevices: (deviceOrdinals: number[]) => void;
  onSubmitAccountManagementUia: (flowId: number, password: string) => void;
}) {
  const [renamingOrdinal, setRenamingOrdinal] = useState<number | null>(null);

  const currentDevice =
    deviceSessions.kind === "loaded"
      ? deviceSessions.devices.find((device) => device.current)
      : undefined;
  const otherDevices =
    deviceSessions.kind === "loaded"
      ? deviceSessions.devices.filter((device) => !device.current)
      : [];
  const otherOrdinals = otherDevices.map((device) => device.device_ordinal);

  return (
    <section className="settings-section" aria-label={t("settings.sessions")}>
      <div className="settings-section-heading">
        <h3>{t("settings.sessions")}</h3>
      </div>

      {accountManagement.kind === "awaitingUia" &&
      (accountManagement.operation === "renameDevice" ||
        accountManagement.operation === "deleteDevice" ||
        accountManagement.operation === "deleteOtherDevices") ? (
        <AccountManagementUiaForm
          flowId={accountManagement.flow_id}
          onSubmit={onSubmitAccountManagementUia}
        />
      ) : null}

      {deviceSessions.kind === "idle" || deviceSessions.kind === "loading" ? (
        <p className="settings-status-text">{t("settings.sessionsLoading")}</p>
      ) : null}

      {deviceSessions.kind === "failed" ? (
        <>
          <p className="settings-status-text">{t("settings.sessionsLoadFailed")}</p>
          <button className="trust-action-button secondary" type="button" onClick={onQueryDevices}>
            <RefreshCcw size={14} />
            <span>{t("action.restartSync")}</span>
          </button>
        </>
      ) : null}

      <div className="sessions-list">
        {currentDevice ? (
          <div className="session-row session-row-current">
            <div className="session-main">
              <strong>{currentDevice.display_name ?? t("settings.deviceNamePlaceholder")}</strong>
              <span className="session-meta">{t("settings.currentSession")}</span>
            </div>
            <div className="session-badges">
              {currentDevice.verified ? (
                <span className="session-badge verified">{t("settings.deviceVerified")}</span>
              ) : (
                <span className="session-badge unverified">{t("settings.deviceUnverified")}</span>
              )}
              {currentDevice.inactive ? (
                <span className="session-badge inactive">{t("settings.deviceInactive")}</span>
              ) : null}
            </div>
          </div>
        ) : null}

        {otherDevices.length > 0 ? (
          <>
            <h4 className="settings-subheading">{t("settings.otherSessions")}</h4>
            {otherDevices.map((device) => (
              <SessionRow
                key={device.device_ordinal}
                device={device}
                renaming={renamingOrdinal === device.device_ordinal}
                onStartRename={() => setRenamingOrdinal(device.device_ordinal)}
                onCancelRename={() => setRenamingOrdinal(null)}
                onRename={(displayName) => {
                  setRenamingOrdinal(null);
                  onRenameDevice(device.device_ordinal, displayName);
                }}
                onSignOut={() => onDeleteDevices([device.device_ordinal])}
              />
            ))}
            <div className="session-actions">
              <button
                className="trust-action-button danger"
                type="button"
                onClick={() => onDeleteDevices(otherOrdinals)}
              >
                <X size={14} />
                <span>{t("settings.signOutOthers")}</span>
              </button>
            </div>
          </>
        ) : null}
      </div>
    </section>
  );
}

function AccountManagementSection({
  accountManagement,
  accountManagementCapabilities,
  currentSession,
  onLoadAccountManagementCapabilities,
  onChangePassword,
  onDeactivateAccount,
  onSubmitAccountManagementUia
}: {
  accountManagement: AccountManagementState;
  accountManagementCapabilities: AccountManagementCapabilities;
  currentSession: SavedSessionInfo | null;
  onLoadAccountManagementCapabilities: () => void;
  onChangePassword: (newPassword: string) => void;
  onDeactivateAccount: (eraseData: boolean) => void;
  onSubmitAccountManagementUia: (flowId: number, password: string) => void;
}) {
  useEffect(() => {
    if (currentSession && accountManagementCapabilities.change_password.kind === "unknown") {
      onLoadAccountManagementCapabilities();
    }
  }, [currentSession, accountManagementCapabilities.change_password.kind, onLoadAccountManagementCapabilities]);

  const [showChangePassword, setShowChangePassword] = useState(false);
  const [showDeactivate, setShowDeactivate] = useState(false);
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
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
    if (newPassword !== confirmPassword) {
      setMismatch(true);
      return;
    }
    setMismatch(false);
    onChangePassword(newPassword);
  }

  function submitDeactivate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onDeactivateAccount(eraseData);
  }

  function resetForms() {
    setShowChangePassword(false);
    setShowDeactivate(false);
    setNewPassword("");
    setConfirmPassword("");
    setEraseData(false);
    setMismatch(false);
  }

  return (
    <section className="settings-section" aria-label={t("settings.accountManagement")}>
      <div className="settings-section-heading">
        <h3>{t("settings.accountManagement")}</h3>
      </div>

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
        <form className="profile-settings-form" onSubmit={submitChangePassword}>
          <label className="profile-settings-field">
            <span>{t("settings.changePasswordLabel")}</span>
            <input
              type="password"
              autoComplete="new-password"
              value={newPassword}
              onInput={(event) => setNewPassword(event.currentTarget.value)}
              data-testid="change-password-input"
            />
          </label>
          <label className="profile-settings-field">
            <span>{t("settings.changePasswordConfirm")}</span>
            <input
              type="password"
              autoComplete="new-password"
              value={confirmPassword}
              onInput={(event) => setConfirmPassword(event.currentTarget.value)}
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
              disabled={!newPassword || !confirmPassword || accountManagement.kind === "working"}
              data-testid="change-password-submit"
            >
              {t("settings.changePassword")}
            </button>
          </div>
        </form>
      ) : null}

      {showDeactivate ? (
        <form className="settings-form" onSubmit={submitDeactivate}>
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
        </form>
      ) : null}
    </section>
  );
}

function SessionRow({
  device,
  renaming,
  onStartRename,
  onCancelRename,
  onRename,
  onSignOut
}: {
  device: DeviceSessionSummary;
  renaming: boolean;
  onStartRename: () => void;
  onCancelRename: () => void;
  onRename: (displayName: string) => void;
  onSignOut: () => void;
}) {
  const [draft, setDraft] = useState(device.display_name ?? "");

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = draft.trim();
    onRename(trimmed.length > 0 ? trimmed : device.display_name ?? "");
  }

  if (renaming) {
    return (
      <form className="session-row session-row-renaming" onSubmit={submit}>
        <label className="session-rename-field">
          <span className="sr-only">{t("settings.deviceNamePlaceholder")}</span>
          <input
            type="text"
            value={draft}
            placeholder={t("settings.deviceNamePlaceholder")}
            onChange={(event) => setDraft(event.currentTarget.value)}
          />
        </label>
        <div className="session-actions">
          <button className="trust-action-button primary" type="submit">
            <Check size={14} />
            <span>{t("settings.renameDevice")}</span>
          </button>
          <button
            className="trust-action-button secondary"
            type="button"
            onClick={onCancelRename}
          >
            <X size={14} />
            <span>{t("action.cancel")}</span>
          </button>
        </div>
      </form>
    );
  }

  return (
    <div className="session-row">
      <div className="session-main">
        <strong>{device.display_name ?? t("settings.deviceNamePlaceholder")}</strong>
      </div>
      <div className="session-badges">
        {device.verified ? (
          <span className="session-badge verified">{t("settings.deviceVerified")}</span>
        ) : (
          <span className="session-badge unverified">{t("settings.deviceUnverified")}</span>
        )}
        {device.inactive ? (
          <span className="session-badge inactive">{t("settings.deviceInactive")}</span>
        ) : null}
      </div>
      <div className="session-actions">
        <button className="trust-action-button secondary" type="button" onClick={onStartRename}>
          <Edit3 size={14} />
          <span>{t("settings.renameDevice")}</span>
        </button>
        <button className="trust-action-button danger" type="button" onClick={onSignOut}>
          <X size={14} />
          <span>{t("settings.signOut")}</span>
        </button>
      </div>
    </div>
  );
}

function AccountManagementUiaForm({
  flowId,
  onSubmit
}: {
  flowId: number;
  onSubmit: (flowId: number, password: string) => void;
}) {
  const passwordInput = useRef<HTMLInputElement>(null);
  const [passwordFilled, setPasswordFilled] = useState(false);

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const password = passwordInput.current?.value ?? "";
    if (!password) {
      return;
    }
    onSubmit(flowId, password);
    if (passwordInput.current) {
      passwordInput.current.value = "";
    }
    setPasswordFilled(false);
  }

  return (
    <form className="trust-auth-row" onSubmit={submit}>
      <label className="trust-password-field">
        <span>{t("auth.password")}</span>
        <input
          autoComplete="current-password"
          ref={passwordInput}
          type="password"
          onInput={(event) => setPasswordFilled(event.currentTarget.value.length > 0)}
        />
      </label>
      <button className="trust-action-button primary" type="submit" disabled={!passwordFilled}>
        <Check size={14} />
        <span>{t("action.continue")}</span>
      </button>
    </form>
  );
}

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

function TrustSection({
  trust,
  onBootstrapCrossSigning,
  onEnableKeyBackup,
  onAcceptVerification,
  onConfirmSasVerification,
  onCancelVerification,
  onResetIdentity,
  onSubmitIdentityResetPassword,
  onSubmitIdentityResetOAuth
}: {
  trust: E2eeTrustState;
  onBootstrapCrossSigning: () => void;
  onEnableKeyBackup: () => void;
  onAcceptVerification: (flowId: number) => void;
  onConfirmSasVerification: (flowId: number) => void;
  onCancelVerification: (flowId: number) => void;
  onResetIdentity: () => void;
  onSubmitIdentityResetPassword: (flowId: number, password: string) => void;
  onSubmitIdentityResetOAuth: (flowId: number) => void;
}) {
  const overall = trustOverallStatus(trust);

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
          value={crossSigningStatusLabel(trust.cross_signing)}
          tone={crossSigningTone(trust.cross_signing)}
          action={
            crossSigningActionAvailable(trust.cross_signing) ? (
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
          value={keyBackupStatusLabel(trust.key_backup)}
          tone={keyBackupTone(trust.key_backup)}
          action={
            keyBackupActionAvailable(trust.key_backup) ? (
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
        onSubmitIdentityResetOAuth={onSubmitIdentityResetOAuth}
        onSubmitIdentityResetPassword={onSubmitIdentityResetPassword}
      />

      <DeviceTrustList devices={trust.devices} />
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

function secureBackupSetupStatusLabel(status: SecureBackupSetupState): string {
  switch (status.kind) {
    case "idle":
      return t("settings.secureBackupIdle");
    case "settingUp":
      return t("settings.secureBackupSettingUp");
    case "recoveryKeyReady":
      return recoveryKeyDeliveryLabel(status.delivery);
    case "enabled":
      return t("settings.secureBackupEnabled");
    case "failed":
      return t("settings.secureBackupFailed", {
        reason: failureKindLabel(status.failureKind)
      });
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
      return t("settings.recoveryKeyReady");
  }
}

function TrustStatusRow({
  icon,
  label,
  value,
  tone,
  action
}: {
  icon: ReactNode;
  label: string;
  value: string;
  tone: TrustTone;
  action?: ReactNode;
}) {
  return (
    <div className="trust-status-row">
      <span className={`trust-status-icon ${tone}`} aria-hidden="true">
        {icon}
      </span>
      <span className="trust-status-copy">
        <span>{label}</span>
        <small>{value}</small>
      </span>
      {action ? <span className="trust-status-action">{action}</span> : null}
    </div>
  );
}

function TrustActionButton({
  icon,
  label,
  disabled = false,
  variant = "primary",
  onClick
}: {
  icon: ReactNode;
  label: string;
  disabled?: boolean;
  variant?: "primary" | "secondary";
  onClick: () => void;
}) {
  return (
    <button
      className={`trust-action-button ${variant}`}
      type="button"
      disabled={disabled}
      onClick={onClick}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

function IdentityResetAuthControls({
  state,
  onSubmitIdentityResetPassword,
  onSubmitIdentityResetOAuth
}: {
  state: IdentityResetState;
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
    <form className="trust-auth-row" onSubmit={submit}>
      <label className="trust-password-field">
        <span>{t("trust.identityResetPassword")}</span>
        <input
          autoComplete="current-password"
          ref={passwordInput}
          type="password"
          onInput={(event) => setPasswordFilled(event.currentTarget.value.length > 0)}
        />
      </label>
      <button className="trust-action-button primary" type="submit" disabled={!passwordFilled}>
        <Check size={14} />
        <span>{t("trust.continueIdentityReset")}</span>
      </button>
    </form>
  );
}

function DeviceTrustList({ devices }: { devices: E2eeTrustState["devices"] }) {
  return (
    <section className="trust-devices" aria-label={t("trust.devices")}>
      <div className="trust-devices-heading">
        <h4>{t("trust.devices")}</h4>
        <span>{t("trust.deviceCount", { count: devices.length })}</span>
      </div>
      <div className="trust-device-list">
        {devices.length > 0 ? (
          devices.map((device, index) => (
            <div className="trust-device-row" key={`${device.user_id}|${device.device_id}`}>
              <span className={`trust-device-icon ${device.trust_level}`} aria-hidden="true">
                {deviceTrustIcon(device.trust_level)}
              </span>
              <span className="trust-device-copy">
                <span>{t("trust.deviceOrdinal", { index: index + 1 })}</span>
                <small>{deviceTrustLevelLabel(device.trust_level)}</small>
              </span>
            </div>
          ))
        ) : (
          <div className="trust-device-row">
            <span className="trust-device-icon unknown" aria-hidden="true">
              <ShieldQuestion size={15} />
            </span>
            <span className="trust-device-copy">
              <span>{t("trust.noDevices")}</span>
              <small>{t("trust.statusUnknown")}</small>
            </span>
          </div>
        )}
      </div>
    </section>
  );
}

type TrustTone = "good" | "warning" | "danger" | "neutral" | "progress";

function trustOverallStatus(trust: E2eeTrustState): { label: string; tone: TrustTone } {
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

function deviceTrustLevelLabel(level: DeviceTrustLevel): string {
  switch (level) {
    case "unknown":
      return t("trust.deviceUnknown");
    case "unverified":
      return t("trust.deviceUnverified");
    case "verified":
      return t("trust.deviceVerified");
    case "blocked":
      return t("trust.deviceBlocked");
  }
}

function deviceTrustIcon(level: DeviceTrustLevel): ReactNode {
  switch (level) {
    case "verified":
      return <ShieldCheck size={15} />;
    case "blocked":
      return <ShieldX size={15} />;
    case "unknown":
      return <ShieldQuestion size={15} />;
    case "unverified":
      return <ShieldAlert size={15} />;
  }
}

function failureKindLabel(kind: TrustOperationFailureKind): string {
  switch (kind) {
    case "cancelled":
      return t("trust.failureCancelled");
    case "mismatch":
      return t("trust.failureMismatch");
    case "network":
      return t("trust.failureNetwork");
    case "forbidden":
      return t("trust.failureForbidden");
    case "timeout":
      return t("trust.failureTimeout");
    case "sdk":
      return t("trust.failureSdk");
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

function ThemeButton({
  label,
  selected,
  value,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: ThemePreference;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ appearance: { theme: value } });
        }
      }}
    >
      {label}
    </button>
  );
}

function FontButton({
  label,
  selected,
  value,
  currentEmoji,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: FontPreference;
  currentEmoji: EmojiPreference;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ typography: { font: value, emoji: currentEmoji } });
        }
      }}
    >
      {label}
    </button>
  );
}

function EmojiButton({
  label,
  selected,
  value,
  currentFont,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: EmojiPreference;
  currentFont: FontPreference;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({ typography: { font: currentFont, emoji: value } });
        }
      }}
    >
      {label}
    </button>
  );
}

function ImageCompressionButton({
  label,
  selected,
  value,
  current,
  onSelect
}: {
  label: string;
  selected: boolean;
  value: ImageUploadCompressionMode;
  current: MediaSettings;
  onSelect: (patch: SettingsPatch) => void;
}) {
  return (
    <button
      className={`segmented-control-option ${selected ? "is-selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={() => {
        if (!selected) {
          onSelect({
            media: {
              ...current,
              image_upload_compression: value
            }
          });
        }
      }}
    >
      {label}
    </button>
  );
}

function NotificationToggle({
  label,
  settingKey,
  current,
  onSelect
}: {
  label: string;
  settingKey: keyof NotificationSettings;
  current: NotificationSettings;
  onSelect: (patch: SettingsPatch) => void;
}) {
  const checked = current[settingKey];
  return (
    <button
      className="settings-toggle-row"
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => {
        onSelect({
          notifications: {
            ...current,
            [settingKey]: !checked
          }
        });
      }}
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <Bell size={15} aria-hidden="true" />
          <span>{label}</span>
        </span>
      </span>
      <span className="settings-switch-track" aria-hidden="true">
        <span className="settings-switch-thumb" />
      </span>
    </button>
  );
}

function DisplayToggle({
  label,
  description,
  settingKey,
  icon,
  current,
  onSelect
}: {
  label: string;
  description?: string;
  settingKey: keyof DisplaySettings;
  icon: "code" | "hideRedacted" | "link";
  current: DisplaySettings;
  onSelect: (patch: SettingsPatch) => void;
}) {
  const checked = current[settingKey];
  const Icon = icon === "code" ? Code2 : icon === "hideRedacted" ? EyeOff : Link;
  return (
    <button
      className="settings-toggle-row"
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => {
        onSelect({
          display: {
            ...current,
            [settingKey]: !checked
          }
        });
      }}
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <Icon size={15} aria-hidden="true" />
          <span>{label}</span>
        </span>
        {description ? (
          <span className="settings-toggle-description">{description}</span>
        ) : null}
      </span>
      <span className="settings-switch-track" aria-hidden="true">
        <span className="settings-switch-thumb" />
      </span>
    </button>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-detail-row">
      <span>{label}</span>
      <small>{value}</small>
    </div>
  );
}

function sessionMatches(left: SavedSessionInfo | null, right: SavedSessionInfo): boolean {
  return (
    left?.homeserver === right.homeserver &&
    left.user_id === right.user_id &&
    left.device_id === right.device_id
  );
}

function sessionKey(session: SavedSessionInfo): string {
  return `${session.homeserver}|${session.user_id}|${session.device_id}`;
}

function avatarSourceUrl(avatar: ProfileState["own"]["avatar"]): string | null {
  if (avatar?.thumbnail.kind !== "ready") {
    return null;
  }
  return avatar.thumbnail.source_url;
}

function accountInitial(userId: string): string {
  return userId.replace(/^@/, "").charAt(0).toUpperCase() || "?";
}
