import { describe, expect, it } from "vitest";

import { t } from "../i18n/messages";
import {
  SCHEDULED_SEND_PERSISTENCE_FAILED_CODE,
  graphemeCount,
  hasScheduledSendPersistenceFailure,
  roomAccessTooltipLabel
} from "./uiShared";

describe("scheduled-send persistence failure (#1159)", () => {
  it("matches only Core's local-save failure code", () => {
    expect(
      hasScheduledSendPersistenceFailure([
        { code: SCHEDULED_SEND_PERSISTENCE_FAILED_CODE }
      ])
    ).toBe(true);
    expect(hasScheduledSendPersistenceFailure([{ code: "other_failure" }])).toBe(false);
    expect(hasScheduledSendPersistenceFailure([])).toBe(false);
  });

  it("uses the code Core publishes", () => {
    // Mirrors `SCHEDULED_SEND_PERSISTENCE_FAILED` in
    // crates/koushi-state/src/reducer/timeline.rs.
    expect(SCHEDULED_SEND_PERSISTENCE_FAILED_CODE).toBe(
      "scheduled_send_persistence_failed"
    );
  });
});

describe("grapheme counting (#1217)", () => {
  it("counts a decomposed name as the same length as its precomposed form", () => {
    expect(graphemeCount("é")).toBe(1);
    expect(graphemeCount("e\u0301")).toBe(1);
    expect(graphemeCount("café")).toBe(4);
    expect(graphemeCount("cafe\u0301")).toBe(4);
  });

  it("counts CJK and emoji sequences as single graphemes", () => {
    expect(graphemeCount("研究室")).toBe(3);
    expect(graphemeCount("👩‍👩‍👧")).toBe(1);
  });
});

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
