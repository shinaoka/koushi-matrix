import { describe, expect, test } from "vitest";

import { e2eeSendDiagnosticMessage } from "./e2eeSendDiagnostics";
import type { DesktopSnapshot } from "./types";

describe("e2eeSendDiagnosticMessage", () => {
  test("summarizes encrypted send state without private identifiers or message bodies", () => {
    const snapshot = {
      state: {
        domain: {
          rooms: [
            {
              room_id: "!secret:example.invalid",
              is_encrypted: true,
              is_dm: true,
              dm_user_ids: ["@recipient:example.invalid"],
              joined_members: 2
            }
          ],
          current_session_status: {
            status: "ready",
            request_id: 1,
            details: { verification: "verified" }
          },
          e2ee_trust: {
            cross_signing: { kind: "trusted" },
            key_backup: { kind: "enabled", version: "private-version" },
            devices: [
              {
                user_id: "@own:example.invalid",
                device_id: "DEVICEID",
                trust_level: "verified"
              }
            ]
          }
        }
      }
    } as unknown as DesktopSnapshot;

    const message = e2eeSendDiagnosticMessage(snapshot, "!secret:example.invalid");

    expect(message).toContain("phase=before_send");
    expect(message).toContain("encrypted=true");
    expect(message).toContain("dm=true");
    expect(message).toContain("dm_targets=1");
    expect(message).toContain("joined_members=2");
    expect(message).toContain("key_backup=enabled");
    expect(message).toContain("cross_signing=trusted");
    expect(message).toContain("current_session_verification=verified");
    expect(message).not.toContain("own_sessions=");
    expect(message).toContain("trust_devices=1");
    expect(message).not.toContain("!secret");
    expect(message).not.toContain("@recipient");
    expect(message).not.toContain("@own");
    expect(message).not.toContain("DEVICEID");
    expect(message).not.toContain("private-version");
  });
});
