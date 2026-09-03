import { Check } from "lucide-react";
import type { ReactNode } from "react";

import { getActiveLocale, t } from "../../i18n/messages";
import type { PresenceKind } from "../../domain/types";

function formatMessageTimestamp(timestampMs: number | null): string | null {
  if (timestampMs === null) {
    return null;
  }
  return new Intl.DateTimeFormat(getActiveLocale(), {
    timeStyle: "short"
  }).format(new Date(timestampMs));
}

function presenceLabel(presence: PresenceKind): string {
  if (presence === "online") {
    return t("timeline.presenceOnline");
  }
  if (presence === "away") {
    return t("timeline.presenceAway");
  }
  return t("timeline.presenceOffline");
}

// ---------------------------------------------------------------------------
// MessageMeta: timestamp + send-state marks (extracted for testability, #83)
// ---------------------------------------------------------------------------

/**
 * Renders the heading-region metadata for a timeline message row:
 * sender label, timestamp, edited marker, send-state text labels, and the
 * sent checkmark. All data comes from Rust-owned DTO fields; no local
 * inference of send/edit state is performed here.
 */
export function MessageMeta({
  senderDisplayLabel,
  timestampMs,
  isEdited,
  isRedacted,
  sendStateKind,
  presence,
  onOpenSenderProfile
}: {
  senderDisplayLabel: string;
  timestampMs: number | null;
  isEdited: boolean;
  isRedacted: boolean;
  sendStateKind: string | null;
  presence?: import("../../domain/types").PresenceKind;
  onOpenSenderProfile?: () => void;
}): ReactNode {
  const messageTimestamp = formatMessageTimestamp(timestampMs);
  const sendStateLabel =
    sendStateKind === "sending"
      ? t("timeline.sending")
      : sendStateKind === "notSent"
        ? t("timeline.notSent")
        : sendStateKind === "cancelled"
          ? t("timeline.cancelledSend")
          : null;
  const sentStateMark =
    sendStateKind === "sent" ? (
      <span
        className="message-send-state"
        data-send-state="sent"
        aria-label={t("timeline.sent")}
      >
        <Check size={12} aria-hidden="true" />
      </span>
    ) : null;

  return (
    <>
      {presence ? (
        <span
          className="presence-dot message-presence"
          data-presence={presence}
          aria-label={presenceLabel(presence)}
        />
      ) : null}
      {onOpenSenderProfile ? (
        <button
          className="sender sender-profile-button"
          type="button"
          dir="auto"
          aria-label={t("people.openProfile", { name: senderDisplayLabel })}
          onClick={(event) => {
            event.stopPropagation();
            onOpenSenderProfile();
          }}
        >
          {senderDisplayLabel}
        </button>
      ) : (
        <span className="sender" dir="auto">{senderDisplayLabel}</span>
      )}
      {messageTimestamp ? (
        <time className="message-timestamp" dateTime={new Date(timestampMs!).toISOString()}>
          {messageTimestamp}
        </time>
      ) : null}
      {isEdited && !isRedacted ? (
        <span className="message-edited">{t("timeline.editedMessage")}</span>
      ) : null}
      {sendStateLabel ? (
        <span
          className="message-send-state"
          data-send-state={sendStateKind ?? undefined}
        >
          {sendStateLabel}
        </span>
      ) : null}
      {sentStateMark}
    </>
  );
}

export { formatMessageTimestamp };
