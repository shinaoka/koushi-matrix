import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import generatedCoreEvents from "./coreEvents.generated.json";
import type {
  EventNavigationSource,
  RoomListReadiness,
  RoomListSource,
  SessionState
} from "./types";

const blockedSession = (
  failure: "unsupported" | "unreachable" | "invalidResponse"
): SessionState => ({
  kind: "capabilityBlocked",
  homeserver: "https://matrix.example.test",
  user_id: "@qa:example.test",
  device_id: "QADEVICE",
  failure
});

describe("sync wire contract", () => {
  test("projects all capability outcomes and readiness states without legacy tokens", () => {
    const capabilityOutcomes: SessionState[] = [
      {
        kind: "ready",
        homeserver: "https://matrix.example.test",
        user_id: "@qa:example.test",
        device_id: "QADEVICE"
      },
      blockedSession("unsupported"),
      blockedSession("unreachable"),
      blockedSession("invalidResponse")
    ];
    const readinessStates: RoomListReadiness[] = [
      { kind: "uninitialized" },
      { kind: "loading", source: "cache", generation: 1 },
      { kind: "ready", source: "live", generation: 2 },
      { kind: "failed", source: "live", generation: 3, failureKind: "service" }
    ];

    expect(capabilityOutcomes.map((state) => state.kind)).toEqual([
      "ready",
      "capabilityBlocked",
      "capabilityBlocked",
      "capabilityBlocked"
    ]);
    expect(readinessStates).toHaveLength(4);
    expect(generatedCoreEvents).toHaveProperty("syncStarted");
    expect(JSON.stringify(generatedCoreEvents)).not.toMatch(
      /SyncMode|syncModeChanged|syncService|legacy|LegacySync/
    );
  });

  test("rejects obsolete source and incomplete capability-blocked states at compile time", () => {
    const live: RoomListSource = "live";
    expect(live).toBe("live");

    // @ts-expect-error Legacy room-list source is no longer a wire state.
    const legacy: RoomListSource = "legacy";
    expect(legacy).toBe("legacy");

    // @ts-expect-error Capability-blocked state must carry a typed failure.
    const incomplete: SessionState = {
      kind: "capabilityBlocked",
      homeserver: "https://matrix.example.test",
      user_id: "@qa:example.test",
      device_id: "QADEVICE"
    };
    expect(incomplete).toBeDefined();

    // @ts-expect-error Ready must carry the complete account identity.
    const incompleteReady: SessionState = { kind: "ready" };
    expect(incompleteReady).toBeDefined();

    // @ts-expect-error Session-only failure tokens cannot appear on signed-out state.
    const invalidSignedOut: SessionState = { kind: "signedOut", failure: "unsupported" };
    expect(invalidSignedOut).toBeDefined();
  });
});

// #1667: the TypeScript union must list exactly the variants Rust serializes
// (camelCase), or a new Rust source falls through renderer switches unnoticed.
const EVENT_NAVIGATION_SOURCES = {
  activity: true,
  search: true,
  pinned: true,
  notification: true,
  replyQuote: true
} satisfies Record<EventNavigationSource, true>;

describe("event navigation source wire contract", () => {
  test("mirrors every Rust EventNavigationSource variant", () => {
    const rust = readFileSync(
      new URL("../../../../crates/koushi-state/src/state/navigation.rs", import.meta.url),
      "utf8"
    );
    const body = /#\[serde\(rename_all = "camelCase"\)\]\s*pub enum EventNavigationSource \{([^}]*)\}/.exec(
      rust
    )?.[1];
    expect(body).toBeDefined();
    const variants = (body ?? "")
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("//"))
      .map((line) => line.replace(/,$/, ""))
      .map((variant) => variant.charAt(0).toLowerCase() + variant.slice(1));

    expect(variants.length).toBeGreaterThan(0);
    expect([...variants].sort()).toEqual(Object.keys(EVENT_NAVIGATION_SOURCES).sort());
  });
});
