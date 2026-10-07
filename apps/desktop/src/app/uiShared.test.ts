import { describe, expect, it } from "vitest";

import { t } from "../i18n/messages";
import { roomAccessTooltipLabel } from "./uiShared";

describe("room access tooltip label (#1166)", () => {
  it("lists a restricted rule's named routes only when they were resolved", () => {
    expect(
      roomAccessTooltipLabel("access.conditionsDescription", ["Alpha Room", "Beta Space"])
    ).toBe(
      `${t("access.conditionsDescription")} ${t("access.allowedRooms", {
        rooms: `Alpha Room${t("access.labelSeparator")}Beta Space`
      })}`
    );
  });

  it("falls back to the plain explanation when nothing resolved", () => {
    // No resolved names: never a raw id and never a guessed label.
    expect(roomAccessTooltipLabel("access.conditionsDescription", [])).toBe(
      t("access.conditionsDescription")
    );
    expect(roomAccessTooltipLabel("access.conditionsDescription", ["   "])).toBe(
      t("access.conditionsDescription")
    );
    expect(roomAccessTooltipLabel("access.conditionsDescription", null)).toBe(
      t("access.conditionsDescription")
    );
    expect(roomAccessTooltipLabel("access.conditionsDescription")).toBe(
      t("access.conditionsDescription")
    );
  });
});
