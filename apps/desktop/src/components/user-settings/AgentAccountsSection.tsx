import { useState } from "react";
import { Bot } from "lucide-react";
import type {
  AccountTabSummary,
  AgentAccounts,
  MentionAllowedSenders,
  SettingsPatch
} from "../../domain/types";
import { t } from "../../i18n/messages";
import { accountTabLabel } from "../accountSignOut";
import { ImeSafeForm, ImeTextField } from "../ImeTextControl";

const MATRIX_USER_ID = /^@[^\s:@]+:[^\s:@]+$/;

/** Signed-in accounts as `[user id, label]`; the derived default of the allow-list. */
function signedInAccounts(tabs: readonly AccountTabSummary[]): [string, string][] {
  const seen = new Set<string>();
  const accounts: [string, string][] = [];
  for (const tab of tabs) {
    if (!tab.accountKey || seen.has(tab.accountKey)) continue;
    seen.add(tab.accountKey);
    accounts.push([tab.accountKey, accountTabLabel(tab)]);
  }
  return accounts;
}

/**
 * Settings -> AI assistants -> Agent accounts (#2110). Marking an account is a
 * device setting kept by user id, so it survives signing that account out and in
 * again. Rust consults it for notifications, badges and the tab strip; this only
 * edits it.
 */
export function AgentAccountsSection({
  agentAccounts,
  accountTabs,
  isSaving,
  onUpdateSettings
}: {
  agentAccounts: AgentAccounts;
  accountTabs: readonly AccountTabSummary[];
  isSaving: boolean;
  onUpdateSettings: (patch: SettingsPatch) => void;
}) {
  const accounts = signedInAccounts(accountTabs);
  if (accounts.length === 0) return null;
  return (
    <section className="settings-assistants-group" aria-label={t("settings.agentAccounts")}>
      <h4>{t("settings.agentAccounts")}</h4>
      <p className="settings-assistants-note">{t("settings.agentAccountsDescription")}</p>
      {accounts.map(([userId, label]) => {
        const role = agentAccounts.accounts[userId];
        return (
          <div className="settings-agent-account" key={userId}>
            <button
              className="settings-toggle-row"
              type="button"
              role="switch"
              disabled={isSaving}
              aria-checked={role !== undefined}
              aria-label={t("settings.agentAccountSwitch", { account: label })}
              onClick={() =>
                onUpdateSettings({
                  scope: "app",
                  agent_account: { user_id: userId, role: role ? null : { hide_tab: false } }
                })
              }
            >
              <span className="settings-toggle-copy">
                <span className="settings-toggle-label">
                  <Bot size={15} aria-hidden="true" />
                  <span dir="auto">{t("settings.agentAccountSwitch", { account: label })}</span>
                </span>
                <span className="settings-toggle-description" dir="ltr">{userId}</span>
              </span>
              <span className="settings-switch-track" aria-hidden="true">
                <span className="settings-switch-thumb" />
              </span>
            </button>
            {role ? (
              <button
                className="settings-toggle-row"
                type="button"
                role="switch"
                disabled={isSaving}
                aria-checked={role.hide_tab}
                aria-label={t("settings.agentAccountHideTab", { account: label })}
                onClick={() =>
                  onUpdateSettings({
                    scope: "app",
                    agent_account: { user_id: userId, role: { hide_tab: !role.hide_tab } }
                  })
                }
              >
                <span className="settings-toggle-copy">
                  <span className="settings-toggle-label">
                    <span dir="auto">{t("settings.agentAccountHideTab", { account: label })}</span>
                  </span>
                  <span className="settings-toggle-description">{t("settings.agentAccountHideTabDescription")}</span>
                </span>
                <span className="settings-switch-track" aria-hidden="true">
                  <span className="settings-switch-thumb" />
                </span>
              </button>
            ) : null}
          </div>
        );
      })}
    </section>
  );
}

/**
 * Settings -> AI assistants -> Who the agent answers (#2110). The default is
 * derived (every signed-in account, so one added later is covered); the owner
 * adds other users and removes any, including a default. The core enforces the
 * list; this edits it through `mcp_allowed_senders`, which carries the whole
 * `added`/`removed` pair, and never widens it for anyone else.
 */
export function AllowedSendersSection({
  allowed,
  accountTabs,
  isSaving,
  onUpdateSettings
}: {
  allowed: MentionAllowedSenders;
  accountTabs: readonly AccountTabSummary[];
  isSaving: boolean;
  onUpdateSettings: (patch: SettingsPatch) => void;
}) {
  const [draft, setDraft] = useState("");
  const [invalid, setInvalid] = useState(false);
  const accounts = signedInAccounts(accountTabs);
  const derived = new Set(accounts.map(([userId]) => userId));
  const removed = new Set(allowed.removed);
  const explicit = allowed.added.filter(userId => !derived.has(userId));

  function save(next: MentionAllowedSenders) {
    onUpdateSettings({ scope: "app", mcp_allowed_senders: next });
  }
  function exclude(userId: string) {
    save({ added: allowed.added.filter(id => id !== userId), removed: [...allowed.removed, userId] });
  }
  function restore(userId: string) {
    save({ added: allowed.added, removed: allowed.removed.filter(id => id !== userId) });
  }
  function dropAdded(userId: string) {
    save({ added: allowed.added.filter(id => id !== userId), removed: allowed.removed });
  }
  function submit(event: { preventDefault(): void }) {
    event.preventDefault();
    const userId = draft.trim();
    if (!MATRIX_USER_ID.test(userId)) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    setDraft("");
    if (removed.has(userId)) restore(userId);
    else if (!allowed.added.includes(userId)) save({ added: [...allowed.added, userId], removed: allowed.removed });
  }

  return (
    <section className="settings-assistants-group" aria-label={t("settings.allowedSenders")}>
      <h4>{t("settings.allowedSenders")}</h4>
      <p className="settings-assistants-note">{t("settings.allowedSendersGuarantee")}</p>
      <p className="settings-assistants-note">{t("settings.allowedSendersDefault")}</p>
      <ul className="settings-allowed-senders">
        {accounts.map(([userId, label]) => (
          <li className="settings-allowed-sender" key={userId} data-implicit="true" data-excluded={removed.has(userId)}>
            <span className="settings-assistant-identity">
              <strong dir="auto">{label}</strong>
              <span className="settings-toggle-description" dir="ltr">{userId}</span>
              <span className="settings-toggle-description">
                {removed.has(userId) ? t("settings.allowedSendersExcluded") : t("settings.allowedSendersImplicit")}
              </span>
            </span>
            <button
              className="trust-action-button secondary"
              type="button"
              disabled={isSaving}
              onClick={() => (removed.has(userId) ? restore(userId) : exclude(userId))}
            >
              {removed.has(userId)
                ? t("settings.allowedSendersRestore", { user: userId })
                : t("settings.allowedSendersExclude", { user: userId })}
            </button>
          </li>
        ))}
        {explicit.map(userId => (
          <li className="settings-allowed-sender" key={userId} data-implicit="false">
            <span className="settings-assistant-identity">
              <strong dir="ltr">{userId}</strong>
              <span className="settings-toggle-description">{t("settings.allowedSendersAdded")}</span>
            </span>
            <button
              className="trust-action-button secondary"
              type="button"
              disabled={isSaving}
              onClick={() => dropAdded(userId)}
            >
              {t("settings.allowedSendersRemove", { user: userId })}
            </button>
          </li>
        ))}
        {removed.size > 0
          ? [...removed]
              .filter(userId => !derived.has(userId))
              .map(userId => (
                <li className="settings-allowed-sender" key={userId} data-implicit="false" data-excluded="true">
                  <span className="settings-assistant-identity">
                    <strong dir="ltr">{userId}</strong>
                    <span className="settings-toggle-description">{t("settings.allowedSendersExcluded")}</span>
                  </span>
                  <button
                    className="trust-action-button secondary"
                    type="button"
                    disabled={isSaving}
                    onClick={() => restore(userId)}
                  >
                    {t("settings.allowedSendersRestore", { user: userId })}
                  </button>
                </li>
              ))
          : null}
      </ul>
      <ImeSafeForm className="profile-settings-form" onSubmit={submit}>
        <label className="profile-settings-field">
          <span>{t("settings.allowedSendersAddLabel")}</span>
          <ImeTextField
            value={draft}
            syncKey="allowed-senders-add"
            disabled={isSaving}
            placeholder="@name:example.org"
            dir="ltr"
            onChange={event => {
              setDraft(event.currentTarget.value);
              setInvalid(false);
            }}
            data-testid="allowed-sender-input"
          />
        </label>
        {invalid ? (
          <p className="profile-settings-hint" role="alert">{t("settings.allowedSendersInvalid")}</p>
        ) : null}
        <div className="session-actions">
          <button
            className="trust-action-button primary"
            type="submit"
            disabled={isSaving || draft.trim().length === 0}
            data-testid="allowed-sender-add"
          >
            {t("settings.allowedSendersAdd")}
          </button>
        </div>
      </ImeSafeForm>
    </section>
  );
}
