import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import {
  Bell,
  Code2,
  Check,
  Edit3,
  EyeOff,
  History,
  Image,
  Keyboard,
  Link,
  LogOut,
  MessageCircle,
  Monitor,
  RefreshCcw,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  Smartphone,
  UserRound
} from "lucide-react";

import { t } from "../i18n/messages";
import { ImeSafeForm, ImeTextField } from "./ImeTextControl";
import { KeyboardSettingsContent } from "./KeyboardSettingsPanel";
import { SearchHistorySection } from "./user-settings/SearchHistorySection";
import { AccountManagementSection } from "./user-settings/AccountManagementSection";
import { SecuritySection } from "./user-settings/SecuritySection";
import { TrustSection } from "./user-settings/TrustSection";
import { AppearanceControls } from "./user-settings/AppearanceControls";
import { DetailRow } from "./user-settings/SettingsStatusPrimitives";
import type { DisplayDensity } from "../domain/types";
import type { ShortcutLabelProfile } from "../domain/shortcuts";
import { renderableThumbnailSourceUrl } from "../backend/linkMediaRuntime";
import { currentSessionStatusDetails } from "../domain/currentSessionStatus";
import type {
  AccountManagementCapabilities,
  AccountManagementState,
  CurrentSessionStatusState,
  DisplaySettings,
  E2eeTrustState,
  DisplayPlatform,
  LocalEncryptionState,
  NotificationSettings,
  RoomSummary,
  SavedSessionInfo,
  SearchCrawlerState,
  SettingsPatch,
  SettingsState,
  SecureBackupSetupIntent,
  ProfileState,
  TimelineSettings
} from "../domain/types";

export function UserSettingsPanel({
  currentSession,
  currentSessionStatus = { status: "idle" },
  displayDensity = "comfortable",
  savedSessions,
  settings,
  searchCrawlerState,
  profile,
  e2eeTrust,
  localEncryption,
  platform,
  accountManagement,
  accountManagementCapabilities,
  keyboardLabelProfile,
  onUpdateSettings,
  onRebuildSearchIndex,
  onSetDisplayName,
  onSetAvatar,
  onBootstrapCrossSigning,
  onEnableKeyBackup,
  onChooseRoomKeyExportDestination,
  onChooseRoomKeyImportSource,
  onChooseSecureBackupDestination = async () => null,
  onExportRoomKeys,
  onImportRoomKeys,
  onBootstrapSecureBackup,
  onChangeSecureBackupPassphrase,
  onAcceptVerification,
  onConfirmSasVerification,
  onCancelVerification,
  onResetIdentity,
  onCancelIdentityReset,
  onSubmitIdentityResetPassword,
  onSubmitIdentityResetOAuth,
  onProbeLocalEncryption,
  onResetLocalData,
  onLogout,
  onOpenRecovery,
  onSwitchAccount,
  onLoadAccountManagementCapabilities,
  onRefreshCurrentSessionStatus = () => undefined,
  onChangePassword,
  onDeactivateAccount,
  onSubmitAccountManagementUia,
  onStartCrawlRoom,
  onStopCrawlRoom,
  onDisplayDensityChange = () => undefined,
  accountManagementUrl = null,
  onManageAccount = () => undefined,
  rooms
}: {
  currentSession: SavedSessionInfo | null;
  currentSessionStatus?: CurrentSessionStatusState;
  displayDensity?: DisplayDensity;
  savedSessions: SavedSessionInfo[];
  settings: SettingsState;
  searchCrawlerState?: SearchCrawlerState;
  profile: ProfileState;
  e2eeTrust: E2eeTrustState;
  localEncryption: LocalEncryptionState;
  platform: DisplayPlatform;
  accountManagement: AccountManagementState;
  accountManagementCapabilities: AccountManagementCapabilities;
  keyboardLabelProfile?: ShortcutLabelProfile;
  onOpenKeyboardSettings: () => void;
  onUpdateSettings: (patch: SettingsPatch) => void;
  onRebuildSearchIndex?: () => void;
  onSetDisplayName: (displayName: string | null) => void;
  onSetAvatar: (file: File) => void;
  onBootstrapCrossSigning: () => void;
  onEnableKeyBackup: () => void;
  onChooseRoomKeyExportDestination: () => Promise<string | null>;
  onChooseRoomKeyImportSource: () => Promise<string | null>;
  onChooseSecureBackupDestination?: () => Promise<string | null>;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onBootstrapSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null,
    intent: SecureBackupSetupIntent
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
  onCancelIdentityReset: (flowId: number) => void;
  onSubmitIdentityResetPassword: (flowId: number, password: string) => void;
  onSubmitIdentityResetOAuth: (flowId: number) => void;
  onProbeLocalEncryption: () => void;
  onResetLocalData: () => void;
  onLogout: () => void;
  onOpenRecovery: () => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
  onLoadAccountManagementCapabilities: () => void;
  onRefreshCurrentSessionStatus?: () => void;
  onChangePassword: (newPassword: string) => void;
  onDeactivateAccount: (eraseData: boolean) => void;
  onSubmitAccountManagementUia: (flowId: number, password: string) => void;
  onStartCrawlRoom?: (roomId: string) => void;
  onStopCrawlRoom?: (roomId: string) => void;
  onDisplayDensityChange?: (density: DisplayDensity) => void;
  accountManagementUrl?: string | null;
  onManageAccount?: () => void;
  rooms?: RoomSummary[];
}) {
  const sessionStatusRefreshOwnerRef = useRef<string | null>(null);
  useEffect(() => {
    const owner = currentSession ? sessionKey(currentSession) : null;
    if (sessionStatusRefreshOwnerRef.current !== owner) {
      sessionStatusRefreshOwnerRef.current = null;
    }
    if (
      owner &&
      currentSessionStatus.status === "idle" &&
      sessionStatusRefreshOwnerRef.current !== owner
    ) {
      sessionStatusRefreshOwnerRef.current = owner;
      onRefreshCurrentSessionStatus();
    }
  }, [currentSession, currentSessionStatus.status, onRefreshCurrentSessionStatus]);
  const selectedTheme = settings.values.appearance.theme;
  const selectedFont = settings.values.typography.font;
  const selectedEmoji = settings.values.typography.emoji;
  const selectedTimeline = settings.values.timeline;
  const selectedNotifications = settings.values.notifications;
  const selectedDisplay = settings.values.display;
  const isSaving = settings.persistence.kind === "saving";
  const [displayNameDraft, setDisplayNameDraft] = useState(profile.own.display_name ?? "");
  const panelRef = useRef<HTMLElement | null>(null);
  const avatarInputRef = useRef<HTMLInputElement | null>(null);
  const profileBusy = profile.update.kind !== "idle";
  const displayNameBusy = profile.update.kind === "settingDisplayName";
  const avatarBusy = profile.update.kind === "settingAvatar";
  const profileAvatarUrl = avatarSourceUrl(profile.own.avatar);
  const profileInitial = profile.own.display_name?.charAt(0).toUpperCase()
    || accountInitial(currentSession?.user_id ?? "");
  const currentSessionDetails = currentSessionStatusDetails(currentSessionStatus);

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

  function scrollToSection(sectionId: string) {
    const panel = panelRef.current;
    const section = panel?.querySelector<HTMLElement>(`#${sectionId}`);
    if (panel && section) {
      panel.scrollTop = section.offsetTop;
    }
  }

  return (
    <section
      ref={panelRef}
      className="settings-panel user-settings-panel"
      aria-labelledby="user-settings-title"
    >
      <header className="settings-panel-header">
        <div>
          <h2 id="user-settings-title">{t("panel.userSettings")}</h2>
          <p dir="auto">{currentSession?.user_id ?? t("settings.matrixAccount")}</p>
        </div>
      </header>

      <div className="settings-list">
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-general")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <UserRound size={16} />
            </span>
            <span>{t("settings.general")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-session")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Smartphone size={16} />
            </span>
            <span>{t("settings.session")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-appearance")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <SlidersHorizontal size={16} />
            </span>
            <span>{t("settings.appearance")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-display")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Monitor size={16} />
            </span>
            <span>{t("settings.display")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-notifications")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Bell size={16} />
            </span>
            <span>{t("settings.notifications")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-messaging-privacy")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <MessageCircle size={16} />
            </span>
            <span>{t("settings.messagingPrivacy")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-keyboard")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Keyboard size={16} />
            </span>
            <span>{t("settings.keyboard")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-timeline")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <History size={16} />
            </span>
            <span>{t("settings.timeline")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-search-history")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Search size={16} />
            </span>
            <span>{t("settings.searchHistory")}</span>
          </span>
        </button>
        <button
          className="settings-list-item"
          type="button"
          onClick={() => scrollToSection("settings-security")}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <ShieldCheck size={16} />
            </span>
            <span>{t("settings.securityPrivacy")}</span>
          </span>
        </button>
      </div>

      <section id="settings-general" className="settings-section" aria-label={t("settings.profile")}>
        <h3>{t("settings.profile")}</h3>
        <div className="profile-settings">
          <div className="profile-settings-avatar" aria-hidden="true">
            {profileAvatarUrl ? (
              <img src={profileAvatarUrl} />
            ) : (
              <span>{profileInitial}</span>
            )}
          </div>
          <ImeSafeForm className="profile-settings-form" onSubmit={submitDisplayName}>
            <label className="profile-settings-field">
              <span>{t("settings.profileDisplayName")}</span>
              <ImeTextField
                value={displayNameDraft}
                syncKey={currentSession?.user_id ?? "profile-display-name"}
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
          </ImeSafeForm>
        </div>
      </section>

      <section
        id="settings-session"
        className="settings-section"
        aria-label={t("settings.session")}
      >
        <h3>{t("settings.session")}</h3>
        <div className="settings-detail-list">
          <DetailRow label={t("settings.homeserver")} value={currentSession?.homeserver ?? t("settings.notRestored")} />
          <DetailRow label={t("settings.userId")} value={currentSession?.user_id ?? t("settings.notRestored")} />
          <DetailRow label={t("settings.device")} value={currentSession?.device_id ?? t("settings.notRestored")} />
          <DetailRow
            label={t("sessionStatus.deviceName")}
            value={currentSessionDetails?.device_display_name ?? t("sessionStatus.unavailable")}
          />
          <DetailRow
            label={t("sessionStatus.verification")}
            value={currentSessionVerificationLabel(currentSessionDetails?.verification)}
          />
          <DetailRow
            label={t("sessionStatus.ownerCrossSigning")}
            value={currentSessionCrossSigningLabel(currentSessionDetails?.is_cross_signed_by_owner)}
          />
          <DetailRow
            label={t("sessionStatus.identity")}
            value={currentSessionIdentityLabel(currentSessionDetails?.own_identity_verification)}
          />
          <DetailRow
            label={t("sessionStatus.keyBackup")}
            value={currentSessionBackupLabel(currentSessionDetails?.key_backup)}
          />
          <DetailRow label={t("settings.localStoreLabel")} value={t("settings.localStore")} />
        </div>
        <div className="profile-settings-actions">
          <button
            className="profile-settings-action"
            type="button"
            disabled={!currentSession}
            onClick={onLogout}
          >
            <LogOut size={14} />
            <span>{t("settings.signOut")}</span>
          </button>
        </div>
      </section>

      <AccountSwitcherSection
        currentSession={currentSession}
        savedSessions={savedSessions}
        onSwitchAccount={onSwitchAccount}
      />

      <AccountManagementSection
        accountManagement={accountManagement}
        accountManagementCapabilities={accountManagementCapabilities}
        accountManagementUrl={accountManagementUrl}
        currentSession={currentSession}
        onLoadAccountManagementCapabilities={onLoadAccountManagementCapabilities}
        onChangePassword={onChangePassword}
        onDeactivateAccount={onDeactivateAccount}
        onManageAccount={onManageAccount}
        onSubmitAccountManagementUia={onSubmitAccountManagementUia}
      />

      <TrustSection
        trust={e2eeTrust}
        currentSessionStatus={currentSessionStatus}
        onAcceptVerification={onAcceptVerification}
        onBootstrapCrossSigning={onBootstrapCrossSigning}
        onCancelVerification={onCancelVerification}
        onConfirmSasVerification={onConfirmSasVerification}
        onEnableKeyBackup={onEnableKeyBackup}
        onResetIdentity={onResetIdentity}
        onCancelIdentityReset={onCancelIdentityReset}
        onSubmitIdentityResetOAuth={onSubmitIdentityResetOAuth}
        onSubmitIdentityResetPassword={onSubmitIdentityResetPassword}
      />

      <section id="settings-keyboard" className="settings-section" aria-label={t("settings.keyboard")}>
        <div className="settings-section-heading">
          <div>
            <h3>{t("settings.keyboard")}</h3>
            <p>{t("settings.keyboardDescription")}</p>
          </div>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <KeyboardSettingsContent
          isSaving={isSaving}
          labelProfile={keyboardLabelProfile}
          selectedSendShortcut={settings.values.keyboard.composer_send_shortcut}
          onUpdateSettings={onUpdateSettings}
        />
      </section>

      <section id="settings-timeline" className="settings-section" aria-label={t("settings.timeline")}>
        <div className="settings-section-heading">
          <h3>{t("settings.timeline")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="settings-toggle-list">
          <TimelineToggle
            label={t("settings.autoLoadOlderMessages")}
            description={t("settings.autoLoadOlderMessagesDescription")}
            settingKey="auto_load_older_messages"
            current={selectedTimeline}
            onSelect={onUpdateSettings}
          />
          <TimelineThreadRootOrderToggle
            label={t("settings.threadRootLatestReply")}
            description={t("settings.threadRootLatestReplyDescription")}
            current={selectedTimeline}
            onSelect={onUpdateSettings}
          />
        </div>
      </section>

      <section id="settings-appearance" className="settings-section" aria-label={t("settings.appearance")}>
        <div className="settings-section-heading">
          <h3>{t("settings.appearance")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <AppearanceControls
          displayDensity={displayDensity}
          selectedEmoji={selectedEmoji}
          selectedFont={selectedFont}
          selectedTheme={selectedTheme}
          onDisplayDensityChange={onDisplayDensityChange}
          onUpdateSettings={onUpdateSettings}
        />
      </section>

      <section id="settings-display" className="settings-section" aria-label={t("settings.display")}>
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
            label={t("settings.urlPreviewsUnencrypted")}
            description={t("settings.urlPreviewsUnencryptedDescription")}
            settingKey="url_previews_enabled"
            icon="link"
            current={selectedDisplay}
            onSelect={onUpdateSettings}
          />
          <DisplayToggle
            label={t("settings.urlPreviewsEncrypted")}
            description={t("settings.urlPreviewsEncryptedDescription")}
            settingKey="encrypted_url_previews_enabled"
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

      <section id="settings-notifications" className="settings-section" aria-label={t("settings.notifications")}>
        <div className="settings-section-heading">
          <h3>{t("settings.notifications")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="settings-toggle-list">
          <NotificationSettingToggle
            label={t("settings.notificationDesktop")}
            settingKey="desktop_notifications"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
            icon={<Bell size={15} aria-hidden="true" />}
          />
          <NotificationSettingToggle
            label={t("settings.notificationSound")}
            settingKey="sound"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
            icon={<Bell size={15} aria-hidden="true" />}
          />
          <NotificationSettingToggle
            label={t("settings.notificationBadges")}
            settingKey="badges"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
            icon={<Bell size={15} aria-hidden="true" />}
          />
        </div>
      </section>

      <section
        id="settings-messaging-privacy"
        className="settings-section"
        aria-label={t("settings.messagingPrivacy")}
      >
        <div className="settings-section-heading">
          <h3>{t("settings.messagingPrivacy")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
        <div className="settings-toggle-list">
          <NotificationSettingToggle
            label={t("settings.sendReadReceipts")}
            settingKey="send_read_receipts"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
            icon={<Check size={15} aria-hidden="true" />}
          />
          <NotificationSettingToggle
            label={t("settings.sendTypingNotifications")}
            settingKey="send_typing_notifications"
            current={selectedNotifications}
            onSelect={onUpdateSettings}
            icon={<Edit3 size={15} aria-hidden="true" />}
          />
        </div>
      </section>

      <section
        id="settings-search-history"
        className="settings-section"
        aria-label={t("settings.searchHistory")}
      >
        <div className="settings-section-heading">
          <h3>{t("settings.searchHistory")}</h3>
          {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
        </div>
          <SearchHistorySection
            crawlerSettings={settings.values.search_crawler}
            crawlerState={searchCrawlerState ?? { rooms: {}, last_active: null }}
          rooms={rooms}
          isSaving={isSaving}
          onUpdateSettings={onUpdateSettings}
          onRebuildSearchIndex={onRebuildSearchIndex}
          onStartCrawlRoom={onStartCrawlRoom}
          onStopCrawlRoom={onStopCrawlRoom}
        />
      </section>

      <section id="settings-security" className="settings-section" aria-label={t("settings.security")}>
        <h3>{t("settings.security")}</h3>
        <SecuritySection
          keyManagement={e2eeTrust.key_management}
          localEncryption={localEncryption}
          platform={platform}
          onBootstrapSecureBackup={onBootstrapSecureBackup}
          onChangeSecureBackupPassphrase={onChangeSecureBackupPassphrase}
          onChooseRoomKeyExportDestination={onChooseRoomKeyExportDestination}
          onChooseRoomKeyImportSource={onChooseRoomKeyImportSource}
          onChooseSecureBackupDestination={onChooseSecureBackupDestination}
          onExportRoomKeys={onExportRoomKeys}
          onImportRoomKeys={onImportRoomKeys}
          onOpenRecovery={onOpenRecovery}
          onProbeLocalEncryption={onProbeLocalEncryption}
          onResetLocalData={onResetLocalData}
        />
      </section>

    </section>
  );
}

function AccountSwitcherSection({
  currentSession,
  savedSessions,
  onSwitchAccount
}: {
  currentSession: SavedSessionInfo | null;
  savedSessions: SavedSessionInfo[];
  onSwitchAccount: (session: SavedSessionInfo) => void;
}) {
  if (savedSessions.length === 0) {
    return null;
  }

  return (
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
  );
}

function currentSessionVerificationLabel(
  state: "verified" | "unverified" | "unknown" | undefined
): string {
  if (state === undefined) return t("sessionStatus.unavailable");
  if (state === "unknown") return t("trust.statusUnknown");
  return state === "verified" ? t("sessionStatus.verified") : t("sessionStatus.unverified");
}

function currentSessionCrossSigningLabel(state: boolean | undefined): string {
  if (state === undefined) return t("sessionStatus.unavailable");
  return state ? t("sessionStatus.crossSigned") : t("sessionStatus.notCrossSigned");
}

function currentSessionIdentityLabel(
  state: "missing" | "unverified" | "verified" | undefined
): string {
  switch (state) {
    case "verified":
      return t("sessionStatus.identityVerified");
    case "unverified":
      return t("sessionStatus.identityUnverified");
    case "missing":
      return t("sessionStatus.identityMissing");
    case undefined:
      return t("sessionStatus.unavailable");
  }
}

function currentSessionBackupLabel(
  state: "ready" | "disabled" | "unknown" | undefined
): string {
  switch (state) {
    case "ready":
      return t("sessionStatus.backupReady");
    case "disabled":
      return t("sessionStatus.backupDisabled");
    case "unknown":
      return t("sessionStatus.unknown");
    case undefined:
      return t("sessionStatus.unavailable");
  }
}

function NotificationSettingToggle({
  label,
  settingKey,
  current,
  onSelect,
  icon
}: {
  label: string;
  settingKey: keyof NotificationSettings;
  current: NotificationSettings;
  onSelect: (patch: SettingsPatch) => void;
  icon: ReactNode;
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
          {icon}
          <span>{label}</span>
        </span>
      </span>
      <span className="settings-switch-track" aria-hidden="true">
        <span className="settings-switch-thumb" />
      </span>
    </button>
  );
}

function TimelineToggle({
  label,
  description,
  settingKey,
  current,
  onSelect
}: {
  label: string;
  description?: string;
  settingKey: "auto_load_older_messages";
  current: TimelineSettings;
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
          timeline: {
            ...current,
            [settingKey]: !checked
          }
        });
      }}
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <History size={15} aria-hidden="true" />
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

function TimelineThreadRootOrderToggle({
  label,
  description,
  current,
  onSelect
}: {
  label: string;
  description: string;
  current: TimelineSettings;
  onSelect: (patch: SettingsPatch) => void;
}) {
  const checked = current.thread_root_order.kind === "latestReply";
  return (
    <button
      className="settings-toggle-row"
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => {
        onSelect({
          timeline: {
            ...current,
            thread_root_order: { kind: checked ? "rootEvent" : "latestReply" }
          }
        });
      }}
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <History size={15} aria-hidden="true" />
          <span>{label}</span>
        </span>
        <span className="settings-toggle-description">{description}</span>
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
  return renderableThumbnailSourceUrl(avatar.thumbnail.source_ref);
}

function accountInitial(userId: string): string {
  return userId.replace(/^@/, "").charAt(0).toUpperCase() || "?";
}
