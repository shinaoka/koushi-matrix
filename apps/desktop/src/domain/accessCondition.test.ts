import { describe, expect, it } from "vitest";

import { ROOM_ACCESS_CHECKING, roomAccessIndicator } from "./accessCondition";

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
