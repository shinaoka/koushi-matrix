import type { MessageId } from "../i18n/messages";
import type { RestrictedConditions, RoomJoinRule } from "./types";

/**
 * #1166, #1220: the access-condition vocabulary shared by the room list, the
 * room header, the Space rail/header and the Room Info summary.
 *
 * The condition comes from Rust (`RoomListItem.access_join_rule` /
 * `SpaceRailItem.access_join_rule` / `access_restricted_conditions`), which
 * projects the object's own `m.room.join_rules`. It is independent of
 * encryption, DM status, the viewer's membership and `can_join`, and it is never
 * inferred here.
 *
 * A `null`/absent rule means "not projected yet" (or a row lane that has no
 * condition, such as an invitation), never a guessed rule.
 */
export type RoomAccessBadge = {
  /** Compact badge text, e.g. 参加条件有 / Conditions apply. */
  labelMessageId: MessageId;
  /** Explanation shown while that badge is hovered or focused. */
  descriptionMessageId: MessageId;
  /** Verified single-Space route name substituted into `{space}` (#1220). */
  descriptionSpaceName?: string;
  /** Named allow routes appended to the generic explanation. */
  descriptionAllowedRoomNames?: readonly string[];
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
  /** Verified single-Space route name substituted into `{space}` (#1220). */
  descriptionSpaceName?: string;
  /** Named allow routes appended to the generic explanation. */
  descriptionAllowedRoomNames?: readonly string[];
};

/**
 * The Rust-resolved route facts a surface hands the indicator: the verified
 * single-Space route name (#1220) and the named allow routes (#1166). Both are
 * already resolved by Rust; React never derives them.
 */
export type RoomAccessRouteFacts = {
  spaceMembersRoute?: string | null;
  allowedRoomNames?: readonly string[] | null;
};

/**
 * The indicator for a projected join rule.
 *
 * `unknown` also covers the reserved Matrix `private` value: the reference
 * vocabulary has no icon or badge for it, and the issue forbids presenting it as
 * a recognized condition. `private` is therefore never the Space-membership
 * route.
 */
export function roomAccessIndicator(
  rule: RoomJoinRule | null | undefined,
  restricted?: RestrictedConditions | null,
  route: RoomAccessRouteFacts = {}
): RoomAccessIndicator | null {
  // #1220: the specific "Space members can join" sentence applies only to a
  // verified membership-only route Rust resolved to a safe Space name. Every
  // other restricted shape keeps the generic facts, so several targets, an
  // ordinary-room target or an unknown/redacted type never claims it.
  const spaceName =
    restricted === "membershipOnly" ? route.spaceMembersRoute?.trim() || null : null;
  const namedRoutes = spaceName ? undefined : route.allowedRoomNames ?? undefined;
  // Only a confirmed empty allow list proves that no usable condition exists; an
  // unmodelled or uninspected rule keeps the generic explanation.
  const noUsableConditions = restricted === "confirmedEmpty";
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
      if (spaceName) {
        return {
          icon: null,
          labelMessageIds: ["access.spaceMembersCanJoin"],
          badges: [
            {
              labelMessageId: "access.spaceMembersCanJoin",
              descriptionMessageId: "access.spaceMembersCanJoinDescription",
              descriptionSpaceName: spaceName
            }
          ],
          descriptionMessageId: "access.spaceMembersCanJoinDescription",
          descriptionSpaceName: spaceName
        };
      }
      return {
        icon: null,
        labelMessageIds: ["access.conditionsApply"],
        badges: [
          {
            labelMessageId: "access.conditionsApply",
            descriptionMessageId: noUsableConditions
              ? "access.restrictedNoUsableConditionsDescription"
              : "access.conditionsDescription",
            descriptionAllowedRoomNames: noUsableConditions ? undefined : namedRoutes
          }
        ],
        descriptionMessageId: noUsableConditions
          ? "access.restrictedNoUsableConditionsDescription"
          : "access.conditionsDescription",
        descriptionAllowedRoomNames: noUsableConditions ? undefined : namedRoutes
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
      if (spaceName) {
        return {
          icon: null,
          labelMessageIds: ["access.spaceMembersCanJoin", "access.canRequest"],
          // The membership route names the Space; the request route keeps its own
          // badge and explanation.
          badges: [
            {
              labelMessageId: "access.spaceMembersCanJoin",
              descriptionMessageId: "access.spaceMembersCanJoinDescription",
              descriptionSpaceName: spaceName
            },
            {
              labelMessageId: "access.canRequest",
              descriptionMessageId: "access.requestRouteDescription"
            }
          ],
          descriptionMessageId: "access.spaceMembersCanJoinCanRequestDescription",
          descriptionSpaceName: spaceName
        };
      }
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
              : "access.conditionsRouteDescription",
            descriptionAllowedRoomNames: noUsableConditions ? undefined : namedRoutes
          },
          {
            labelMessageId: "access.canRequest",
            descriptionMessageId: "access.requestRouteDescription"
          }
        ],
        descriptionMessageId: noUsableConditions
          ? "access.restrictedNoUsableConditionsCanRequestDescription"
          : "access.knockRestrictedDescription",
        descriptionAllowedRoomNames: noUsableConditions ? undefined : namedRoutes
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
 * otherwise. Each entry keeps the explanation and route substitution for its
 * own route, falling back to the whole-condition explanation.
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
      descriptionMessageId: badge?.descriptionMessageId ?? indicator.descriptionMessageId,
      descriptionSpaceName: badge?.descriptionSpaceName ?? indicator.descriptionSpaceName,
      descriptionAllowedRoomNames:
        badge?.descriptionAllowedRoomNames ?? indicator.descriptionAllowedRoomNames
    };
  });
}

/**
 * #1249: the compact glyph a room-list access fact renders instead of a text
 * badge. The full localized label stays in the tooltip, the row description and
 * the room header/Room Info; the compact list never repeats the long badge.
 */
export type RoomAccessGlyph =
  | "globe"
  | "padlock"
  | "spaceMembers"
  | "conditions"
  | "request"
  | "unknown"
  | "checking";

/**
 * Maps a compact badge's label to its glyph. Public and invite-only use the
 * indicator's own `icon`, so only the badge labels reach here.
 */
export function roomAccessBadgeGlyph(labelMessageId: MessageId): RoomAccessGlyph {
  switch (labelMessageId) {
    case "access.spaceMembersCanJoin":
      return "spaceMembers";
    case "access.conditionsApply":
      return "conditions";
    case "access.canRequest":
      return "request";
    case "access.checking":
      return "checking";
    default:
      // `access.unknown` (and any future compact label without its own glyph)
      // stays a question mark rather than guessing a condition.
      return "unknown";
  }
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
  access_space_members_route?: string | null;
}

interface SidebarAccessLists {
  space_rooms: readonly SidebarAccessRow[];
  global_dms: readonly SidebarAccessRow[];
  not_joined_space_rooms: readonly SidebarAccessRow[];
  sections?: {
    favourites: readonly SidebarAccessRow[];
    rooms: readonly SidebarAccessRow[];
    people: readonly SidebarAccessRow[];
    low_priority: readonly SidebarAccessRow[];
    not_joined: readonly SidebarAccessRow[];
  };
}

export type RoomAccessProjection = {
  joinRule: RoomJoinRule | null;
  restricted: RestrictedConditions | null;
  spaceMembersRoute: string | null;
  allowedRoomNames: readonly string[];
};

export function sidebarRoomAccess(
  sidebar: SidebarAccessLists,
  roomId: string
): RoomAccessProjection {
  // #1220: a room can render from any lane (Home, a Space, People/DM, favourites,
  // low priority), so all of them are searched; otherwise the three surfaces
  // could disagree about one room.
  const row = [
    ...sidebar.space_rooms,
    ...sidebar.global_dms,
    ...sidebar.not_joined_space_rooms,
    ...(sidebar.sections?.favourites ?? []),
    ...(sidebar.sections?.rooms ?? []),
    ...(sidebar.sections?.people ?? []),
    ...(sidebar.sections?.low_priority ?? []),
    ...(sidebar.sections?.not_joined ?? [])
  ].find((item) => item.room_id === roomId);
  return {
    joinRule: row?.access_join_rule ?? null,
    restricted: row?.access_restricted_conditions ?? null,
    spaceMembersRoute: row?.access_space_members_route ?? null,
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
