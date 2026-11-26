import { Check, Info } from "lucide-react";
import type { ReactNode } from "react";

import { t } from "../../i18n/messages";
import type { PresenceKind } from "../../domain/types";
import { useActiveLocaleKey } from "../../i18n/activeLocaleContext";
import { MessageTimestamp } from "./MessageTimestampLabel";
import { Tooltip } from "../Tooltip";

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
  onOpenSenderProfile,
  onOpenSenderDeviceAdvisory
}: {
  senderDisplayLabel: string;
  timestampMs: number | null;
  isEdited: boolean;
  isRedacted: boolean;
  sendStateKind: string | null;
  presence?: import("../../domain/types").PresenceKind;
  onOpenSenderProfile?: () => void;
  /**
   * #1494: present only when Rust lists this sender in
   * `sender_device_advisories.rooms[roomId]`; renders the muted mark.
   */
  onOpenSenderDeviceAdvisory?: () => void;
}): ReactNode {
  useActiveLocaleKey();
  const sendStateLabel =
    sendStateKind === "sending"
      ? t("timeline.sending")
      : sendStateKind === "notSent"
        ? t("timeline.notSent")
        : sendStateKind === "locked"
          ? t("timeline.sendWaiting")
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
      {onOpenSenderDeviceAdvisory ? (
        <Tooltip
          label={t("people.unverifiedDevices.tooltipBody", { name: senderDisplayLabel })}
          placement="above"
        >
          {(tooltipProps) => (
            <button
              className="sender-device-advisory"
              type="button"
              aria-label={t("people.unverifiedDevices.badgeLabel", { name: senderDisplayLabel })}
              {...tooltipProps}
              onClick={(event) => {
                event.stopPropagation();
                onOpenSenderDeviceAdvisory();
              }}
            >
              <Info size={11} aria-hidden="true" />
            </button>
          )}
        </Tooltip>
      ) : null}
      <MessageTimestamp timestampMs={timestampMs} />
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

export { formatMessageTimestamp } from "./messageTimestamp";
