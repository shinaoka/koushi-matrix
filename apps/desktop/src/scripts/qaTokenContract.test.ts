import { describe, expect, test } from "vitest";

import {
  assertNoMatrixIdentifiers,
  assertRequiredTokens,
  tokensFromOutput
} from "../../../../scripts/lib/qa-token-contract.mjs";

describe("qa token contract", () => {
  test("extracts only closed-vocabulary success tokens", () => {
    const tokens = tokensFromOutput(
      "login=ok sync=running qa_room=created rooms=5 sync_backend=SyncService prose here"
    );
    expect(tokens.has("login=ok")).toBe(true);
    expect(tokens.has("sync=running")).toBe(true);
    expect(tokens.has("qa_room=created")).toBe(true);
    // counts and backend names are not status tokens
    expect(tokens.has("rooms=5")).toBe(false);
    expect(tokens.has("sync_backend=SyncService")).toBe(false);
  });

  test("assertRequiredTokens throws naming the missing tokens", () => {
    expect(() =>
      assertRequiredTokens("login=ok", ["login=ok", "logout=ok"], "lane")
    ).toThrow(/missing required QA tokens: logout=ok/);
    expect(() =>
      assertRequiredTokens("login=ok logout=ok", ["login=ok", "logout=ok"], "lane")
    ).not.toThrow();
  });

  test("assertNoMatrixIdentifiers rejects user, room, and event ids", () => {
    for (const leak of [
      "login=ok user=@alice:example.org",
      "qa_room=created !room123:example.org",
      "send_msg1=ok $event-abc:example.org"
    ]) {
      expect(() => assertNoMatrixIdentifiers(leak, "lane")).toThrow(
        /private Matrix identifier leaked/
      );
    }
  });

  test("assertNoMatrixIdentifiers passes private-data-free token output", () => {
    expect(() =>
      assertNoMatrixIdentifiers(
        "login=ok sync=running qa_room=created send_msg1=ok logout=ok",
        "lane"
      )
    ).not.toThrow();
  });
});
