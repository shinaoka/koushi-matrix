import { describe, expect, test } from "vitest";

import { currentSessionStatusFactsRemainAuthoritative } from "./currentSessionStatus";
import type { CurrentSessionStatusDetails, CurrentSessionStatusFailureKind } from "./types";

const details: CurrentSessionStatusDetails = {
  device_display_name: "Koushi",
  device_id: "DEVICE",
  authentication_method: "oauth",
  sync_state: "running",
  is_cross_signed_by_owner: true,
  own_identity_verification: "verified",
  key_backup: "ready",
  verification: "verified",
  checked_at_ms: 1
};

function failed(kind: CurrentSessionStatusFailureKind) {
  return {
    status: "failed" as const,
    request_id: 2,
    kind,
    checked_at_ms: 2,
    last_known_details: details
  };
}

describe("current-session stale facts", () => {
  test("remain authoritative only for transient connectivity failures", () => {
    for (const kind of ["timed_out", "connectivity_unavailable", "network"] as const) {
      expect(currentSessionStatusFactsRemainAuthoritative(failed(kind))).toBe(true);
    }
    for (const kind of ["authentication", "server", "sdk", "unavailable"] as const) {
      expect(currentSessionStatusFactsRemainAuthoritative(failed(kind))).toBe(false);
    }
  });
});
