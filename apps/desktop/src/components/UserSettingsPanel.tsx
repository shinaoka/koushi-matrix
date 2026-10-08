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
  Monitor
} from "lucide-react";

import { settingsCategories, type SettingsCategoryId } from "../domain/settingsNavigation";
import { HelpContent } from "./HelpDialog";
import { t } from "../i18n/messages";
import { ImeSafeForm, ImeTextField } from "./ImeTextControl";
import { KeyboardSettingsContent } from "./KeyboardSettingsPanel";
import { SearchHistorySection } from "./user-settings/SearchHistorySection";
import { AccountManagementSection } from "./user-settings/AccountManagementSection";
import {
  type AccountNotificationActions,
  AccountNotificationsLoadStatus,
  EmailNotificationsSection,
  NotificationCategoriesSection,
  noopAccountNotificationActions
} from "./user-settings/AccountNotificationsSections";
import { SecuritySection } from "./user-settings/SecuritySection";
import { TrustSection } from "./user-settings/TrustSection";
import { AppearanceControls, LanguageControls } from "./user-settings/AppearanceControls";
import { DetailRow } from "./user-settings/SettingsStatusPrimitives";
import type { DisplayDensity } from "../domain/types";
import type { ShortcutLabelProfile } from "../domain/shortcuts";
import { renderableThumbnailSourceUrl } from "../backend/linkMediaRuntime";
import { avatarInitial } from "../app/uiShared";
import { currentSessionStatusDetails } from "../domain/currentSessionStatus";
import type { RuntimeAlert } from "./Shell";
import { ModalDialog } from "./ModalDialog";
import type {
  AccountManagementCapabilities,
  AccountManagementState,
  DesktopSnapshot,
  AccountNotificationsState,
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
  SessionStatusRefreshCommandTrigger,
  SecureBackupGateState,
  SecureBackupSetupIntent,
  ProfileState,
  TimelineSettings,
  WindowSettings
} from "../domain/types";

export function UserSettingsPanel({
  initialCategory = "account",
  settingsScope = "account",
  onSettingsScopeChange,
  currentSession,
  currentSessionStatus = { status: "idle" },
  displayDensity = "comfortable",
  settings,
  searchCrawlerState,
  profile,
  e2eeTrust,
  secureBackupGate,
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
  onExportRoomKeys,
  onImportRoomKeys,
  onBootstrapSecureBackup,
  onChangeSecureBackupPassphrase,
  onSaveSecureBackupRecoveryKey,
  onConfirmSecureBackupRecoveryKeySaved,
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
  onLoadAccountManagementCapabilities,
  onRefreshCurrentSessionStatus = (_trigger: SessionStatusRefreshCommandTrigger) => undefined,
  canRestartSync = false,
  onRestartSync = () => undefined,
  settingsBusy = false,
  runtimeAlerts = [],
  runtimeAlertRetrying = false,
  onRetryRuntimeAlert = () => undefined,
  onChangePassword,
  onDeactivateAccount,
  onSubmitAccountManagementUia,
  onStartCrawlRoom,
  onStopCrawlRoom,
  onDisplayDensityChange = () => undefined,
  accountManagementUrl = null,
  onManageAccount = () => undefined,
  accountNotifications = defaultAccountNotificationsState,
  accountNotificationActions = noopAccountNotificationActions,
  rooms
}: {
  initialCategory?: SettingsCategoryId;
  settingsScope?: "account" | "app";
  onSettingsScopeChange?: (scope: "account" | "app") => void;
  currentSession: SavedSessionInfo | null;
  currentSessionStatus?: CurrentSessionStatusState;
  displayDensity?: DisplayDensity;
  /** @deprecated Account selection belongs to the persistent account tabs. */
  savedSessions?: SavedSessionInfo[];
  settings: SettingsState;
  searchCrawlerState?: SearchCrawlerState;
  profile: ProfileState;
  e2eeTrust: E2eeTrustState;
  /** Account-level secure-backup gate for the row's account truth (#1201). */
  secureBackupGate?: SecureBackupGateState;
  localEncryption: LocalEncryptionState;
  platform: DisplayPlatform;
  accountManagement: AccountManagementState;
  accountManagementCapabilities: AccountManagementCapabilities;
  keyboardLabelProfile?: ShortcutLabelProfile;
  onUpdateSettings: (patch: SettingsPatch) => void;
  onRebuildSearchIndex?: () => void;
  onSetDisplayName: (displayName: string | null) => void;
  onSetAvatar: (file: File) => void;
  onBootstrapCrossSigning: () => void;
  onEnableKeyBackup: () => void;
  onChooseRoomKeyExportDestination: () => Promise<string | null>;
  onChooseRoomKeyImportSource: () => Promise<string | null>;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onBootstrapSecureBackup: (passphrase: string | null, intent: SecureBackupSetupIntent) => void;
  onChangeSecureBackupPassphrase: (oldSecret: string, newPassphrase: string) => void;
  onSaveSecureBackupRecoveryKey?: (revealRequestId: number) => Promise<void>;
  onConfirmSecureBackupRecoveryKeySaved?: (revealRequestId: number) => void | Promise<void>;
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
  /** @deprecated Account selection belongs to the persistent account tabs. */
  onSwitchAccount?: (session: SavedSessionInfo) => void;
  onLoadAccountManagementCapabilities: () => void;
  onRefreshCurrentSessionStatus?: (trigger: SessionStatusRefreshCommandTrigger) => void;
  canRestartSync?: boolean;
  onRestartSync?: () => void;
  settingsBusy?: boolean;
  runtimeAlerts?: RuntimeAlert[];
  runtimeAlertRetrying?: boolean;
  onRetryRuntimeAlert?: (kind: RuntimeAlert["kind"]) => void;
  onChangePassword: (newPassword: string) => void;
  onDeactivateAccount: (eraseData: boolean) => void;
  onSubmitAccountManagementUia: (flowId: number, password: string) => void;
  onStartCrawlRoom?: (roomId: string) => void;
  onStopCrawlRoom?: (roomId: string) => void;
  onDisplayDensityChange?: (density: DisplayDensity) => void;
  accountManagementUrl?: string | null;
  onManageAccount?: () => void;
  accountNotifications?: AccountNotificationsState;
  accountNotificationActions?: AccountNotificationActions;
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
      onRefreshCurrentSessionStatus("open");
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
  // macOS hides on close unconditionally (overview.md, "Desktop Window
  // Lifecycle And Tray"), so the setting has nothing to control there.
  const closeToTrayIsConfigurable = platform !== "macos";
  const isSaving = settings.persistence.kind === "saving";
  const [displayNameDraft, setDisplayNameDraft] = useState(profile.own.display_name ?? "");
  const visibleCategories = settingsCategories.filter((category) =>
    settingsScope === "app"
      ? ["appearance", "notifications", "preferences", "keyboard", "search", "help"].includes(category.id)
      : ["account", "sessions", "notifications", "privacy", "encryption", "search"].includes(category.id)
  );
  const initialVisibleCategory = visibleCategories.some((category) => category.id === initialCategory)
    ? initialCategory
    : visibleCategories[0]?.id ?? initialCategory;
  const [activeCategory, setActiveCategory] = useState<SettingsCategoryId>(initialVisibleCategory);
  const contentRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!visibleCategories.some((category) => category.id === activeCategory)) {
      setActiveCategory(visibleCategories[0]?.id ?? initialCategory);
    }
  }, [activeCategory, initialCategory, settingsScope, visibleCategories]);
  useEffect(() => { if (contentRef.current) contentRef.current.scrollTop = 0; }, [activeCategory]);
  // Opening the Notifications page re-reads the account's server-owned
  // notification settings (read-only; it never writes rules or pushers), so
  // changes made in other clients are reflected on every visit.
  const notificationsOwner =
    activeCategory === "notifications" && currentSession ? sessionKey(currentSession) : null;
  const loadAccountNotifications = accountNotificationActions.load;
  useEffect(() => {
    if (notificationsOwner) {
      loadAccountNotifications();
    }
  }, [notificationsOwner, loadAccountNotifications]);
  const avatarInputRef = useRef<HTMLInputElement | null>(null);
  const profileBusy = profile.update.kind !== "idle";
  const displayNameBusy = profile.update.kind === "settingDisplayName";
  const avatarBusy = profile.update.kind === "settingAvatar";
  const profileAvatarUrl = avatarSourceUrl(profile.own.avatar);
  const profileInitial = avatarInitial(
    profile.own.display_name?.trim() || currentSession?.user_id
  );
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
    <section
      className="settings-panel user-settings-panel"
      aria-label={t(settingsScope === "app" ? "settings.appSettings" : "settings.accountSettings")}
    >
      <nav className="settings-category-list" role="tablist" aria-label={t("settings.categories")} aria-orientation="vertical">
        <div className="settings-scope-heading">
          <strong>{t(settingsScope === "app" ? "settings.appSettings" : "settings.accountSettings")}</strong>
          {settingsScope !== "app" && currentSession ? (
            <div className="settings-account-owner">
              <span className="settings-account-owner-avatar" aria-hidden="true">
                {profileAvatarUrl ? <img src={profileAvatarUrl} /> : profileInitial}
              </span>
              <span>{currentSession.user_id}</span>
            </div>
          ) : null}
        </div>
        {visibleCategories.map((category, index) => (
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
              const next = event.key === "ArrowDown" ? (index + 1) % visibleCategories.length
                : event.key === "ArrowUp" ? (index + visibleCategories.length - 1) % visibleCategories.length
                : event.key === "Home" ? 0 : event.key === "End" ? visibleCategories.length - 1 : null;
              if (next !== null) {
                event.preventDefault();
                setActiveCategory(visibleCategories[next].id);
                document.getElementById(`settings-tab-${visibleCategories[next].id}`)?.focus();
              }
            }}
          >{t(category.label)}</button>
        ))}
        {onSettingsScopeChange ? (
          <button
            className="settings-scope-switch"
            type="button"
            disabled={settingsScope === "app" && !currentSession}
            onClick={() => onSettingsScopeChange(settingsScope === "app" ? "account" : "app")}
          >
            {t(settingsScope === "app" ? "settings.accountSettings" : "settings.appSettings")}
          </button>
        ) : null}
      </nav>
      <div className="settings-category-content" ref={contentRef}>
        <div id="settings-page-account" role="tabpanel" aria-labelledby="settings-tab-account" className="settings-category" hidden={activeCategory !== "account"} tabIndex={0}>
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
                label={t("sessionStatus.authentication")}
                value={currentSessionAuthenticationLabel(currentSessionDetails?.authentication_method)}
              />
              <DetailRow
                label={t("sessionStatus.sync")}
                value={currentSessionSyncLabel(currentSessionDetails?.sync_state)}
              />
              <DetailRow
                label={t("sessionStatus.title")}
                value={currentSessionCheckLabel(currentSessionStatus)}
              />
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
              <DetailRow
                label={t("sessionStatus.lastChecked")}
                value={currentSessionDetails
                  ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" })
                      .format(currentSessionDetails.checked_at_ms)
                  : t("sessionStatus.unavailable")}
              />
              <DetailRow label={t("settings.localStoreLabel")} value={t("settings.localStore")} />
            </div>
            {currentSessionStatus.status === "failed" ? (
              <p className="session-status-failure">
                {currentSessionFailureLabel(currentSessionStatus.kind)}
              </p>
            ) : null}
            {runtimeAlerts.length ? (
              <section className="runtime-alerts" aria-label={t("sessionStatus.runtimeWarnings")}>
                <h4>{t("sessionStatus.runtimeWarnings")}</h4>
                <ul>
                  {runtimeAlerts.map((alert) => (
                    <li key={alert.kind} data-runtime-alert-severity={alert.severity}>
                      <strong>{alert.title}</strong>
                      <p>{alert.detail}</p>
                      {alert.retryable ? (
                        <button
                          type="button"
                          disabled={runtimeAlertRetrying}
                          onClick={() => onRetryRuntimeAlert(alert.kind)}
                        >
                          {alert.kind === "secureBackup" ? t("gate.secureBackupRetry") : t("sessionStatus.retry")}
                        </button>
                      ) : null}
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}
            <div className="profile-settings-actions">
              <button
                className="profile-settings-action"
                type="button"
                disabled={!currentSession || currentSessionStatus.status === "checking"}
                onClick={() => onRefreshCurrentSessionStatus("manual")}
              >
                <span>{currentSessionStatus.status === "checking"
                  ? t("sessionStatus.checking")
                  : currentSessionStatus.status === "failed"
                    ? t("sessionStatus.retry")
                    : t("sessionStatus.recheck")}</span>
              </button>
              {canRestartSync ? (
                <button
                  className="profile-settings-action"
                  type="button"
                  disabled={!currentSession || settingsBusy}
                  onClick={onRestartSync}
                >
                  <span>{t("action.restartSync")}</span>
                </button>
              ) : null}
              <button
                className="profile-settings-action"
                type="button"
                disabled={!currentSession?.device_id}
                onClick={() => {
                  if (currentSession?.device_id) {
                    void navigator.clipboard?.writeText(currentSession.device_id);
                  }
                }}
              >
                <span>{t("sessionStatus.copyDeviceId")}</span>
              </button>
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
          <section className="settings-section" aria-label={t("settings.language")}>
            <LanguageControls selectedLocale={selectedLocale} onUpdateSettings={onUpdateSettings} />
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
        </div>
        <div id="settings-page-notifications" role="tabpanel" aria-labelledby="settings-tab-notifications" className="settings-category" hidden={activeCategory !== "notifications"} tabIndex={0}>
          <section id="settings-notifications" className="settings-section" aria-label={t("settings.notifications")}>
            <div className="settings-section-heading">
              <h3>{t("settings.notifications")}</h3>
              {isSaving ? <span className="settings-save-state">{t("settings.saving")}</span> : null}
            </div>
            <h4 className="settings-subheading">{t("settings.notificationsThisDevice")}</h4>
            <div className="settings-toggle-list">
              {settingsScope !== "app" ? (
                <NotificationSettingToggle
                  label={t("settings.notificationDesktop")}
                  description={t("settings.notificationDesktopDescription")}
                  settingKey="desktop_notifications"
                  current={selectedNotifications}
                  onSelect={onUpdateSettings}
                  icon={<Bell size={15} aria-hidden="true" />}
                />
              ) : null}
              {settingsScope !== "account" ? (
                <>
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
                </>
              ) : null}
              {settingsScope !== "app" ? (
                <NotificationSettingToggle
                  label={t("settings.notificationMessagePreviews")}
                  description={t("settings.notificationMessagePreviewsDescription")}
                  settingKey="message_previews"
                  current={selectedNotifications}
                  onSelect={onUpdateSettings}
                  icon={<Bell size={15} aria-hidden="true" />}
                />
              ) : null}
            </div>
          </section>
          {settingsScope !== "app" && currentSession ? (
            <>
              <AccountNotificationsLoadStatus
                state={accountNotifications}
                onRetry={accountNotificationActions.load}
              />
              <EmailNotificationsSection
                state={accountNotifications}
                actions={accountNotificationActions}
                syncKey={sessionKey(currentSession)}
                accountManagementAvailable={Boolean(accountManagementUrl)}
                onManageAccount={onManageAccount}
              />
              <NotificationCategoriesSection
                state={accountNotifications}
                actions={accountNotificationActions}
              />
            </>
          ) : null}
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
              secureBackupGate={secureBackupGate}
              localEncryption={localEncryption}
              platform={platform}
              onBootstrapSecureBackup={onBootstrapSecureBackup}
              onChangeSecureBackupPassphrase={onChangeSecureBackupPassphrase}
              onChooseRoomKeyExportDestination={onChooseRoomKeyExportDestination}
              onChooseRoomKeyImportSource={onChooseRoomKeyImportSource}
              onSaveSecureBackupRecoveryKey={onSaveSecureBackupRecoveryKey}
              onConfirmSecureBackupRecoveryKeySaved={onConfirmSecureBackupRecoveryKeySaved}
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
                settingsScope={settingsScope}
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

export function AppSettingsDialog({
  snapshot,
  onUpdateSettings,
  onClose
}: {
  snapshot: DesktopSnapshot;
  onUpdateSettings: (patch: SettingsPatch) => void;
  onClose: () => void;
}) {
  const domain = snapshot.state.domain;
  const noop = () => undefined;
  return (
    <ModalDialog
      title={t("settings.appSettings")}
      className="user-settings-modal"
      onClose={onClose}
    >
      <UserSettingsPanel
        initialCategory="appearance"
        settingsScope="app"
        currentSession={null}
        currentSessionStatus={domain.current_session_status}
        displayDensity={domain.settings.values.appearance.density}
        settings={domain.settings}
        searchCrawlerState={domain.search_crawler}
        onDisplayDensityChange={(density) => onUpdateSettings({
          appearance: { ...domain.settings.values.appearance, density }
        })}
        profile={domain.profile}
        e2eeTrust={domain.e2ee_trust}
        secureBackupGate={domain.secure_backup_gate}
        localEncryption={domain.local_encryption}
        platform={domain.locale_profile.platform}
        accountManagement={domain.account_management}
        accountManagementCapabilities={domain.account_management_capabilities}
        onUpdateSettings={onUpdateSettings}
        onSetDisplayName={noop}
        onSetAvatar={noop}
        onBootstrapCrossSigning={noop}
        onEnableKeyBackup={noop}
        onChooseRoomKeyExportDestination={async () => null}
        onChooseRoomKeyImportSource={async () => null}
        onExportRoomKeys={noop}
        onImportRoomKeys={noop}
        onBootstrapSecureBackup={noop}
        onChangeSecureBackupPassphrase={noop}
        onAcceptVerification={noop}
        onConfirmSasVerification={noop}
        onCancelVerification={noop}
        onResetIdentity={noop}
        onCancelIdentityReset={noop}
        onSubmitIdentityResetPassword={noop}
        onSubmitIdentityResetOAuth={noop}
        onProbeLocalEncryption={noop}
        onResetLocalData={noop}
        onLogout={noop}
        onOpenRecovery={noop}
        onLoadAccountManagementCapabilities={noop}
        onChangePassword={noop}
        onDeactivateAccount={noop}
        onSubmitAccountManagementUia={noop}
      />
    </ModalDialog>
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

function currentSessionAuthenticationLabel(method: string | undefined): string {
  switch (method) {
    case "password": return t("sessionStatus.authPassword");
    case "sso": return t("sessionStatus.authSso");
    case "oauth": return t("sessionStatus.authOauth");
    case "token": return t("sessionStatus.authToken");
    default: return t("sessionStatus.unknown");
  }
}

function currentSessionSyncLabel(sync: string | undefined): string {
  switch (sync) {
    case "running": return t("sessionStatus.syncRunning");
    case "starting": return t("sessionStatus.syncStarting");
    case "error": return t("sessionStatus.syncError");
    case "stopped": return t("sessionStatus.syncStopped");
    default: return t("sessionStatus.unavailable");
  }
}

function currentSessionCheckLabel(status: CurrentSessionStatusState): string {
  switch (status.status) {
    case "idle": return t("sessionStatus.notChecked");
    case "checking": return t("sessionStatus.checking");
    case "ready": return currentSessionVerificationLabel(status.details.verification);
    case "failed": return t("sessionStatus.failed");
  }
}

function currentSessionFailureLabel(kind: Extract<CurrentSessionStatusState, { status: "failed" }>["kind"]): string {
  switch (kind) {
    case "sdk": return t("sessionStatus.failureSdk");
    case "timed_out": return t("sessionStatus.failureTimedOut");
    case "unavailable": return t("sessionStatus.failureUnavailable");
    case "connectivity_unavailable": return t("sessionStatus.failureConnectivityUnavailable");
    case "authentication": return t("sessionStatus.failureAuthentication");
    case "network": return t("sessionStatus.failureNetwork");
    case "server": return t("sessionStatus.failureServer");
  }
}

function NotificationSettingToggle({
  label,
  description,
  settingKey,
  current,
  onSelect,
  icon
}: {
  label: string;
  description?: string;
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


function sessionKey(session: SavedSessionInfo): string {
  return `${session.homeserver}|${session.user_id}|${session.device_id}`;
}

function avatarSourceUrl(avatar: ProfileState["own"]["avatar"]): string | null {
  if (avatar?.thumbnail.kind !== "ready") {
    return null;
  }
  return renderableThumbnailSourceUrl(avatar.thumbnail.source_ref);
}

const defaultAccountNotificationsState: AccountNotificationsState = {
  load: { kind: "notLoaded" },
  snapshot: null,
  pending_email: null,
  operation: { kind: "idle" }
};
