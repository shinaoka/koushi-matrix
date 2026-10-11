import type { MessageId } from "../i18n/messages";
import type { RestrictedConditions, RoomJoinRule } from "./types";

/**
 * #1327: the access-condition vocabulary shared by the room list, the
 * conversation header, the Space rail/header and the Room Info summary.
 *
 * The condition comes from Rust (`RoomListItem.access_join_rule` /
 * `SpaceRailItem.access_join_rule` / `access_restricted_conditions`), which
 * projects the object's own `m.room.join_rules`. It is independent of
 * encryption, DM status, the viewer's membership and `can_join`, and it is never
 * inferred here.
 *
 * A `null`/absent rule means "not projected yet" (or a row lane that has no
 * condition, such as an invitation), never a guessed rule.
 *
 * #1327 replaces the #1166 vocabulary: a padlock now means encryption only, so
 * participation uses `UserRoundPlus` (invite), `UsersRound` (restricted) and
 * `Hand` (request), and `CircleHelp` carries the reserved/unsupported states.
 */
export type RoomAccessGlyph =
  | "globe"
  | "userRoundPlus"
  | "usersRound"
  | "hand"
  | "circleHelp"
  | "checking";

export type RoomAccessIndicator = {
  /** The one icon this state renders; never a padlock for participation. */
  glyph: RoomAccessGlyph;
  /** The pill text where the surface has room for a label. */
  labelMessageId: MessageId;
  /** The concise explanatory popup for hover and keyboard focus. */
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
 * `null`/`undefined` stays `null`: the caller renders the checking state, never a
 * guessed rule.
 *
 * `isDirectMessage` selects the shortened invitation explanation the issue
 * specifies for DMs (the "Space members still need an invitation" sentence is
 * meaningless in a DM). It never suppresses the condition itself.
 */
export function roomAccessIndicator(
  rule: RoomJoinRule | null | undefined,
  restricted?: RestrictedConditions | null,
  route: RoomAccessRouteFacts = {},
  isDirectMessage = false
): RoomAccessIndicator | null {
  // #1220: the specific "Space members can join" sentence applies only to a
  // verified membership-only route Rust resolved to a safe Space name. Every
  // other restricted shape keeps the generic facts, so several targets, an
  // ordinary-room target or an unknown/redacted type never claims it.
  const spaceName = restricted === "membershipOnly" ? route.spaceMembersRoute?.trim() || null : null;
  const namedRoutes = route.allowedRoomNames ?? undefined;
  // Only a confirmed empty allow list proves that no usable condition exists; an
  // unmodelled or uninspected rule keeps the generic explanation.
  const noUsableConditions = restricted === "confirmedEmpty";
  switch (rule) {
    case "public":
      return {
        glyph: "globe",
        labelMessageId: "access.public",
        descriptionMessageId: "access.publicDescription"
      };
    case "invite":
      return {
        glyph: "userRoundPlus",
        labelMessageId: "access.inviteOnly",
        descriptionMessageId: isDirectMessage
          ? "access.inviteOnlyDmDescription"
          : "access.inviteOnlyDescription"
      };
    case "restricted":
      if (spaceName) {
        return {
          glyph: "usersRound",
          labelMessageId: "access.spaceMembersCanJoin",
          descriptionMessageId: "access.spaceMembersCanJoinDescription",
          descriptionSpaceName: spaceName
        };
      }
      return {
        glyph: "usersRound",
        labelMessageId: "access.conditionsApply",
        descriptionMessageId: noUsableConditions
          ? "access.restrictedNoUsableConditionsDescription"
          : "access.conditionsDescription",
        descriptionAllowedRoomNames: noUsableConditions ? undefined : namedRoutes
      };
    case "knock":
      return {
        glyph: "hand",
        labelMessageId: "access.canRequest",
        descriptionMessageId: "access.requestDescription"
      };
    case "knockRestricted":
      if (spaceName) {
        // One pill explains both routes: the membership route names the Space,
        // the request route keeps its own sentence inside the same popup.
        return {
          glyph: "hand",
          labelMessageId: "access.knockRestrictedLabel",
          descriptionMessageId: "access.knockRestrictedSpaceDescription",
          descriptionSpaceName: spaceName
        };
      }
      return {
        glyph: "hand",
        labelMessageId: "access.knockRestrictedLabel",
        descriptionMessageId: noUsableConditions
          ? "access.restrictedNoUsableConditionsCanRequestDescription"
          : "access.knockRestrictedDescription",
        descriptionAllowedRoomNames: noUsableConditions ? undefined : namedRoutes
      };
    case "private":
      // The reserved Matrix value: ordinary joining is unavailable, so it is its
      // own state rather than folded into `unknown`.
      return {
        glyph: "circleHelp",
        labelMessageId: "access.privateReserved",
        descriptionMessageId: "access.privateReservedDescription"
      };
    case "unknown":
      return {
        glyph: "circleHelp",
        labelMessageId: "access.unknown",
        descriptionMessageId: "access.unknownDescription"
      };
    default:
      return null;
  }
}

/** The indicator a surface shows while the condition has not been projected. */
export const ROOM_ACCESS_CHECKING: RoomAccessIndicator = {
  glyph: "checking",
  labelMessageId: "access.checking",
  descriptionMessageId: "access.checkingDescription"
};

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
