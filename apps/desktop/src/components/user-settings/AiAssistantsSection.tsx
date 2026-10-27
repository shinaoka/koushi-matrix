import { useState } from "react";
import { Bot, Copy, Play, Puzzle } from "lucide-react";
import { api } from "../../backend/appRuntime";
import { uiDateTimeFormat } from "../../domain/intlFormatCache";
import { ModalDialog } from "../ModalDialog";
import type {
  AccountTabSummary,
  McpExtensionOutcome,
  McpRegisterOutcome,
  McpAssistant,
  McpAssistantAccount,
  McpAssistantsState,
  McpNeed,
  McpPendingApproval,
  McpRegistration,
  McpSettings,
  SettingsPatch
} from "../../domain/types";
import { t } from "../../i18n/messages";
import { mcpAccountId, mcpAccountLabel, useMcpAssistantsState } from "../useMcpAssistantsState";
import { MCP_ALLOW_ARMING_DELAY_MS, mcpAllowLabel, mcpWantsText, useArmed } from "../McpApprovalDialog";

const NEEDS: readonly McpNeed[] = ["read", "send", "attachments", "message_actions"];

/** Product names of the clients; not translated. */
const CLIENT_NAMES: Record<Exclude<McpRegistration["client"], "other">, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  pi: "pi",
  claude_desktop: "Claude Desktop",
  deepseek_harness: "DeepSeek Harness"
};

function needLabel(need: McpNeed): string {
  return {
    read: t("settings.assistantsRead"),
    send: t("settings.assistantsSend"),
    attachments: t("settings.assistantsAttachments"),
    message_actions: t("settings.assistantsMessageActions")
  }[need];
}

/** App Settings -> AI assistants (#1536). Every decision is the Rust broker's;
 * this page renders its state and forwards the user's changes. */
export function AiAssistantsSection({
  settings,
  isSaving,
  accountTabs,
  onUpdateSettings
}: {
  settings: McpSettings;
  isSaving: boolean;
  accountTabs: readonly AccountTabSummary[];
  onUpdateSettings: (patch: SettingsPatch) => void;
}) {
  const state = useMcpAssistantsState();
  return (
    <>
      <button
        className="settings-toggle-row"
        type="button"
        role="switch"
        disabled={isSaving}
        aria-checked={settings.enabled}
        aria-label={t("settings.assistantsEnable")}
        onClick={() => onUpdateSettings({ scope: "app", mcp: { enabled: !settings.enabled } })}
      >
        <span className="settings-toggle-copy">
          <span className="settings-toggle-label">
            <Bot size={15} aria-hidden="true" />
            <span>{t("settings.assistantsEnable")}</span>
          </span>
          <span className="settings-toggle-description">{t("settings.assistantsEnableDescription")}</span>
        </span>
        <span className="settings-switch-track" aria-hidden="true">
          <span className="settings-switch-thumb" />
        </span>
      </button>
      <p className="settings-assistants-note">{t("settings.assistantsKeys")}</p>
      {state ? <AssistantsStatus state={state} /> : null}
      {state && state.pending.length > 0 ? <PendingApprovals pending={state.pending} accountTabs={accountTabs} /> : null}
      {state ? <ConnectedAssistants assistants={state.assistants} accountTabs={accountTabs} /> : null}
      {state && state.registrations.length > 0 ? <ConnectAssistant registrations={state.registrations} /> : null}
    </>
  );
}

function AssistantsStatus({ state }: { state: McpAssistantsState }) {
  const messages: string[] = [];
  if (state.bridge === "start_failed") messages.push(t("settings.assistantsBridgeFailed"));
  if (state.store === "malformed" || state.store === "unreadable" || state.store === "unsupported_schema") {
    messages.push(t("settings.assistantsStoreUnreadable"));
  }
  if (state.store === "write_failed") messages.push(t("settings.assistantsStoreWriteFailed"));
  if (state.launcher.state === "unavailable") {
    messages.push({
      not_bundled: t("settings.assistantsLauncherMissing"),
      translocated: t("settings.assistantsLauncherTranslocated"),
      ephemeral: t("settings.assistantsLauncherEphemeral")
    }[state.launcher.reason]);
  }
  for (const client of state.outdated_launchers) {
    messages.push(t("settings.assistantsOutdated", { client }));
  }
  if (messages.length === 0) return null;
  return (
    <div className="settings-assistants-status" role="status">
      {messages.map(message => <p key={message}>{message}</p>)}
    </div>
  );
}

/** Approvals still open, so one whose dialog was closed can be answered here. */
function PendingApprovals({
  pending,
  accountTabs
}: {
  pending: readonly McpPendingApproval[];
  accountTabs: readonly AccountTabSummary[];
}) {
  return (
    <section className="settings-assistants-group" aria-label={t("settings.assistantsPending")}>
      <h4>{t("settings.assistantsPending")}</h4>
      {pending.map(request => (
        <PendingApprovalRow key={request.request_id} request={request} accountTabs={accountTabs} />
      ))}
    </section>
  );
}

function PendingApprovalRow({
  request,
  accountTabs
}: {
  request: McpPendingApproval;
  accountTabs: readonly AccountTabSummary[];
}) {
  const armed = useArmed(MCP_ALLOW_ARMING_DELAY_MS);
  const [busy, setBusy] = useState(false);
  const account = mcpAccountLabel(request.account_id, accountTabs) ?? t("settings.assistantsUnknownAccount");
  function answer(allow: boolean) {
    setBusy(true);
    void api.answerMcpApproval(request.request_id, allow).catch(() => undefined).finally(() => setBusy(false));
  }
  return (
    <div className="settings-assistant-pending">
      <span className="settings-assistant-identity">
        <strong dir="ltr">{request.client_name}</strong>
        <span className="settings-toggle-description">{mcpWantsText(request.need, account)}</span>
      </span>
      <button type="button" className="profile-settings-action" disabled={busy} onClick={() => answer(false)}>
        {t("mcp.deny")}
      </button>
      <button type="button" className="profile-settings-action" disabled={busy || !armed} onClick={() => answer(true)}>
        {mcpAllowLabel(request.need, account)}
      </button>
    </div>
  );
}

function ConnectedAssistants({
  assistants,
  accountTabs
}: {
  assistants: readonly McpAssistant[];
  accountTabs: readonly AccountTabSummary[];
}) {
  return (
    <section className="settings-assistants-group" aria-label={t("settings.assistantsConnected")}>
      <h4>{t("settings.assistantsConnected")}</h4>
      {assistants.length === 0 ? <p className="settings-assistants-note">{t("settings.assistantsNone")}</p> : null}
      {assistants.map(assistant => (
        <AssistantCard key={assistant.client_name} assistant={assistant} accountTabs={accountTabs} />
      ))}
    </section>
  );
}

function AssistantCard({
  assistant,
  accountTabs
}: {
  assistant: McpAssistant;
  accountTabs: readonly AccountTabSummary[];
}) {
  const [busy, setBusy] = useState(false);
  const client = assistant.client_name;
  const rows: McpAssistantAccount[] = [...assistant.accounts];
  const unrequested = accountTabs.filter(tab => {
    const accountId = mcpAccountId(tab);
    return accountId !== null && !rows.some(row => row.account_id === accountId);
  });
  const details = [
    t("settings.assistantsSelfDeclared"),
    assistant.client_version ? t("settings.assistantsVersion", { version: assistant.client_version }) : null,
    assistant.last_used_ms !== null
      ? t("settings.assistantsLastUsed", {
          time: uiDateTimeFormat({ dateStyle: "medium", timeStyle: "short" }).format(new Date(assistant.last_used_ms))
        })
      : null
  ].filter((detail): detail is string => detail !== null);

  function run(operation: Promise<unknown>) {
    setBusy(true);
    void operation.catch(() => undefined).finally(() => setBusy(false));
  }

  return (
    <article className="settings-assistant-card" aria-label={client}>
      <header className="settings-assistant-header">
        <span className="mcp-approval-icon" aria-hidden="true"><Bot size={16} /></span>
        <span className="settings-assistant-identity">
          <strong dir="ltr">{client}</strong>
          <span className="settings-toggle-description">{details.join(" · ")}</span>
        </span>
        <button
          type="button"
          className="profile-settings-action is-danger"
          disabled={busy}
          aria-label={t("settings.assistantsRemoveLabel", { client })}
          onClick={() => run(api.removeMcpAssistant(client))}
        >
          {t("settings.assistantsRemove")}
        </button>
      </header>
      <div className="settings-assistant-grid" role="table" aria-label={client}>
        <div role="row" className="settings-assistant-row is-heading">
          <span role="columnheader">{t("settings.assistantsAccount")}</span>
          {NEEDS.map(need => <span role="columnheader" key={need}>{needLabel(need)}</span>)}
        </div>
        {rows.map(row => {
          const account = row.account_id.replace(/^account:/, "");
          return (
            <div role="row" className="settings-assistant-row" key={row.account_id}>
              <span role="rowheader" dir="ltr">{account}</span>
              {NEEDS.map(need => (
                <span role="cell" key={need}>
                  <PermissionSwitch
                    allowed={row[need]}
                    denied={row.denied.includes(need)}
                    label={t("settings.assistantsPermissionLabel", { permission: needLabel(need), account })}
                    disabled={busy}
                    onToggle={() => run(api.setMcpPermission(client, row.account_id, need, !row[need]))}
                  />
                </span>
              ))}
            </div>
          );
        })}
        {unrequested.map(tab => (
          <div role="row" className="settings-assistant-row" key={tab.id}>
            <span role="rowheader" dir="ltr">{tab.accountKey}</span>
            <span role="cell" aria-colspan={NEEDS.length} className="settings-assistant-unrequested">
              {t("settings.assistantsNotRequested", { client })}
            </span>
          </div>
        ))}
      </div>
    </article>
  );
}

function PermissionSwitch({
  allowed,
  denied,
  label,
  disabled,
  onToggle
}: {
  allowed: boolean;
  denied: boolean;
  label: string;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <span className="settings-assistant-permission">
      <button
        type="button"
        role="switch"
        className="settings-switch-button"
        aria-checked={allowed}
        aria-label={label}
        disabled={disabled}
        onClick={onToggle}
      >
        <span className="settings-switch-track" aria-hidden="true">
          <span className="settings-switch-thumb" />
        </span>
      </button>
      {denied && !allowed ? <span className="settings-assistant-denied">{t("settings.assistantsDenied")}</span> : null}
    </span>
  );
}

function clientLabel(registration: McpRegistration): string {
  return registration.client === "other" ? t("settings.assistantsClientOther") : CLIENT_NAMES[registration.client];
}

function ConnectAssistant({ registrations }: { registrations: readonly McpRegistration[] }) {
  const [selected, setSelected] = useState(registrations[0]?.client);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [confirming, setConfirming] = useState(false);
  const [running, setRunning] = useState(false);
  const [registerResult, setRegisterResult] = useState<{ client: string; outcome: McpRegisterOutcome } | null>(null);
  const [extensionResult, setExtensionResult] = useState<McpExtensionOutcome | null>(null);
  const current = registrations.find(registration => registration.client === selected) ?? registrations[0];
  if (!current) return null;
  const label = clientLabel(current);
  const instruction = current.kind === "command"
    ? t("settings.assistantsConnectCommand")
    : current.config_hint
      ? t("settings.assistantsConnectYaml", { file: current.config_hint })
      : t("settings.assistantsConnectJson");

  async function copy() {
    try {
      if (!navigator.clipboard) throw new Error("clipboard unavailable");
      await navigator.clipboard.writeText(current!.text);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  async function register() {
    setConfirming(false);
    setRunning(true);
    setRegisterResult(null);
    try {
      const outcome = await api.registerMcpClient(current!.client);
      setRegisterResult({ client: label, outcome });
    } catch {
      setRegisterResult({ client: label, outcome: { outcome: "failed", exit_code: null, output: "" } });
    } finally {
      setRunning(false);
    }
  }

  async function createExtension() {
    setRunning(true);
    setExtensionResult(null);
    try {
      setExtensionResult(await api.createClaudeDesktopExtension());
    } catch {
      setExtensionResult({ outcome: "write_failed" });
    } finally {
      setRunning(false);
    }
  }

  return (
    <section className="settings-assistants-group" aria-label={t("settings.assistantsConnect")}>
      <h4>{t("settings.assistantsConnect")}</h4>
      <div className="settings-assistants-tabs" role="tablist" aria-label={t("settings.assistantsClients")}>
        {registrations.map(registration => (
          <button
            key={registration.client}
            id={`settings-assistants-tab-${registration.client}`}
            type="button"
            role="tab"
            aria-controls="settings-assistants-snippet"
            aria-selected={registration.client === current.client}
            className={registration.client === current.client ? "is-selected" : ""}
            onClick={() => {
              setSelected(registration.client);
              setCopyState("idle");
              setRegisterResult(null);
              setExtensionResult(null);
            }}
          >
            {clientLabel(registration)}
          </button>
        ))}
      </div>
      <div
        id="settings-assistants-snippet"
        role="tabpanel"
        aria-labelledby={`settings-assistants-tab-${current.client}`}
      >
        {current.action === "run_command" ? (
          <div className="settings-assistants-action">
            <button type="button" className="profile-settings-action is-primary" disabled={running} onClick={() => setConfirming(true)}>
              <Play size={14} aria-hidden="true" />
              {running ? t("settings.assistantsRegisterRunning") : t("settings.assistantsRegister")}
            </button>
          </div>
        ) : null}
        {current.action === "create_extension" ? (
          <div className="settings-assistants-action">
            <p className="settings-assistants-note">{t("settings.assistantsExtensionExplain")}</p>
            <button type="button" className="profile-settings-action is-primary" disabled={running} onClick={() => void createExtension()}>
              <Puzzle size={14} aria-hidden="true" />
              {t("settings.assistantsCreateExtension")}
            </button>
          </div>
        ) : null}
        <div role="status" aria-live="polite">
          {registerResult ? <RegisterResult client={registerResult.client} outcome={registerResult.outcome} /> : null}
          {extensionResult ? <ExtensionResult outcome={extensionResult} /> : null}
        </div>
        <p className="settings-assistants-note">{instruction}</p>
        <div className="settings-assistants-snippet">
          <pre dir="ltr"><code>{current.text}</code></pre>
          <button type="button" className="profile-settings-action" onClick={() => void copy()}>
            <Copy size={14} aria-hidden="true" />
            {t("settings.assistantsCopy")}
          </button>
        </div>
        <p role="status" className="settings-assistants-note">
          {copyState === "copied" ? t("settings.assistantsCopied") : copyState === "failed" ? t("settings.assistantsCopyFailed") : ""}
        </p>
      </div>
      {confirming ? (
        <ModalDialog
          title={t("settings.assistantsRegisterTitle", { client: label })}
          className="mcp-approval-modal"
          onClose={() => setConfirming(false)}
        >
          <div className="mcp-approval-content">
            <p className="mcp-approval-note">{t("settings.assistantsRegisterExplain", { client: label })}</p>
            <pre className="mcp-register-command" dir="ltr"><code>{current.text}</code></pre>
            <div className="dialog-actions">
              <button type="button" className="dialog-button" onClick={() => setConfirming(false)}>
                {t("action.cancel")}
              </button>
              <button type="button" className="dialog-button is-primary" onClick={() => void register()}>
                {t("settings.assistantsRegisterRun")}
              </button>
            </div>
          </div>
        </ModalDialog>
      ) : null}
    </section>
  );
}

function RegisterResult({ client, outcome }: { client: string; outcome: McpRegisterOutcome }) {
  switch (outcome.outcome) {
    case "registered":
      return <p className="settings-assistants-result is-success">{t("settings.assistantsRegistered", { client })}</p>;
    case "not_found":
      return <p className="settings-assistants-result">{t("settings.assistantsRegisterNotFound", { program: outcome.program, client })}</p>;
    case "failed":
      return (
        <div className="settings-assistants-result">
          <p>
            {outcome.exit_code === null
              ? t("settings.assistantsRegisterFailedNoCode", { client })
              : t("settings.assistantsRegisterFailed", { client, code: String(outcome.exit_code) })}
          </p>
          {outcome.output ? <pre dir="ltr"><code>{outcome.output}</code></pre> : null}
        </div>
      );
    case "timed_out":
      return <p className="settings-assistants-result">{t("settings.assistantsRegisterTimedOut", { client })}</p>;
    case "in_progress":
      return <p className="settings-assistants-result">{t("settings.assistantsRegisterInProgress")}</p>;
    case "unsafe_arguments":
      return <p className="settings-assistants-result">{t("settings.assistantsRegisterUnsafe")}</p>;
    case "launcher_unavailable":
    case "unsupported":
      return <p className="settings-assistants-result">{t("settings.assistantsRegisterUnavailable")}</p>;
  }
}

function ExtensionResult({ outcome }: { outcome: McpExtensionOutcome }) {
  switch (outcome.outcome) {
    case "opened":
      return <p className="settings-assistants-result is-success">{t("settings.assistantsExtensionOpened", { path: outcome.path })}</p>;
    case "saved":
      return <p className="settings-assistants-result">{t("settings.assistantsExtensionSaved", { path: outcome.path })}</p>;
    case "launcher_unavailable":
      return <p className="settings-assistants-result">{t("settings.assistantsRegisterUnavailable")}</p>;
    case "unsupported":
    case "write_failed":
      return <p className="settings-assistants-result">{t("settings.assistantsExtensionFailed")}</p>;
  }
}
