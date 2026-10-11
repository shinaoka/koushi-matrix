import { describe, expect, it } from "vitest";

import {
  ROOM_ACCESS_CHECKING,
  roomAccessIndicator,
  sidebarRoomAccess
} from "./accessCondition";

describe("room access indicator (#1327)", () => {
  it("maps the documented join rules to one icon, one label and one explanation", () => {
    expect(roomAccessIndicator("public")).toEqual({
      glyph: "globe",
      labelMessageId: "access.public",
      descriptionMessageId: "access.publicDescription"
    });

    // #1327: participation never uses a padlock; the padlock is the encryption
    // indicator and nothing else.
    const inviteOnly = roomAccessIndicator("invite");
    expect(inviteOnly).toEqual({
      glyph: "userRoundPlus",
      labelMessageId: "access.inviteOnly",
      descriptionMessageId: "access.inviteOnlyDescription"
    });

    const knock = roomAccessIndicator("knock");
    expect(knock).toEqual({
      glyph: "hand",
      labelMessageId: "access.canRequest",
      descriptionMessageId: "access.requestDescription"
    });

    // #1327: every participation state uses one of these glyphs. A padlock is
    // the encryption indicator, so no access state may resolve to one.
    for (const indicator of [
      roomAccessIndicator("public"),
      inviteOnly,
      knock,
      roomAccessIndicator("restricted"),
      roomAccessIndicator("knockRestricted"),
      roomAccessIndicator("private"),
      roomAccessIndicator("unknown"),
      ROOM_ACCESS_CHECKING
    ]) {
      expect(["globe", "userRoundPlus", "usersRound", "hand", "circleHelp", "checking"]).toContain(
        indicator?.glyph
      );
    }
  });

  it("shortens only the invitation explanation for a DM, never the condition", () => {
    expect(roomAccessIndicator("invite", null, {}, true)).toEqual({
      glyph: "userRoundPlus",
      labelMessageId: "access.inviteOnly",
      descriptionMessageId: "access.inviteOnlyDmDescription"
    });
    // Every other rule keeps one explanation for rooms and DMs alike.
    for (const rule of ["public", "knock", "restricted", "knockRestricted", "private", "unknown"] as const) {
      expect(roomAccessIndicator(rule, null, {}, true)).toEqual(roomAccessIndicator(rule));
    }
  });

  it("keeps the reserved and unrecognised rules their own state", () => {
    const reserved = roomAccessIndicator("private");
    expect(reserved?.glyph).toBe("circleHelp");
    expect(reserved?.labelMessageId).toBe("access.privateReserved");
    expect(reserved?.descriptionMessageId).toBe("access.privateReservedDescription");

    const unknown = roomAccessIndicator("unknown");
    expect(unknown?.glyph).toBe("circleHelp");
    expect(unknown?.labelMessageId).toBe("access.unknown");
    expect(unknown?.descriptionMessageId).toBe("access.unknownDescription");
  });

  it("has no indicator for a rule that has not been projected", () => {
    expect(roomAccessIndicator(null)).toBeNull();
    expect(roomAccessIndicator(undefined)).toBeNull();
  });

  it("presents the checking state as its own neutral glyph, never a guessed rule", () => {
    expect(ROOM_ACCESS_CHECKING).toEqual({
      glyph: "checking",
      labelMessageId: "access.checking",
      descriptionMessageId: "access.checkingDescription"
    });
  });
});

describe("restricted allow-condition facts (#1166, #1220)", () => {
  it("says no usable condition is configured only when that is confirmed", () => {
    const noUsable = roomAccessIndicator("restricted", "confirmedEmpty");
    expect(noUsable?.descriptionMessageId).toBe(
      "access.restrictedNoUsableConditionsDescription"
    );

    // An allow-rule type the client does not model keeps the generic wording:
    // the absence of a usable condition is not confirmed.
    expect(roomAccessIndicator("restricted", "unsupportedOnly")?.descriptionMessageId).toBe(
      "access.conditionsDescription"
    );
    expect(roomAccessIndicator("restricted", "notInspected")?.descriptionMessageId).toBe(
      "access.conditionsDescription"
    );
  });

  it("keeps the request route when a knock_restricted rule has no usable condition", () => {
    const indicator = roomAccessIndicator("knockRestricted", "confirmedEmpty");
    expect(indicator?.glyph).toBe("hand");
    expect(indicator?.labelMessageId).toBe("access.knockRestrictedLabel");
    expect(indicator?.descriptionMessageId).toBe(
      "access.restrictedNoUsableConditionsCanRequestDescription"
    );
  });

  it("reads both access facts from a sidebar row", () => {
    const sidebar = {
      space_rooms: [
        {
          room_id: "!room:example.invalid",
          access_join_rule: "restricted" as const,
          access_restricted_conditions: "confirmedEmpty" as const
        }
      ],
      global_dms: [],
      not_joined_space_rooms: []
    };
    expect(sidebarRoomAccess(sidebar, "!room:example.invalid")).toEqual({
      joinRule: "restricted",
      restricted: "confirmedEmpty",
      spaceMembersRoute: null,
      allowedRoomNames: []
    });
    expect(sidebarRoomAccess(sidebar, "!missing:example.invalid")).toEqual({
      joinRule: null,
      restricted: null,
      spaceMembersRoute: null,
      allowedRoomNames: []
    });
  });

  it("uses the specific sentence only for a verified single-Space route", () => {
    const specific = roomAccessIndicator("restricted", "membershipOnly", {
      spaceMembersRoute: "Alpha Space"
    });
    expect(specific).toEqual({
      glyph: "usersRound",
      labelMessageId: "access.spaceMembersCanJoin",
      descriptionMessageId: "access.spaceMembersCanJoinDescription",
      descriptionSpaceName: "Alpha Space"
    });

    // A blank name never claims it; the generic facts carry the named routes.
    for (const name of ["", "   ", null, undefined]) {
      const generic = roomAccessIndicator("restricted", "membershipOnly", {
        spaceMembersRoute: name,
        allowedRoomNames: ["Allowed Room"]
      });
      expect(generic?.descriptionMessageId).toBe("access.conditionsDescription");
      expect(generic?.descriptionSpaceName).toBeUndefined();
      expect(generic?.descriptionAllowedRoomNames).toEqual(["Allowed Room"]);
    }

    // Every other completeness keeps the generic facts even with a name.
    for (const restricted of [
      "notInspected",
      "confirmedEmpty",
      "membershipPlusUnsupported",
      "unsupportedOnly"
    ] as const) {
      const generic = roomAccessIndicator("restricted", restricted, {
        spaceMembersRoute: "Alpha Space"
      });
      expect(generic?.descriptionSpaceName).toBeUndefined();
    }

    // `private` is never the Space-membership route.
    const privateRule = roomAccessIndicator("private", "membershipOnly", {
      spaceMembersRoute: "Alpha Space"
    });
    expect(privateRule?.descriptionMessageId).toBe("access.privateReservedDescription");
    expect(privateRule?.descriptionSpaceName).toBeUndefined();
  });

  it("explains both routes of a single-Space knock-restricted rule in one pill", () => {
    const indicator = roomAccessIndicator("knockRestricted", "membershipOnly", {
      spaceMembersRoute: "Alpha Space"
    });
    expect(indicator).toEqual({
      glyph: "hand",
      labelMessageId: "access.knockRestrictedLabel",
      descriptionMessageId: "access.knockRestrictedSpaceDescription",
      descriptionSpaceName: "Alpha Space"
    });
  });

  it("finds a row in every sidebar lane and carries the verified Space route", () => {
    const row = {
      room_id: "!room:example.invalid",
      access_join_rule: "restricted" as const,
      access_restricted_conditions: "membershipOnly" as const,
      access_allowed_room_names: ["Allowed Space"],
      access_space_members_route: "Allowed Space"
    };
    // A room can render from any lane, so the row is placed in each lane in turn
    // rather than only in the one the reader happens to check.
    const sidebarFor = (
      lane:
        | "space_rooms"
        | "global_dms"
        | "not_joined_space_rooms"
        | "favourites"
        | "rooms"
        | "people"
        | "low_priority"
        | "not_joined"
    ) => {
      const sidebar: Parameters<typeof sidebarRoomAccess>[0] = {
        space_rooms: [],
        global_dms: [],
        not_joined_space_rooms: [],
        sections: { favourites: [], rooms: [], people: [], low_priority: [], not_joined: [] }
      };
      if (lane === "space_rooms" || lane === "global_dms" || lane === "not_joined_space_rooms") {
        sidebar[lane] = [row];
      } else {
        sidebar.sections![lane] = [row];
      }
      return sidebar;
    };
    const expected = {
      joinRule: "restricted" as const,
      restricted: "membershipOnly" as const,
      spaceMembersRoute: "Allowed Space",
      allowedRoomNames: ["Allowed Space"]
    };
    for (const lane of [
      "space_rooms",
      "global_dms",
      "not_joined_space_rooms",
      "favourites",
      "rooms",
      "people",
      "low_priority",
      "not_joined"
    ] as const) {
      expect(sidebarRoomAccess(sidebarFor(lane), row.room_id), `${lane} lane`).toEqual(expected);
    }
  });
});
