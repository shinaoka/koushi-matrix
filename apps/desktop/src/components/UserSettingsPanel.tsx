import { Keyboard, RefreshCcw, ShieldCheck, SlidersHorizontal, UserRound } from "lucide-react";

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
          <h2 id="user-settings-title">User settings</h2>
          <p>{currentSession?.user_id ?? "Matrix account"}</p>
        </div>
      </header>

      <div className="settings-list">
        <button className="settings-list-item" type="button">
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <UserRound size={16} />
            </span>
            <span>General</span>
          </span>
        </button>
        <button className="settings-list-item" type="button">
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <ShieldCheck size={16} />
            </span>
            <span>Security & Privacy</span>
          </span>
        </button>
        <button className="settings-list-item" type="button" onClick={onOpenKeyboardSettings}>
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <Keyboard size={16} />
            </span>
            <span>Keyboard</span>
          </span>
        </button>
        <button className="settings-list-item" type="button">
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              <SlidersHorizontal size={16} />
            </span>
            <span>Preferences</span>
          </span>
        </button>
      </div>

      <section className="settings-section" aria-label="Session">
        <h3>Session</h3>
        <div className="settings-detail-list">
          <DetailRow label="Homeserver" value={currentSession?.homeserver ?? "Not restored"} />
          <DetailRow label="User ID" value={currentSession?.user_id ?? "Not restored"} />
          <DetailRow label="Device" value={currentSession?.device_id ?? "Not restored"} />
          <DetailRow label="Local store" value="Separate encrypted namespace" />
        </div>
      </section>

      <section className="settings-section" aria-label="Security">
        <h3>Security</h3>
        <div className="settings-detail-list">
          <DetailRow label="Session secret" value="OS credential store" />
          <DetailRow label="Search index" value="Encrypted local index" />
        </div>
      </section>

      <section className="account-switcher" aria-label="Account switcher">
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
                  <span>{isCurrent ? "Current" : "Switch"}</span>
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
