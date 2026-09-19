import { Copy, XCircle } from "lucide-react";
import { useCallback } from "react";
import { t } from "../../i18n/messages";
import type { TimelineMegolmSessionReason, TimelineMessageSource } from "../../domain/coreEvents";
import { writeClipboardText } from "./TimelineMessageBody";
import { FloatingLayer } from "../floatingLayer";

export function MessageSourceDialog({
  source,
  onClose
}: {
  source: TimelineMessageSource;
  onClose: () => void;
}) {
  const sourceJson = messageSourceJson(source);
  const sourceText = JSON.stringify(sourceJson, null, 2);
  const copyEventId = useCallback(() => {
    void writeClipboardText(source.event_id);
  }, [source.event_id]);
  const copySource = useCallback(() => {
    void writeClipboardText(sourceText);
  }, [sourceText]);
  const copyMegolmSessionFingerprint = useCallback(() => {
    if (source.megolm_session_fingerprint) {
      void writeClipboardText(source.megolm_session_fingerprint);
    }
  }, [source.megolm_session_fingerprint]);

  return (
    <FloatingLayer><div
      className="message-source-dialog"
      role="dialog"
      aria-label={t("timeline.messageSource")}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.nativeEvent.isComposing) {
          event.preventDefault(); event.stopPropagation(); onClose();
        }
      }}
    >
      <div className="message-source-dialog-header">
        <span>{t("timeline.messageSource")}</span>
        <button
          className="message-source-close"
          type="button"
          aria-label={t("timeline.closeMessageSource")}
          onClick={onClose}
        >
          <XCircle size={15} aria-hidden="true" />
        </button>
      </div>
      <div className="message-source-event-id">
        <span>{t("timeline.sourceEventId")}</span>
        <code>{source.event_id}</code>
        <button
          className="message-source-copy"
          type="button"
          aria-label={t("timeline.copyEventId")}
          onClick={copyEventId}
        >
          <Copy size={15} aria-hidden="true" />
          <span>{t("timeline.copyEventId")}</span>
        </button>
      </div>
      {source.megolm_session_fingerprint || source.megolm_message_index != null ? (
        <section className="message-source-encryption" aria-label={t("timeline.encryptionDetails")}>
          <h3>{t("timeline.encryptionDetails")}</h3>
          {source.megolm_session_fingerprint ? (
            <div className="message-source-encryption-row">
              <span>{t("timeline.megolmSessionFingerprint")}</span>
              <code>{source.megolm_session_fingerprint}</code>
              <button
                className="message-source-copy"
                type="button"
                aria-label={t("timeline.copyMegolmSessionFingerprint")}
                onClick={copyMegolmSessionFingerprint}
              >
                <Copy size={15} aria-hidden="true" />
                <span>{t("timeline.copyMegolmSessionFingerprint")}</span>
              </button>
            </div>
          ) : null}
          {source.megolm_message_index != null ? (
            <div className="message-source-encryption-row">
              <span>{t("timeline.megolmMessageIndex")}</span>
              <code>{source.megolm_message_index}</code>
            </div>
          ) : null}
          {source.megolm_session_rotation_reason ? (
            <div className="message-source-encryption-row">
              <span>{t("timeline.megolmRotationReason")}</span>
              <span>{megolmSessionReasonLabel(source.megolm_session_rotation_reason)}</span>
            </div>
          ) : null}
        </section>
      ) : null}
      <div className="message-source-section-header">
        <h3>{t("timeline.originalEventSource")}</h3>
        <button
          className="message-source-copy"
          type="button"
          aria-label={t("timeline.copyOriginalEventSource")}
          onClick={copySource}
        >
          <Copy size={15} aria-hidden="true" />
          <span>{t("timeline.copyOriginalEventSource")}</span>
        </button>
      </div>
      <pre className="message-source-json">
        <code>{sourceText}</code>
      </pre>
    </div></FloatingLayer>
  );
}

function megolmSessionReasonLabel(reason: TimelineMegolmSessionReason): string {
  switch (reason) {
    case "initial":
      return t("timeline.megolmReasonInitial");
    case "expiredTime":
      return t("timeline.megolmReasonExpiredTime");
    case "expiredMessageCount":
      return t("timeline.megolmReasonExpiredMessageCount");
    case "membershipOrDeviceChange":
      return t("timeline.megolmReasonMembershipOrDeviceChange");
    case "encryptionSettingsChanged":
      return t("timeline.megolmReasonEncryptionSettingsChanged");
    case "explicitDiscard":
      return t("timeline.megolmReasonExplicitDiscard");
    case "fullMemberListReload":
      return t("timeline.megolmReasonFullMemberListReload");
    case "roomSubscription":
      return t("timeline.megolmReasonRoomSubscription");
    case "limitedSyncResponse":
      return t("timeline.megolmReasonLimitedSyncResponse");
    case "keyShareFailure":
      return t("timeline.megolmReasonKeyShareFailure");
    case "storeMissing":
      return t("timeline.megolmReasonStoreMissing");
    case "invalidated":
      return t("timeline.megolmReasonInvalidated");
    case "unknown":
      return t("timeline.megolmReasonUnknown");
    case "notRetained":
      return t("timeline.megolmReasonNotRetained");
  }
}

function messageSourceJson(source: TimelineMessageSource): unknown {
  if (source.original_json && typeof source.original_json === "object") {
    return source.original_json;
  }

  const content: Record<string, unknown> = {};
  if (source.body) {
    content.body = source.body;
    content.msgtype = source.has_media ? "m.file" : "m.text";
  }
  if (source.in_reply_to_event_id) {
    content["m.relates_to"] = {
      "m.in_reply_to": {
        event_id: source.in_reply_to_event_id
      }
    };
  }
  if (source.thread_root) {
    content["m.relates_to"] = {
      ...(typeof content["m.relates_to"] === "object" && content["m.relates_to"] !== null
        ? (content["m.relates_to"] as Record<string, unknown>)
        : {}),
      rel_type: "m.thread",
      event_id: source.thread_root
    };
  }

  return {
    content,
    event_id: source.event_id,
    origin_server_ts: source.timestamp_ms,
    sender: source.sender,
    type: "m.room.message",
    unsigned: {
      redacted: source.is_redacted || undefined,
      edited: source.is_edited || undefined,
      media: source.has_media || undefined
    }
  };
}
