import { describe, expect, it } from "vitest";

import {
  ROOM_ACCESS_CHECKING,
  roomAccessHeaderBadges,
  roomAccessIndicator,
  roomAccessRailSummary,
  sidebarRoomAccess
} from "./accessCondition";

describe("room access indicator (#1166)", () => {
  it("maps the documented join rules to their icons, badges and explanations", () => {
    const publicRoom = roomAccessIndicator("public");
    expect(publicRoom?.icon).toBe("globe");
    expect(publicRoom?.badges).toEqual([]);
    expect(publicRoom?.labelMessageIds).toEqual(["access.public"]);
    expect(publicRoom?.descriptionMessageId).toBe("access.publicDescription");

    const inviteOnly = roomAccessIndicator("invite");
    expect(inviteOnly?.icon).toBe("padlock");
    expect(inviteOnly?.badges).toEqual([]);
    expect(inviteOnly?.labelMessageIds).toEqual(["access.inviteOnly"]);

    const restricted = roomAccessIndicator("restricted");
    expect(restricted?.icon).toBeNull();
    expect(restricted?.badges.map((badge) => badge.labelMessageId)).toEqual([
      "access.conditionsApply"
    ]);

    const knock = roomAccessIndicator("knock");
    expect(knock?.badges.map((badge) => badge.labelMessageId)).toEqual([
      "access.canRequest"
    ]);
  });

  it("orders both knock_restricted badges and keeps each route's explanation", () => {
    const indicator = roomAccessIndicator("knockRestricted");
    expect(indicator?.icon).toBeNull();
    expect(indicator?.badges).toEqual([
      {
        labelMessageId: "access.conditionsApply",
        descriptionMessageId: "access.conditionsRouteDescription"
      },
      {
        labelMessageId: "access.canRequest",
        descriptionMessageId: "access.requestRouteDescription"
      }
    ]);
    expect(indicator?.labelMessageIds).toEqual([
      "access.conditionsApply",
      "access.canRequest"
    ]);
    expect(indicator?.descriptionMessageId).toBe("access.knockRestrictedDescription");
  });

  it("never presents an unrecognised rule as a known condition", () => {
    for (const rule of ["private", "unknown"] as const) {
      const indicator = roomAccessIndicator(rule);
      // No globe and no padlock: it must not look public or invite-only.
      expect(indicator?.icon).toBeNull();
      expect(indicator?.badges.map((badge) => badge.labelMessageId)).toEqual([
        "access.unknown"
      ]);
      expect(indicator?.labelMessageIds).toEqual(["access.unknownFull"]);
    }
  });

  it("has no indicator for a rule that has not been projected", () => {
    expect(roomAccessIndicator(null)).toBeNull();
    expect(roomAccessIndicator(undefined)).toBeNull();
  });

  it("explains the checking state without looking public or invite-only", () => {
    expect(ROOM_ACCESS_CHECKING.icon).toBeNull();
    expect(ROOM_ACCESS_CHECKING.badges).toEqual([
      {
        labelMessageId: "access.checking",
        descriptionMessageId: "access.checkingDescription"
      }
    ]);
  });
});

describe("room access header badges", () => {
  it("uses the full label in a header and keeps each route's explanation", () => {
    // Public and invite-only have no compact badge; the header shows the label.
    expect(roomAccessHeaderBadges(roomAccessIndicator("public")!)).toEqual([
      {
        labelMessageId: "access.public",
        descriptionMessageId: "access.publicDescription"
      }
    ]);
    expect(roomAccessHeaderBadges(roomAccessIndicator("knockRestricted")!)).toEqual([
      {
        labelMessageId: "access.conditionsApply",
        descriptionMessageId: "access.conditionsRouteDescription"
      },
      {
        labelMessageId: "access.canRequest",
        descriptionMessageId: "access.requestRouteDescription"
      }
    ]);
  });

  it("uses the full unknown and checking labels rather than their compact form", () => {
    expect(roomAccessHeaderBadges(roomAccessIndicator("unknown")!)).toEqual([
      {
        labelMessageId: "access.unknownFull",
        descriptionMessageId: "access.unknownDescription"
      }
    ]);
    expect(roomAccessHeaderBadges(ROOM_ACCESS_CHECKING)).toEqual([
      {
        labelMessageId: "access.checkingFull",
        descriptionMessageId: "access.checkingDescription"
      }
    ]);
  });
});

describe("room access rail summary", () => {
  it("collapses the five states to one glyph that never reads as a known icon for unknowns", () => {
    expect(roomAccessRailSummary("public")).toBe("globe");
    expect(roomAccessRailSummary("invite")).toBe("padlock");
    for (const rule of ["restricted", "knock", "knockRestricted"] as const) {
      expect(roomAccessRailSummary(rule)).toBe("info");
    }
    for (const rule of ["private", "unknown"] as const) {
      expect(roomAccessRailSummary(rule)).toBe("question");
    }
    expect(roomAccessRailSummary(null)).toBe("loading");
    expect(roomAccessRailSummary(undefined)).toBe("loading");
  });
});

describe("restricted allow-condition facts (#1166)", () => {
  it("says an invitation is required only when no usable condition is confirmed", () => {
    const noUsable = roomAccessIndicator("restricted", "confirmedEmpty");
    expect(noUsable?.descriptionMessageId).toBe(
      "access.restrictedNoUsableConditionsDescription"
    );
    expect(noUsable?.badges).toEqual([
      {
        labelMessageId: "access.conditionsApply",
        descriptionMessageId: "access.restrictedNoUsableConditionsDescription"
      }
    ]);

    // An allow-rule type the client does not model keeps the generic wording:
    // the absence of a usable condition is not confirmed.
    const unknown = roomAccessIndicator("restricted", "unsupportedOnly");
    expect(unknown?.descriptionMessageId).toBe("access.conditionsDescription");

    // A usable condition keeps the membership explanation.
    const usable = roomAccessIndicator("restricted", "membershipOnly");
    expect(usable?.descriptionMessageId).toBe("access.conditionsDescription");
  });

  it("keeps the request route when a knock_restricted rule has no usable condition", () => {
    const indicator = roomAccessIndicator("knockRestricted", "confirmedEmpty");
    expect(indicator?.badges).toEqual([
      {
        labelMessageId: "access.conditionsApply",
        descriptionMessageId: "access.restrictedNoUsableConditionsDescription"
      },
      {
        labelMessageId: "access.canRequest",
        descriptionMessageId: "access.requestRouteDescription"
      }
    ]);
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
    // A verified membership-only route names the Space.
    const specific = roomAccessIndicator("restricted", "membershipOnly", {
      spaceMembersRoute: "Alpha Space"
    });
    expect(specific?.labelMessageIds).toEqual(["access.spaceMembersCanJoin"]);
    expect(specific?.descriptionMessageId).toBe("access.spaceMembersCanJoinDescription");
    expect(specific?.descriptionSpaceName).toBe("Alpha Space");
    expect(specific?.badges).toEqual([
      {
        labelMessageId: "access.spaceMembersCanJoin",
        descriptionMessageId: "access.spaceMembersCanJoinDescription",
        descriptionSpaceName: "Alpha Space"
      }
    ]);

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
    expect(privateRule?.descriptionMessageId).toBe("access.unknownDescription");
    expect(privateRule?.descriptionSpaceName).toBeUndefined();
  });

  it("keeps the request badge and names the Space for a single-Space knock-restricted rule", () => {
    const indicator = roomAccessIndicator("knockRestricted", "membershipOnly", {
      spaceMembersRoute: "Alpha Space"
    });
    expect(indicator?.badges).toEqual([
      {
        labelMessageId: "access.spaceMembersCanJoin",
        descriptionMessageId: "access.spaceMembersCanJoinDescription",
        descriptionSpaceName: "Alpha Space"
      },
      {
        labelMessageId: "access.canRequest",
        descriptionMessageId: "access.requestRouteDescription"
      }
    ]);
    expect(indicator?.descriptionMessageId).toBe(
      "access.spaceMembersCanJoinCanRequestDescription"
    );
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
