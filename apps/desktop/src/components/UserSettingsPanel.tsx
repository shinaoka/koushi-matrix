import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import {
  Bell,
  Code2,
  Check,
  Edit3,
  EyeOff,
  History,
  Image,
  Link,
  LogOut,
  Monitor,
  RefreshCcw
} from "lucide-react";

import { settingsCategories, type SettingsCategoryId } from "../domain/settingsNavigation";
import { HelpContent } from "./HelpDialog";
import { t } from "../i18n/messages";
import { ImeSafeForm, ImeTextField } from "./ImeTextControl";
import { KeyboardSettingsContent } from "./KeyboardSettingsPanel";
import { SearchHistorySection } from "./user-settings/SearchHistorySection";
import { AccountManagementSection } from "./user-settings/AccountManagementSection";
import { SecuritySection } from "./user-settings/SecuritySection";
import { TrustSection } from "./user-settings/TrustSection";
import { AppearanceControls, LanguageControls } from "./user-settings/AppearanceControls";
import { DetailRow } from "./user-settings/SettingsStatusPrimitives";
import type { DisplayDensity } from "../domain/types";
import type { ShortcutLabelProfile } from "../domain/shortcuts";
import { renderableThumbnailSourceUrl } from "../backend/linkMediaRuntime";
import { currentSessionStatusDetails } from "../domain/currentSessionStatus";
import type {
  AccountManagementCapabilities,
  AccountManagementState,
  CurrentSessionStatusState,
  DesktopUpdateState,
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
  TimelineSettings,
  UpdatesSettings,
  WindowSettings
} from "../domain/types";

export function UserSettingsPanel({
  initialCategory = "account",
  currentSession,
  currentSessionStatus = { status: "idle" },
  displayDensity = "comfortable",
  desktopUpdate = { kind: "unsupported" },
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
  onRestartToInstallDesktopUpdate = () => undefined,
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
  initialCategory?: SettingsCategoryId;
  currentSession: SavedSessionInfo | null;
  currentSessionStatus?: CurrentSessionStatusState;
  displayDensity?: DisplayDensity;
  desktopUpdate?: DesktopUpdateState;
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
  onUpdateSettings: (patch: SettingsPatch) => void;
  onRestartToInstallDesktopUpdate?: () => void;
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
  const selectedLocale = settings.values.locale;
  const selectedFont = settings.values.typography.font;
  const selectedEmoji = settings.values.typography.emoji;
  const selectedTimeline = settings.values.timeline;
  const selectedNotifications = settings.values.notifications;
  const selectedDisplay = settings.values.display;
  const selectedWindow = settings.values.window;
  const selectedUpdates = settings.values.updates;
  // macOS hides on close unconditionally (overview.md, "Desktop Window
  // Lifecycle And Tray"), so the setting has nothing to control there.
  const closeToTrayIsConfigurable = platform !== "macos";
  const isSaving = settings.persistence.kind === "saving";
  const [displayNameDraft, setDisplayNameDraft] = useState(profile.own.display_name ?? "");
  const [activeCategory, setActiveCategory] = useState<SettingsCategoryId>(initialCategory);
  const contentRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => { if (contentRef.current) contentRef.current.scrollTop = 0; }, [activeCategory]);
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

  return (
    <section className="settings-panel user-settings-panel" aria-label={t("panel.userSettings")}>
      <nav className="settings-category-list" role="tablist" aria-label={t("settings.categories")} aria-orientation="vertical">
        {settingsCategories.map((category, index) => (
          <button
            key={category.id}
            id={`settings-tab-${category.id}`}
            role="tab"
            type="button"
            aria-selected={activeCategory === category.id}
            aria-controls={`settings-page-${category.id}`}
            tabIndex={activeCategory === category.id ? 0 : -1}
            onClick={() => setActiveCategory(category.id)}
            onKeyDown={(event) => {
              const next = event.key === "ArrowDown" ? (index + 1) % settingsCategories.length
                : event.key === "ArrowUp" ? (index + settingsCategories.length - 1) % settingsCategories.length
                : event.key === "Home" ? 0 : event.key === "End" ? settingsCategories.length - 1 : null;
              if (next !== null) {
                event.preventDefault();
                setActiveCategory(settingsCategories[next].id);
                document.getElementById(`settings-tab-${settingsCategories[next].id}`)?.focus();
              }
            }}
          >{t(category.label)}</button>
        ))}
      </nav>
      <div className="settings-category-content" ref={contentRef}>
        <div id="settings-page-account" role="tabpanel" aria-labelledby="settings-tab-account" className="settings-category" hidden={activeCategory !== "account"} tabIndex={0}>
          <section className="settings-section" aria-label={t("settings.language")}>
            <LanguageControls selectedLocale={selectedLocale} onUpdateSettings={onUpdateSettings} />
          </section>
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
        </div>
        <div id="settings-page-sessions" role="tabpanel" aria-labelledby="settings-tab-sessions" className="settings-category" hidden={activeCategory !== "sessions"} tabIndex={0}>
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
        </div>
        <div id="settings-page-appearance" role="tabpanel" aria-labelledby="settings-tab-appearance" className="settings-category" hidden={activeCategory !== "appearance"} tabIndex={0}>
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
        </div>
        <div id="settings-page-notifications" role="tabpanel" aria-labelledby="settings-tab-notifications" className="settings-category" hidden={activeCategory !== "notifications"} tabIndex={0}>
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
        </div>
        <div id="settings-page-preferences" role="tabpanel" aria-labelledby="settings-tab-preferences" className="settings-category" hidden={activeCategory !== "preferences"} tabIndex={0}>
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
              {closeToTrayIsConfigurable ? (
                <WindowToggle
                  label={t("settings.closeToTray")}
                  description={t("settings.closeToTrayDescription")}
                  settingKey="close_to_tray"
                  current={selectedWindow}
                  onSelect={onUpdateSettings}
                />
              ) : null}
              {platform === "macos" ? (
                <DesktopUpdateControls
                  current={selectedUpdates}
                  state={desktopUpdate}
                  onSelect={onUpdateSettings}
                  onRestart={onRestartToInstallDesktopUpdate}
                />
              ) : null}
            </div>
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
        </div>
        <div id="settings-page-keyboard" role="tabpanel" aria-labelledby="settings-tab-keyboard" className="settings-category" hidden={activeCategory !== "keyboard"} tabIndex={0}>
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
        </div>
        <div id="settings-page-privacy" role="tabpanel" aria-labelledby="settings-tab-privacy" className="settings-category" hidden={activeCategory !== "privacy"} tabIndex={0}>
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
        </div>
        <div id="settings-page-encryption" role="tabpanel" aria-labelledby="settings-tab-encryption" className="settings-category" hidden={activeCategory !== "encryption"} tabIndex={0}>
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
        </div>
        <div id="settings-page-search" role="tabpanel" aria-labelledby="settings-tab-search" className="settings-category" hidden={activeCategory !== "search"} tabIndex={0}>
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
        </div>
        <div id="settings-page-help" role="tabpanel" aria-labelledby="settings-tab-help" className="settings-category" hidden={activeCategory !== "help"} tabIndex={0}>
          <HelpContent />
        </div>
      </div>
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

function WindowToggle({
  label,
  description,
  settingKey,
  current,
  onSelect
}: {
  label: string;
  description?: string;
  settingKey: keyof WindowSettings;
  current: WindowSettings;
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
          window: {
            ...current,
            [settingKey]: !checked
          }
        });
      }}
    >
      <span className="settings-toggle-copy">
        <span className="settings-toggle-label">
          <Monitor size={15} aria-hidden="true" />
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

export function DesktopUpdateControls({
  current,
  state,
  onSelect,
  onRestart
}: {
  current: UpdatesSettings;
  state: DesktopUpdateState;
  onSelect: (patch: SettingsPatch) => void;
  onRestart: () => void;
}) {
  return (
    <>
      <button
        className="settings-toggle-row"
        type="button"
        role="switch"
        aria-checked={current.auto_check}
        aria-label={t("settings.autoUpdate")}
        onClick={() => onSelect({ updates: { auto_check: !current.auto_check } })}
      >
        <span className="settings-toggle-copy">
          <span className="settings-toggle-label">
            <RefreshCcw size={15} aria-hidden="true" />
            <span>{t("settings.autoUpdate")}</span>
          </span>
          <span className="settings-toggle-description">
            {t("settings.autoUpdateDescription")}
          </span>
        </span>
        <span className="settings-switch-track" aria-hidden="true">
          <span className="settings-switch-thumb" />
        </span>
      </button>
      {state.kind !== "unsupported" ? (
        <div className="settings-update-status" aria-live="polite">
          <p className="settings-status-text">{desktopUpdateStatusText(state)}</p>
          {state.kind === "ready" ? (
            <button className="profile-settings-action" type="button" onClick={onRestart}>
              <RefreshCcw size={14} aria-hidden="true" />
              {t("settings.updateRestart")}
            </button>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

function desktopUpdateStatusText(state: DesktopUpdateState): string {
  switch (state.kind) {
    case "idle":
      return t("settings.updateIdle");
    case "checking":
      return t("settings.updateChecking");
    case "downloading":
      return t("settings.updateDownloading", { version: state.version });
    case "ready":
      return t("settings.updateReady", { version: state.version });
    case "installing":
      return t("settings.updateInstalling", { version: state.version });
    case "failed":
      return state.stage === "install"
        ? t("settings.updateInstallFailed")
        : t("settings.updateCheckFailed");
    case "unsupported":
      return "";
  }
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
