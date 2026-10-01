import { t } from "../i18n/messages";
import type {
  InvitePreview,
  RoomListItem,
  RoomNamePlaceholder,
  RoomSummary,
  SpaceChildSummary
} from "./types";

function roomNamePlaceholderText(placeholder: RoomNamePlaceholder): string {
  switch (placeholder.kind) {
    case "empty":
      return t("room.namePlaceholderEmpty");
    case "emptyWas":
      return t("room.namePlaceholderEmptyWas", { previousNames: placeholder.previous_names });
  }
}

/**
 * A room's people-facing name (#1050). Rust marks the SDK's English
 * calculated empty-room name structurally; that case renders catalog text and
 * every other label stays caller data.
 */
export function roomDisplayLabel(
  room: Pick<RoomSummary, "display_label" | "display_label_placeholder">
): string {
  return room.display_label_placeholder
    ? roomNamePlaceholderText(room.display_label_placeholder)
    : room.display_label;
}

/** `roomDisplayLabel` for a Rust sidebar row. */
export function roomListItemLabel(
  room: Pick<RoomListItem, "display_name" | "display_name_placeholder">
): string {
  return room.display_name_placeholder
    ? roomNamePlaceholderText(room.display_name_placeholder)
    : room.display_name;
}

/** The localized people-facing name shown for an invite preview. */
export function invitePreviewLabel(
  invite: Pick<InvitePreview, "display_name" | "display_name_placeholder">
): string {
  return invite.display_name_placeholder
    ? roomNamePlaceholderText(invite.display_name_placeholder)
    : invite.display_name;
}

/** `roomDisplayLabel` for a Space child the account has not joined (#1070). */
export function spaceChildLabel(
  child: Pick<SpaceChildSummary, "display_name" | "display_name_placeholder">
): string {
  return child.display_name_placeholder
    ? roomNamePlaceholderText(child.display_name_placeholder)
    : child.display_name;
}
