import { useState } from "react";
import { Copy, KeyRound } from "lucide-react";
import { api } from "../../backend/appRuntime";
import { uiDateTimeFormat } from "../../domain/intlFormatCache";
import type {
  AccountTabSummary,
  McpCreateProfileOutcome,
  McpNeed,
  McpProfile,
  McpProfileGrants
} from "../../domain/types";
import { t, type MessageId } from "../../i18n/messages";
import { ImeSafeForm, ImeTextField } from "../ImeTextControl";
import { ModalDialog } from "../ModalDialog";
import { mcpAccountLabel } from "../useMcpAssistantsState";

const NEEDS: readonly McpNeed[] = ["read", "send", "attachments", "message_actions", "draft"];
const NEED_LABELS: Record<McpNeed, MessageId> = {
  read: "settings.assistantsRead",
  send: "settings.assistantsSend",
  attachments: "settings.assistantsAttachments",
  message_actions: "settings.assistantsMessageActions",
  draft: "settings.assistantsDraft"
};
const NO_GRANTS: McpProfileGrants = { read: false, send: false, attachments: false, message_actions: false, draft: false };

const STATUS_LABELS: Record<McpProfile["status"], MessageId> = {
  ready: "settings.agentProfileStatusReady",
  account_unavailable: "settings.agentProfileStatusAccountUnavailable",
  not_agent_account: "settings.agentProfileStatusNotAgentAccount",
  secret_missing: "settings.agentProfileStatusSecretMissing"
};

function failureMessage(reason: Extract<McpCreateProfileOutcome, { kind: "failed" }>["reason"]): string {
  switch (reason) {
    case "invalid_label":
      return t("settings.agentProfileFailedLabel");
    case "not_agent_account":
    case "account_unavailable":
      return t("settings.agentProfileFailedAccount");
    default:
      return t("settings.agentProfileFailedStore");
  }
}

function formatTime(ms: number): string {
  return uiDateTimeFormat({ dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));
}

/**
 * Settings -> AI assistants -> Agent profiles (#2132). Rust owns every fact
 * shown here (status, connection, lease, launch snippets, which accounts may be
 * chosen); this renders them and dispatches the owner's create and revoke
 * commands. A row appears or leaves only when Rust publishes it.
 */
export function AgentProfilesSection({
  profiles,
  agentAccounts,
  accountTabs
}: {
  profiles: readonly McpProfile[];
  /** The accounts Rust allows a profile to be bound to. */
  agentAccounts: readonly string[];
  accountTabs: readonly AccountTabSummary[];
}) {
  const accountName = (accountId: string) =>
    mcpAccountLabel(accountId, accountTabs) ?? accountId.replace(/^account:/, "");
  const [label, setLabel] = useState("");
  const [chosen, setChosen] = useState("");
  const [grants, setGrants] = useState<McpProfileGrants>(NO_GRANTS);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ kind: "created" } | { kind: "failed"; message: string } | null>(null);
  const account = agentAccounts.includes(chosen) ? chosen : (agentAccounts[0] ?? "");

  async function create(event: { preventDefault(): void }) {
    event.preventDefault();
    if (busy || !account || label.trim().length === 0) return;
    setBusy(true);
    setResult(null);
    try {
      const outcome = await api.createAgentProfile(label.trim(), account, grants);
      if (outcome.kind === "created") {
        setResult({ kind: "created" });
        setLabel("");
        setGrants(NO_GRANTS);
      } else {
        setResult({ kind: "failed", message: failureMessage(outcome.reason) });
      }
    } catch {
      setResult({ kind: "failed", message: failureMessage("io") });
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="settings-assistants-group" aria-label={t("settings.agentProfiles")}>
      <h4>{t("settings.agentProfiles")}</h4>
      <p className="settings-assistants-note">{t("settings.agentProfilesDescription")}</p>
      {profiles.length === 0 ? <p className="settings-assistants-note">{t("settings.agentProfilesNone")}</p> : null}
      {profiles.map(profile => (
        <ProfileRow key={profile.profile_id} profile={profile} accountName={accountName(profile.account_id)} />
      ))}
      {agentAccounts.length === 0 ? (
        <p className="settings-assistants-note">{t("settings.agentProfileNoAgentAccount")}</p>
      ) : null}
      <ImeSafeForm className="profile-settings-form" onSubmit={create}>
        <label className="profile-settings-field">
          <span>{t("settings.agentProfileLabelField")}</span>
          <ImeTextField
            value={label}
            syncKey="agent-profile-label"
            disabled={busy}
            maxLength={80}
            onChange={event => setLabel(event.currentTarget.value)}
            data-testid="agent-profile-label"
          />
        </label>
        <label className="profile-settings-field">
          <span>{t("settings.agentProfileAccountField")}</span>
          <select
            value={account}
            disabled={busy || agentAccounts.length === 0}
            onChange={event => setChosen(event.currentTarget.value)}
          >
            {agentAccounts.map(accountId => (
              <option key={accountId} value={accountId}>{accountName(accountId)}</option>
            ))}
          </select>
        </label>
        <fieldset className="settings-profile-grants">
          <legend>{t("settings.agentProfileGrantsLegend")}</legend>
          {NEEDS.map(need => (
            <label key={need} className="settings-profile-grant">
              <input
                type="checkbox"
                checked={grants[need]}
                disabled={busy}
                onChange={event => setGrants({ ...grants, [need]: event.currentTarget.checked })}
              />
              <span>{t(NEED_LABELS[need])}</span>
            </label>
          ))}
        </fieldset>
        <div className="session-actions">
          <button
            className="trust-action-button primary"
            type="submit"
            disabled={busy || !account || label.trim().length === 0}
          >
            {t("settings.agentProfileCreate")}
          </button>
        </div>
        <div role="status" aria-live="polite">
          {result?.kind === "created" ? (
            <p className="settings-assistants-note">{t("settings.agentProfileCreated")}</p>
          ) : null}
        </div>
        {result?.kind === "failed" ? (
          <p className="profile-settings-hint" role="alert">{result.message}</p>
        ) : null}
      </ImeSafeForm>
    </section>
  );
}

function ProfileRow({ profile, accountName }: { profile: McpProfile; accountName: string }) {
  const [client, setClient] = useState(profile.snippets[0]?.client);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [revokeFailed, setRevokeFailed] = useState(false);
  const snippet = profile.snippets.find(candidate => candidate.client === client) ?? profile.snippets[0];

  async function copy() {
    if (!snippet) return;
    try {
      if (!navigator.clipboard) throw new Error("clipboard unavailable");
      // The text is Rust's, copied as it is.
      await navigator.clipboard.writeText(snippet.text);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  async function revoke() {
    setConfirming(false);
    setBusy(true);
    setRevokeFailed(false);
    try {
      const outcome = await api.revokeAgentProfile(profile.profile_id);
      if (outcome === "failed") setRevokeFailed(true);
    } catch {
      setRevokeFailed(true);
    } finally {
      setBusy(false);
    }
  }

  const grantNames = NEEDS.filter(need => profile[need]).map(need => t(NEED_LABELS[need]));
  return (
    <article className="settings-assistant-card" aria-label={profile.label} data-status={profile.status}>
      <header className="settings-assistant-header">
        <span className="mcp-approval-icon" aria-hidden="true"><KeyRound size={16} /></span>
        <span className="settings-assistant-identity">
          <strong dir="auto">{profile.label}</strong>
          <span className="settings-toggle-description" dir="auto">{accountName}</span>
          <span className="settings-toggle-description">{grantNames.join(" · ")}</span>
        </span>
        <button
          type="button"
          className="profile-settings-action is-danger"
          disabled={busy}
          aria-label={t("settings.agentProfileRevokeLabel", { label: profile.label })}
          onClick={() => setConfirming(true)}
        >
          {t("settings.agentProfileRevoke")}
        </button>
      </header>
      <ul className="settings-profile-status">
        <li data-status-kind="status">{t(STATUS_LABELS[profile.status])}</li>
        <li>{profile.connected ? t("settings.agentProfileConnected") : t("settings.agentProfileNotConnected")}</li>
        <li>{profile.subscribed ? t("settings.agentProfileSubscribed") : t("settings.agentProfileNotSubscribed")}</li>
        <li>
          {profile.last_seen_ms !== null
            ? t("settings.agentProfileLastSeen", { time: formatTime(profile.last_seen_ms) })
            : t("settings.agentProfileNeverSeen")}
        </li>
        {profile.last_rejected_ms !== null ? (
          <li>{t("settings.agentProfileLastRejected", { time: formatTime(profile.last_rejected_ms) })}</li>
        ) : null}
      </ul>
      {revokeFailed ? <p className="profile-settings-hint" role="alert">{t("settings.agentProfileRevokeFailed")}</p> : null}
      {snippet ? (
        <div className="settings-profile-launch">
          <label className="profile-settings-field">
            <span>{t("settings.agentProfileClientField")}</span>
            <select
              value={snippet.client}
              onChange={event => {
                setClient(event.currentTarget.value as typeof client);
                setCopyState("idle");
              }}
            >
              {profile.snippets.map(candidate => (
                <option key={candidate.client} value={candidate.client}>{candidate.client_name}</option>
              ))}
            </select>
          </label>
          <p className="settings-assistants-note">{t("settings.agentProfileLaunchHint")}</p>
          <pre dir="ltr"><code>{snippet.text}</code></pre>
          <button type="button" className="profile-settings-action" onClick={() => void copy()}>
            <Copy size={14} aria-hidden="true" />
            {t("settings.assistantsCopy")}
          </button>
          <p role="status" className="settings-assistants-note">
            {copyState === "copied"
              ? t("settings.assistantsCopied")
              : copyState === "failed"
                ? t("settings.assistantsCopyFailed")
                : ""}
          </p>
        </div>
      ) : null}
      {confirming ? (
        <ModalDialog
          title={t("settings.agentProfileRevokeTitle", { label: profile.label })}
          className="mcp-approval-modal"
          onClose={() => setConfirming(false)}
        >
          <div className="mcp-approval-content">
            <p className="mcp-approval-note">{t("settings.agentProfileRevokeExplain")}</p>
            <div className="dialog-actions">
              <button type="button" className="dialog-button" onClick={() => setConfirming(false)}>
                {t("action.cancel")}
              </button>
              <button type="button" className="dialog-button is-danger" onClick={() => void revoke()}>
                {t("settings.agentProfileRevokeConfirm")}
              </button>
            </div>
          </div>
        </ModalDialog>
      ) : null}
    </article>
  );
}
