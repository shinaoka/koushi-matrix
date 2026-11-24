import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import {
  assertNoMatrixIdentifiers,
  assertRequiredTokens,
  requiredTokensForHeadlessScenario,
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

  test("registers the redaction/edit convergence token", () => {
    expect(requiredTokensForHeadlessScenario("redact_edit_convergence")).toEqual([
      "redact_edit_convergence=ok",
      "thread_summary_convergence=ok"
    ]);
    expect(() =>
      assertRequiredTokens(
        "redact_edit_convergence=ok thread_summary_convergence=ok",
        requiredTokensForHeadlessScenario("redact_edit_convergence"),
        "redact_edit_convergence"
      )
    ).not.toThrow();
  });

  test.each([
    [
      "scroll_back_cached_history",
      "QaStage::ScrollBackCachedHistory",
      [
        "scroll_back_seeded=ok",
        "scroll_back_restored=ok",
        "scroll_back_reached_start=ok",
        "scroll_back_store_served=ok",
        "scroll_back_no_runaway=ok",
        "scroll_back_cached_history=ok"
      ]
    ],
    [
      "thread_receipt_fresh_session",
      "QaStage::ThreadReceiptFreshSession",
      [
        "thread_receipt_fresh_unthreaded=ok",
        "thread_receipt_fresh_clash=ok",
        "thread_receipt_fresh_read=ok",
        "thread_receipt_fresh_restart=ok",
        "thread_receipt_fresh_session=ok"
      ]
    ]
  ])("registers the %s checkpoints its stage declares", (scenario, stage, tokens) => {
    // #2079: the runner admitted a core-only scenario without --core and exited
    // green without running it. This pins the JS contract to the stage's own
    // declaration, so a Rust-side rename cannot leave the lane permanently red.
    expect(requiredTokensForHeadlessScenario(scenario)).toEqual(tokens);

    const registry = readFileSync(
      new URL("../../../../crates/koushi-qa/src/bin/headless_core_qa/registry.rs", import.meta.url),
      "utf8"
    );
    const arm = registry.match(new RegExp(`${stage} => &\\[([\\s\\S]*?)\\],`));
    expect(arm, `${stage} arm not found in registry.rs`).not.toBeNull();
    for (const token of tokens) {
      expect(arm?.[1]).toContain(`"${token}"`);
    }
  });

  test.each([
    ["avatar_demand", "avatar_window_requests=ok"],
    ["directory", "room_address_preview_create_share=ok"],
    ["directory", "room_address_collision=ok"],
    ["live_signals", "ignored_user_history_recovery=ok"],
    ["all", "room_address_collision=ok"],
    ["all", "ignored_user_history_recovery=ok"]
  ])("requires %s checkpoint %s even with a successful process", (scenario, token) => {
    const required = requiredTokensForHeadlessScenario(scenario);
    expect(required).toContain(token);
    expect(() => assertRequiredTokens(required.filter(value => value !== token).join(" "), required, scenario)).toThrow(/missing required QA tokens/);
    expect(() => assertRequiredTokens(required.join(" "), required, scenario)).not.toThrow();
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
