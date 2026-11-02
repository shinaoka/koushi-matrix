import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import type { AppDomainState } from "./types";

/**
 * #1748: `ConversationAccess` is a Rust-owned projection whose field names come
 * from `#[serde(rename_all = "camelCase")]`, so the hand-maintained TypeScript
 * mirror must use those exact keys. A snake_case mirror type-checks and passes
 * every fixture that shares the same mistake, while the real snapshot makes
 * `sendAdmitted` permanently `undefined` and the desktop shell un-writable.
 *
 * `frontend_app_state.json` is generated from the Rust serialization, so it is
 * the artifact that proves the wire names.
 */
const golden: { state: { domain: Record<string, unknown> } } = JSON.parse(
  readFileSync(
    new URL("../../src-tauri/tests/golden/frontend_app_state.json", import.meta.url),
    "utf8"
  )
);

describe("conversation_access wire contract", () => {
  test("uses the Rust serialization keys, not Rust field names", () => {
    const access = golden.state.domain.conversation_access as Record<string, unknown>;
    expect(access).toBeTruthy();
    expect(Object.keys(access).sort()).toEqual(["listAuthoritative", "readable", "sendAdmitted"]);
  });

  test("the TypeScript mirror declares exactly those keys", () => {
    // A literal of the mirror type: a snake_case interface makes this a type
    // error, and a camelCase interface that disagrees with the wire makes the
    // comparison below fail.
    const mirror: AppDomainState["conversation_access"] = {
      readable: true,
      listAuthoritative: true,
      sendAdmitted: true
    };
    expect(Object.keys(mirror).sort()).toEqual(
      Object.keys(golden.state.domain.conversation_access as Record<string, unknown>).sort()
    );
  });
});
