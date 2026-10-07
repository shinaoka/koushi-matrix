import { describe, expect, it } from "vitest";

import {
  SCHEDULED_SEND_PERSISTENCE_FAILED_CODE,
  hasScheduledSendPersistenceFailure
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
