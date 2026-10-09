import { t } from "../i18n/messages";
import type { RoomHistoryVisibility } from "./types";

/**
 * Issue #1177: the four history policies exposed through the shared
 * choice-and-detail editor, in the order every surface presents them. One
 * owner keeps the Room Info editor, the create dialog and their tests in step.
 */
export const HISTORY_VISIBILITY_OPTIONS: readonly RoomHistoryVisibility[] = [
  "worldReadable",
  "shared",
  "invited",
  "joined"
];

export function roomHistoryVisibilityLabel(visibility: RoomHistoryVisibility): string {
  switch (visibility) {
    case "worldReadable":
      return t("room.historyWorldReadable");
    case "shared":
      return t("room.historyShared");
    case "invited":
      return t("room.historyInvited");
    case "joined":
      return t("room.historyJoined");
  }
}

export function roomHistoryVisibilityDescription(visibility: RoomHistoryVisibility): string {
  switch (visibility) {
    case "worldReadable":
      return t("room.historyWorldReadableDescription");
    case "shared":
      return t("room.historySharedDescription");
    case "invited":
      return t("room.historyInvitedDescription");
    case "joined":
      return t("room.historyJoinedDescription");
  }
}
