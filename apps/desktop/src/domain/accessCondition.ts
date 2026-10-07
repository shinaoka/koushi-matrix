import type { MessageId } from "../i18n/messages";
import type { RestrictedConditions, RoomJoinRule } from "./types";

/**
 * #1166: the access-condition vocabulary shared by the room list, the room
 * header and the Space rail/header.
 *
 * The condition comes from Rust (`RoomListItem.access_join_rule` /
 * `SpaceSummary.join_rule`), which projects the object's own
 * `m.room.join_rules`. It is independent of encryption, DM status, the viewer's
 * membership and `can_join`, and it is never inferred here.
 *
 * A `null`/absent rule means "not projected yet" (or a row lane that has no
 * condition, such as an invitation), never a guessed rule.
 */
export type RoomAccessBadge = {
  /** Compact badge text, e.g. 参加条件有 / Conditions apply. */
  labelMessageId: MessageId;
  /** Explanation shown while that badge is hovered or focused. */
  descriptionMessageId: MessageId;
};

export type RoomAccessIndicator = {
  /** Public access uses a globe, invite-only a closed padlock; others none. */
  icon: "globe" | "padlock" | null;
  /** Full labels in display order; also the row/header's accessible text. */
  labelMessageIds: MessageId[];
  /** Compact badges in display order (conditions, then request). */
  badges: RoomAccessBadge[];
  /** Explanation for the whole condition: row/header hover or focus. */
  descriptionMessageId: MessageId;
};

/**
 * The indicator for a projected join rule.
 *
 * `unknown` also covers the reserved Matrix `private` value: the reference
 * vocabulary has no icon or badge for it, and the issue forbids presenting it as
 * a recognized condition.
 */
export function roomAccessIndicator(
  rule: RoomJoinRule | null | undefined,
  restricted?: RestrictedConditions | null
): RoomAccessIndicator | null {
  // #1166: a restricted rule with no usable allow condition still classifies as
  // restricted, but its explanation must say an invitation is required. An
  // allow-rule type the client does not model keeps the generic explanation:
  // the absence of a usable condition is not confirmed.
  const noUsableConditions = restricted === "noneUsable";
  switch (rule) {
    case "public":
      return {
        icon: "globe",
        labelMessageIds: ["access.public"],
        badges: [],
        descriptionMessageId: "access.publicDescription"
      };
    case "invite":
      return {
        icon: "padlock",
        labelMessageIds: ["access.inviteOnly"],
        badges: [],
        descriptionMessageId: "access.inviteOnlyDescription"
      };
    case "restricted":
      return {
        icon: null,
        labelMessageIds: ["access.conditionsApply"],
        badges: [
          {
            labelMessageId: "access.conditionsApply",
            descriptionMessageId: noUsableConditions
              ? "access.restrictedNoUsableConditionsDescription"
              : "access.conditionsDescription"
          }
        ],
        descriptionMessageId: noUsableConditions
          ? "access.restrictedNoUsableConditionsDescription"
          : "access.conditionsDescription"
      };
    case "knock":
      return {
        icon: null,
        labelMessageIds: ["access.canRequest"],
        badges: [
          {
            labelMessageId: "access.canRequest",
            descriptionMessageId: "access.requestDescription"
          }
        ],
        descriptionMessageId: "access.requestDescription"
      };
    case "knockRestricted":
      return {
        icon: null,
        labelMessageIds: ["access.conditionsApply", "access.canRequest"],
        // The two routes keep their own explanation, and focusing the row or
        // header explains both together.
        badges: [
          {
            labelMessageId: "access.conditionsApply",
            descriptionMessageId: noUsableConditions
              ? "access.restrictedNoUsableConditionsDescription"
              : "access.conditionsRouteDescription"
          },
          {
            labelMessageId: "access.canRequest",
            descriptionMessageId: "access.requestRouteDescription"
          }
        ],
        descriptionMessageId: noUsableConditions
          ? "access.restrictedNoUsableConditionsCanRequestDescription"
          : "access.knockRestrictedDescription"
      };
    case "private":
    case "unknown":
      return {
        icon: null,
        labelMessageIds: ["access.unknownFull"],
        badges: [
          {
            labelMessageId: "access.unknown",
            descriptionMessageId: "access.unknownDescription"
          }
        ],
        descriptionMessageId: "access.unknownDescription"
      };
    default:
      return null;
  }
}

/** The indicator a surface shows while the condition has not been projected. */
export const ROOM_ACCESS_CHECKING: RoomAccessIndicator = {
  icon: null,
  labelMessageIds: ["access.checkingFull"],
  badges: [
    {
      labelMessageId: "access.checking",
      descriptionMessageId: "access.checkingDescription"
    }
  ],
  descriptionMessageId: "access.checkingDescription"
};

/**
 * The badges a room/Space header shows: the full label of the condition, which
 * is the icon's own label for public/invite-only and the compact badges
 * otherwise. Each entry keeps the explanation for its own route.
 */
export function roomAccessHeaderBadges(
  indicator: RoomAccessIndicator
): RoomAccessBadge[] {
  // The header always uses the *full* label (参加条件不明 / 参加条件確認中 for
  // unknown and checking), while each label keeps its own route explanation when
  // the compact badge carries one.
  return indicator.labelMessageIds.map((labelMessageId) => {
    const badge = indicator.badges.find(
      (candidate) => candidate.labelMessageId === labelMessageId
    );
    return {
      labelMessageId,
      descriptionMessageId: badge?.descriptionMessageId ?? indicator.descriptionMessageId
    };
  });
}

/**
 * The projected access condition of a room in the sidebar model, across every
 * list a room can appear in. `null` means it was not projected.
 */
interface SidebarAccessRow {
  room_id: string;
  access_join_rule?: RoomJoinRule | null;
  access_restricted_conditions?: RestrictedConditions | null;
  access_allowed_room_names?: string[];
}

export function sidebarRoomAccess(
  sidebar: {
    space_rooms: readonly SidebarAccessRow[];
    global_dms: readonly SidebarAccessRow[];
    not_joined_space_rooms: readonly SidebarAccessRow[];
  },
  roomId: string
): {
  joinRule: RoomJoinRule | null;
  restricted: RestrictedConditions | null;
  allowedRoomNames: readonly string[];
} {
  const row = [
    ...sidebar.space_rooms,
    ...sidebar.global_dms,
    ...sidebar.not_joined_space_rooms
  ].find((item) => item.room_id === roomId);
  return {
    joinRule: row?.access_join_rule ?? null,
    restricted: row?.access_restricted_conditions ?? null,
    allowedRoomNames: row?.access_allowed_room_names ?? []
  };
}

/**
 * The compact summary a narrow Space-rail item shows at its avatar.
 *
 * The rail has no room for badges, so the five states collapse to one glyph.
 * None of them may read as public or invite-only when the rule is unknown or
 * still unavailable.
 */
export type RoomAccessRailSummary = "globe" | "padlock" | "info" | "question" | "loading";

export function roomAccessRailSummary(
  rule: RoomJoinRule | null | undefined
): RoomAccessRailSummary {
  switch (rule) {
    case "public":
      return "globe";
    case "invite":
      return "padlock";
    case "restricted":
    case "knock":
    case "knockRestricted":
      return "info";
    case "private":
    case "unknown":
      return "question";
    default:
      return "loading";
  }
}