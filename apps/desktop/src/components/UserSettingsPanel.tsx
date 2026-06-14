import { Keyboard, RefreshCcw, ShieldCheck, SlidersHorizontal, UserRound } from "lucide-react";

import { t } from "../i18n/messages";
import type { SettingsPatch, SettingsState, ThemePreference, SavedSessionInfo } from "../domain/types";

export function UserSettingsPanel({
  currentSession,
  savedSessions,
  settings,
  onOpenKeyboardSettings,
  onUpdateSettings,
  onSwitchAccount
}: {
  currentSession: SavedSessionInfo | null;
  savedSessions: SavedSessionInfo[];
  settings: SettingsState;
  onOpenKeyboardSettings: () => void;
  onUpdateSettings: (patch: SettingsPatch) => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
}) {
  const selectedTheme = settings.values.appearance.theme;
  const isSaving = settings.persistence.kind === "saving";

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
      </section>

      <section className="settings-section" aria-label={t("settings.security")}>
        <h3>{t("settings.security")}</h3>
        <div className="settings-detail-list">
          <DetailRow label={t("settings.sessionSecretLabel")} value={t("settings.sessionSecret")} />
          <DetailRow label={t("settings.searchIndex")} value={t("settings.searchIndex")} />
        </div>
      </section>

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

function accountInitial(userId: string): string {
  return userId.replace(/^@/, "").charAt(0).toUpperCase() || "?";
}
