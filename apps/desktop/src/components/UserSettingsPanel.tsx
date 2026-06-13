import { Keyboard, RefreshCcw, ShieldCheck, SlidersHorizontal, UserRound } from "lucide-react";

import { t } from "../i18n/messages";
import type { SavedSessionInfo } from "../domain/types";

export function UserSettingsPanel({
  currentSession,
  savedSessions,
  onOpenKeyboardSettings,
  onSwitchAccount
}: {
  currentSession: SavedSessionInfo | null;
  savedSessions: SavedSessionInfo[];
  onOpenKeyboardSettings: () => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
}) {
  return (
    <section className="settings-panel user-settings-panel" aria-labelledby="user-settings-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="user-settings-title">{t("panel.userSettings")}</h2>
          <p>{currentSession?.user_id ?? t("settings.matrixAccount")}</p>
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
          <DetailRow label="Local store" value={t("settings.localStore")} />
        </div>
      </section>

      <section className="settings-section" aria-label={t("settings.security")}>
        <h3>{t("settings.security")}</h3>
        <div className="settings-detail-list">
          <DetailRow label="Session secret" value={t("settings.sessionSecret")} />
          <DetailRow label="Search index" value={t("settings.searchIndex")} />
        </div>
      </section>

      <section className="account-switcher" aria-label={t("settings.accountSwitcher")}>
        <h3>Accounts</h3>
        <div className="account-switcher-list">
          {savedSessions.map((session) => {
            const isCurrent = sessionMatches(currentSession, session);
            return (
              <article className="account-switcher-row" key={sessionKey(session)}>
                <div className="account-switcher-avatar" aria-hidden="true">
                  {accountInitial(session.user_id)}
                </div>
                <div className="account-switcher-main">
                  <div className="account-switcher-user">{session.user_id}</div>
                  <div className="account-switcher-meta">
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
