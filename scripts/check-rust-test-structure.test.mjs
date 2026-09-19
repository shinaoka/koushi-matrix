#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import {
  analyzeRustSource,
  checkDesktopTauriCommandRegistrationContract,
  checkDesktopNativeWindowLifecycleContract,
  checkCoreRuntimePersistenceBlockingPort,
  checkCoreRoomActorCommandLoop,
  checkCoreRoomDirectoryJoinOrder,
  checkCoreRoomListKnownBookDelivery,
  checkCoreRoomListNoLegacyProjection,
  checkCoreRoomListRelayOrder,
  checkCoreRoomLiveDirectSubscriptionOrder,
  checkCoreRoomMarkReadOrder,
  checkCoreRoomMentionMembershipRefresh,
  checkCoreRoomMissingSpaceChildRepair,
  checkCoreRoomPinCommandGuard,
  checkCoreRoomPinSettlementOrder,
  checkCoreRoomSpaceInviteCancellationOrder,
  checkCoreRoomSpaceMemberFailureProjection,
  checkCoreRoomSpaceMemberBackgroundFailure,
  checkCoreRoomSyncStartedOwner,
  checkCoreRoomTagNoStaleRefresh,
  checkCoreRoomCreateLinksBeforeCompletion,
  checkCoreStoreFileCredentialCfg,
  checkCoreSearchQueryFailureClassification,
  checkCoreSearchPageCancellation,
  checkCoreSyncSingleAllRoomsOwner,
  checkCoreThreadsReliableRelays,
  checkCoreTimelineUnsubscribeCleanupOrder,
  checkCoreTimelinePaginationScheduler,
  checkCoreTimelineSendSupervision,
  checkCoreTimelineThreadReadReceipts,
  checkSdkLibrarySourceManifest,
  checkSdkRoomReadMarkerContract,
  checkSdkSessionBackupFence,
  checkStateFocusedContextReducerContract,
  findIncludeStrInvocations,
  findInlineTestModules,
  formatViolation,
  runSourceContractRules,
  scanRepository
} from "./check-rust-test-structure.mjs";

function moduleSource(bodyLines) {
  return `#[cfg(test)]\nmod tests {\n${bodyLines.join("\n")}\n}\n`;
}

test("balances comments, nested braces, strings, byte strings, raw strings, chars, and lifetimes", () => {
  const analysis = analyzeRustSource(
    moduleSource([
      '/* comment with { } */ let _ = "escaped \\\"{\\\"";',
      'let _ = b"}"; let _ = r###"raw { } \"#"###;',
      "let _ = '{'; let _: &'a str = &\"ok\";",
      "fn nested() { if true { let _ = '}'; } }"
    ]),
    { filePath: "fixture.rs" }
  );

  assert.equal(analysis.inlineTestModules.length, 1);
  assert.equal(analysis.inlineTestModules[0].name, "tests");
  assert.equal(analysis.nestedTestModules.length, 0);
});

test("rejects an inline module at the 200 physical-line ceiling", () => {
  const below = findInlineTestModules(moduleSource(Array(196).fill("    const N: usize = 1;")), "fixture.rs");
  const at = findInlineTestModules(moduleSource(Array(197).fill("    const N: usize = 1;")), "fixture.rs");

  assert.equal(below[0].physicalLines, 199);
  assert.equal(at[0].physicalLines, 200);
  assert.equal(below[0].overThreshold, false);
  assert.equal(at[0].overThreshold, true);
});

test("does not confuse a feature named test with cfg(test)", () => {
  const analysis = analyzeRustSource(
    '#[cfg(feature = "test")]\nmod feature_gate { const VALUE: usize = 1; }\n',
    { filePath: "fixture.rs" }
  );

  assert.equal(analysis.inlineTestModules.length, 0);
  assert.equal(analysis.violations.length, 0);
});

test("accepts external and path module declarations", () => {
  const analysis = analyzeRustSource(
    '#[cfg(test)] mod tests;\n#[cfg(test)] #[path = "fixture_tests.rs"] mod path_tests;\n',
    { filePath: "fixture.rs" }
  );

  assert.equal(analysis.inlineTestModules.length, 0);
  assert.equal(analysis.externalTestModules.length, 2);
  assert.equal(analysis.violations.length, 0);
});

test("rejects nested cfg(test) modules from top-level inventory", () => {
  const analysis = analyzeRustSource(
    'fn host() {\n    #[cfg(test)]\n    mod nested { fn check() {} }\n}\n',
    { filePath: "fixture.rs" }
  );

  assert.equal(analysis.inlineTestModules.length, 0);
  assert.deepEqual(analysis.nestedTestModules.map(({ name }) => name), ["nested"]);
  assert.match(formatViolation(analysis.violations[0]), /nested inline cfg\(test\) module/);
});

test("resolves literal and CARGO_MANIFEST_DIR concat include targets", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "rust-test-structure-"));
  const crate = path.join(root, "crates", "fixture");
  const sourcePath = path.join(crate, "tests", "contracts.rs");
  fs.mkdirSync(path.dirname(sourcePath), { recursive: true });
  fs.mkdirSync(path.join(crate, "src"), { recursive: true });
  fs.writeFileSync(path.join(crate, "Cargo.toml"), "[package]\nname = \"fixture\"\n");
  fs.writeFileSync(path.join(crate, "src", "lib.rs"), "fn fixture() {}\n");

  const source = `
    const A: &str = include_str!("../src/lib.rs");
    const B: &str = include_str!(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/src/lib.rs"
    ));
  `;
  const includes = findIncludeStrInvocations(source, sourcePath, { repositoryRoot: root });

  assert.deepEqual(includes.map(({ target }) => target), [
    "crates/fixture/src/lib.rs",
    "crates/fixture/src/lib.rs"
  ]);
  assert.equal(includes.every(({ exists }) => exists), true);
});

test("runs representative migrated state, SDK, and src-tauri source-contract rules", () => {
  assert.deepEqual(checkStateFocusedContextReducerContract(), []);
  assert.deepEqual(checkSdkLibrarySourceManifest(), []);
  assert.deepEqual(checkSdkRoomReadMarkerContract(), []);
  assert.deepEqual(checkSdkSessionBackupFence(), []);
  assert.deepEqual(checkDesktopTauriCommandRegistrationContract(), []);
  assert.deepEqual(checkDesktopNativeWindowLifecycleContract(), []);
});

test("runs representative migrated core source-contract rules", () => {
  for (const check of [
    checkCoreRuntimePersistenceBlockingPort,
    checkCoreSearchQueryFailureClassification,
    checkCoreSearchPageCancellation,
    checkCoreSyncSingleAllRoomsOwner,
    checkCoreThreadsReliableRelays
  ]) assert.deepEqual(check(), []);
});

test("runs representative migrated timeline source-contract rules", () => {
  for (const check of [
    checkCoreTimelineUnsubscribeCleanupOrder,
    checkCoreTimelinePaginationScheduler,
    checkCoreTimelineSendSupervision,
    checkCoreTimelineThreadReadReceipts
  ]) assert.deepEqual(check(), []);
});

test("timeline source-contract failures stay closed-token and private-data-free", () => {
  const message = formatViolation({
    kind: "source-contract",
    rule: "core.timeline.send_queue_supervision",
    message: "missing manager terminal marker"
  });

  assert.equal(
    message,
    "core.timeline.send_queue_supervision: missing manager terminal marker"
  );
  assert.doesNotMatch(message, /SECRET|@|!|synthetic-room|private-path/);
});

test("scoped core sources contain no Rust-source include embeddings", () => {
  const scoped = scanRepository().rustSourceIncludes.filter(({ file }) =>
    file === "crates/koushi-core/src/runtime.rs" ||
    file.startsWith("crates/koushi-core/src/runtime/") ||
    [
      "crates/koushi-core/src/search.rs",
      "crates/koushi-core/src/search_crawler.rs",
      "crates/koushi-core/src/sync.rs",
      "crates/koushi-core/src/threads_list.rs",
      "crates/koushi-core/src/executor.rs",
      "crates/koushi-core/src/send_diagnostics.rs",
      "crates/koushi-core/src/renderable_thumbnail.rs"
    ].includes(file)
  );
  assert.deepEqual(scoped, []);
});

test("timeline sources contain no Rust-source include embeddings", () => {
  const includes = scanRepository().rustSourceIncludes.filter(({ file }) =>
    file.startsWith("crates/koushi-core/src/timeline/")
  );
  assert.deepEqual(includes, []);
});

test("room and credential source contracts have direct checker rules and no embeddings", () => {
  const scopedFiles = [
    "crates/koushi-store/src/credential_backend.rs",
    ...scanRepository().files
      .filter((file) => file.startsWith("crates/koushi-core/src/room/"))
  ];
  const includes = scanRepository().rustSourceIncludes.filter(({ file }) =>
    scopedFiles.includes(file)
  );
  assert.deepEqual(includes, []);

  const checks = [
    checkCoreRoomActorCommandLoop,
    checkCoreRoomSyncStartedOwner,
    checkCoreRoomDirectoryJoinOrder,
    checkCoreRoomLiveDirectSubscriptionOrder,
    checkCoreRoomListNoLegacyProjection,
    checkCoreRoomListRelayOrder,
    checkCoreRoomListKnownBookDelivery,
    checkCoreRoomMentionMembershipRefresh,
    checkCoreRoomMarkReadOrder,
    checkCoreRoomTagNoStaleRefresh,
    checkCoreRoomCreateLinksBeforeCompletion,
    checkCoreRoomMissingSpaceChildRepair,
    checkCoreRoomPinSettlementOrder,
    checkCoreRoomPinCommandGuard,
    checkCoreRoomSpaceMemberFailureProjection,
    checkCoreRoomSpaceMemberBackgroundFailure,
    checkCoreRoomSpaceInviteCancellationOrder,
    checkCoreStoreFileCredentialCfg
  ];
  assert.deepEqual(checks.flatMap((check) => check()), []);
});

test("all desktop source-contract rules pass", () => {
  assert.deepEqual(
    runSourceContractRules().filter(({ rule }) => rule?.startsWith("desktop.")),
    []
  );
});

test("desktop shutdown guard rejects skipped acknowledgement and premature restart", () => {
  const source = fs.readFileSync(new URL("../apps/desktop/src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.deepEqual(checkDesktopNativeWindowLifecycleContract(source), []);
  for (const [before, after] of [
    ["runtime.wait_for_shutdown()", "runtime.submit_only()"],
    ["updater.await;", "/* updater join removed */"],
    ["restart_after_shutdown.swap(true", "restart_after_shutdown.load(/* no intent */"],
    ["quit_stage.store(QuitStage::ForcedExit.repr()", "quit_stage.store(QuitStage::ShutdownComplete.repr()"],
  ]) {
    assert.ok(source.includes(before), `mutation target missing: ${before}`);
    assert.ok(checkDesktopNativeWindowLifecycleContract(source.replace(before, after)).length > 0);
  }
  const start = source.indexOf("fn request_application_restart_with");
  const end = source.indexOf("pub(crate) fn request_application_restart", start);
  assert.ok(start >= 0 && end > start);
  const premature = source.slice(0, start)
    + source.slice(start, end).replace("exit.ordinary_exit()", "exit.final_restart()")
    + source.slice(end);
  assert.ok(checkDesktopNativeWindowLifecycleContract(premature).some(({ message }) =>
    message === "restart bypasses ordinary shutdown admission"));
});

test("registers the complete account source-contract rule set", () => {
  assert.deepEqual(
    runSourceContractRules().filter(({ rule }) => rule?.startsWith("core.account.")),
    []
  );
});

test("account source-contract failures stay closed-token and private-data-free", () => {
  const message = formatViolation({
    kind: "source-contract",
    rule: "core.account.restore_trace",
    message: "missing required restore marker"
  });

  assert.equal(
    message,
    "core.account.restore_trace: missing required restore marker"
  );
  assert.doesNotMatch(message, /SECRET|@|!|synthetic-room|private-path/);
});

test("src-tauri source-contract failures stay closed-token and private-data-free", () => {
  const message = formatViolation({
    kind: "source-contract",
    rule: "desktop.native.window_lifecycle_contract",
    message: "missing required lifecycle marker"
  });

  assert.equal(
    message,
    "desktop.native.window_lifecycle_contract: missing required lifecycle marker"
  );
  assert.doesNotMatch(message, /SECRET|@|!|synthetic-room|private-path/);
});

test("formats source-contract failures without source contents", () => {
  const message = formatViolation({
    kind: "source-contract",
    rule: "sdk.room_read_marker_contract",
    message: "mark_room_as_read is missing private_read_receipt"
  });

  assert.equal(
    message,
    "sdk.room_read_marker_contract: mark_room_as_read is missing private_read_receipt"
  );
  assert.doesNotMatch(message, /SECRET|@|!/);
});

test("allows exactly the four current non-Rust artifacts and keeps source diagnostics private", () => {
  const repositoryRoot = "/workspace";
  const sources = [
    ["crates/koushi-state/tests/focused_context_state.rs", 'include_str!("../../../docs/architecture/state-machine.md");'],
    ["apps/desktop/src-tauri/src/lib.rs", 'include_str!("../capabilities/windows-overlay.json");'],
    ["apps/desktop/src-tauri/src/core_event_forwarder.rs", 'include_str!("../../src/domain/coreEvents.generated.json");\ninclude_str!("../../src/domain/coreEvents.generated.json");'],
    ["apps/desktop/src-tauri/src/lib.rs", 'include_str!("lib.rs");']
  ];
  const analyses = sources.map(([filePath, source]) => analyzeRustSource(source, { filePath, repositoryRoot }));
  const includes = analyses.flatMap(({ includes }) => includes);

  assert.equal(includes.length, 5);
  assert.equal(includes.filter(({ allowedNonRust }) => allowedNonRust).length, 4);
  assert.equal(includes.filter(({ rustSource }) => rustSource).length, 1);
  const violation = analyses.at(-1).violations[0];
  assert.match(formatViolation(violation), /lib\.rs/);
  assert.doesNotMatch(formatViolation(violation), /SECRET/);
});
