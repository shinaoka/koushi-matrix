import { useEffect, useState } from "react";
import { Bot, ShieldAlert } from "lucide-react";
import { api } from "../backend/appRuntime";
import type { AccountTabSummary, McpNeed, McpPendingApproval } from "../domain/types";
import { t } from "../i18n/messages";
import { ModalDialog } from "./ModalDialog";
import { mcpAccountLabel, mcpAccountTab, useMcpAssistantsState } from "./useMcpAssistantsState";

/** How long Allow ignores activation after a request appears (#1536), so a
 * keystroke or click meant for something else cannot approve it. */
export const MCP_ALLOW_ARMING_DELAY_MS = 1000;

/**
 * Asks the user about the oldest waiting MCP approval. The Rust broker owns the
 * request; closing the dialog leaves it pending until it expires, and the
 * answer is fenced by `request_id`.
 */
export function McpApprovalDialog({ accountTabs }: { accountTabs: readonly AccountTabSummary[] }) {
  const state = useMcpAssistantsState();
  const [dismissed, setDismissed] = useState<ReadonlySet<number>>(new Set());
  const pending = (state?.pending ?? []).filter(request => !dismissed.has(request.request_id));
  const current = pending[0];
  if (!current) return null;
  return (
    <McpApprovalPrompt
      key={current.request_id}
      request={current}
      accountTabs={accountTabs}
      moreWaiting={pending.length - 1}
      onClose={() => setDismissed(previous => new Set(previous).add(current.request_id))}
    />
  );
}

/** Whether a just-shown approval may be allowed yet (see the arming delay). */
export function useArmed(armingDelayMs: number): boolean {
  const [armed, setArmed] = useState(armingDelayMs <= 0);
  useEffect(() => {
    if (armingDelayMs <= 0) return;
    const timer = window.setTimeout(() => setArmed(true), armingDelayMs);
    return () => window.clearTimeout(timer);
  }, [armingDelayMs]);
  return armed;
}

export function mcpWantsText(need: McpNeed, account: string): string {
  return {
    read: t("mcp.wantsRead", { account }),
    send: t("mcp.wantsSend", { account }),
    attachments: t("mcp.wantsAttachments", { account }),
    message_actions: t("mcp.wantsMessageActions", { account }),
    draft: t("mcp.wantsDraft", { account })
  }[need];
}

export function mcpAllowLabel(need: McpNeed, account: string): string {
  return {
    read: t("mcp.allowRead", { account }),
    send: t("mcp.allowSend", { account }),
    attachments: t("mcp.allowAttachments", { account }),
    message_actions: t("mcp.allowMessageActions", { account }),
    draft: t("mcp.allowDraft", { account })
  }[need];
}

export function McpApprovalPrompt({
  request,
  accountTabs,
  moreWaiting,
  onClose,
  armingDelayMs = MCP_ALLOW_ARMING_DELAY_MS
}: {
  request: McpPendingApproval;
  accountTabs: readonly AccountTabSummary[];
  moreWaiting: number;
  onClose: () => void;
  armingDelayMs?: number;
}) {
  const armed = useArmed(armingDelayMs);
  const [busy, setBusy] = useState(false);
  const [saveFailed, setSaveFailed] = useState(false);

  const client = request.client_name;
  const tab = mcpAccountTab(request.account_id, accountTabs);
  const account = mcpAccountLabel(request.account_id, accountTabs) ?? t("settings.assistantsUnknownAccount");
  const title = {
    read: t("mcp.approveReadTitle", { client }),
    send: t("mcp.approveSendTitle", { client }),
    attachments: t("mcp.approveAttachmentsTitle", { client }),
    message_actions: t("mcp.approveMessageActionsTitle", { client }),
    draft: t("mcp.approveDraftTitle", { client })
  }[request.need];
  const wants = mcpWantsText(request.need, account);
  const allowLabel = mcpAllowLabel(request.need, account);

  async function answer(allow: boolean) {
    setBusy(true);
    setSaveFailed(false);
    try {
      const outcome = await api.answerMcpApproval(request.request_id, allow);
      if (outcome === "store_failed") setSaveFailed(true);
    } catch {
      setSaveFailed(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <ModalDialog title={title} className="mcp-approval-modal" onClose={onClose}>
      <div className="mcp-approval-content">
        <p className="mcp-approval-requested">{t("mcp.requestedBy")}</p>
        <div className="mcp-approval-client">
          <span className="mcp-approval-icon" aria-hidden="true"><Bot size={18} /></span>
          <span>
            <strong className="mcp-approval-client-name" dir="ltr">{client}</strong>
            {request.client_version
              ? <span className="mcp-approval-version">{t("settings.assistantsVersion", { version: request.client_version })}</span>
              : null}
            <span className="mcp-approval-wants">{wants}</span>
            {tab?.accountKey ? <span className="mcp-approval-account-id" dir="ltr">{tab.accountKey}</span> : null}
          </span>
        </div>

        {request.need === "read" ? (
          <>
            <p className="mcp-approval-heading">{t("mcp.readCan", { client })}</p>
            <ul className="mcp-approval-list">
              <li>{t("mcp.readCanRooms", { account })}</li>
              <li>{t("mcp.readCanMessages")}</li>
              <li>{t("mcp.readCanSearch", { account })}</li>
            </ul>
            <p className="mcp-approval-note">{t("mcp.readCannot")}</p>
          </>
        ) : null}

        {request.need === "send" ? (
          <>
            {request.waiting && request.message_preview !== null ? (
              <>
                <p className="mcp-approval-heading">
                  {request.room_label
                    ? t("mcp.sendPreviewHeadingRoom", { room: request.room_label })
                    : t("mcp.sendPreviewHeading")}
                </p>
                <p className="mcp-approval-preview">{request.message_preview}</p>
              </>
            ) : (
              <p className="mcp-approval-note">{t("mcp.sendNoLongerWaiting")}</p>
            )}
            <p className="mcp-approval-note">{t("mcp.sendExplain", { client, account })}</p>
          </>
        ) : null}

        {request.need === "draft" ? (
          <>
            {request.waiting && request.message_preview !== null ? (
              <>
                <p className="mcp-approval-heading">
                  {request.room_label
                    ? t("mcp.draftPreviewHeadingRoom", { room: request.room_label })
                    : t("mcp.draftPreviewHeading")}
                </p>
                <p className="mcp-approval-preview">{request.message_preview}</p>
              </>
            ) : (
              <p className="mcp-approval-note">{t("mcp.draftNoLongerWaiting")}</p>
            )}
            <p className="mcp-approval-note">{t("mcp.draftExplain", { client, account })}</p>
          </>
        ) : null}

        {request.need === "attachments" ? (
          <>
            {request.room_label ? <p className="mcp-approval-note">{t("mcp.attachmentsRoom", { room: request.room_label })}</p> : null}
            <p className="mcp-approval-note">{t("mcp.attachmentsExplain", { client, account })}</p>
          </>
        ) : null}

        {request.need === "message_actions" ? (
          <>
            <p className="mcp-approval-heading">{t("mcp.messageActionsCan", { client })}</p>
            <ul className="mcp-approval-list">
              <li>{t("mcp.messageActionsCanReact")}</li>
              <li>{t("mcp.messageActionsCanEdit")}</li>
              <li>{t("mcp.messageActionsCanDelete")}</li>
            </ul>
            <p className="mcp-approval-note">{t("mcp.messageActionsCannot", { account })}</p>
          </>
        ) : null}

        <p className="mcp-approval-warning">
          <ShieldAlert size={16} aria-hidden="true" />
          <span>{t("mcp.selfDeclared", { client })}</span>
        </p>
        {saveFailed ? <p role="alert" className="mcp-approval-error">{t("mcp.saveFailed")}</p> : null}
        {request.waiting_calls > 1
          ? <p className="mcp-approval-more">{t("mcp.moreMessagesWaiting", { count: String(request.waiting_calls - 1) })}</p>
          : null}
        {moreWaiting > 0 ? <p className="mcp-approval-more">{t("mcp.moreWaiting", { count: String(moreWaiting) })}</p> : null}

        <div className="dialog-actions">
          <button type="button" className="dialog-button" disabled={busy} onClick={() => void answer(false)}>
            {t("mcp.deny")}
          </button>
          <button
            type="button"
            className="dialog-button is-primary"
            disabled={busy || !armed}
            onClick={() => void answer(true)}
          >
            {allowLabel}
          </button>
        </div>
      </div>
    </ModalDialog>
  );
}
