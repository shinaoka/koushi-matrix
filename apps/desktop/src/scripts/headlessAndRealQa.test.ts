import { existsSync,readFileSync } from "node:fs";
import { describe,expect,test } from "vitest";

import { readLinuxProductionSource,readRealHomeserverProductionSource,runScript } from "./releaseTestSupport";

describe("desktop release scripts", () => {
  test("macOS Keychain Tier 2 workflow stays disabled while retaining the temporary-keychain recipe", () => {
    const workflowUrl = new URL(
      "../../../../.github/workflows/macos-keychain-tier2.yml",
      import.meta.url
    );
    const disabledWorkflowUrl = new URL(
      "../../../../.github/workflows.disabled/macos-keychain-tier2.yml",
      import.meta.url
    );

    expect(existsSync(workflowUrl)).toBe(false);
    expect(existsSync(disabledWorkflowUrl)).toBe(true);

    const workflow = readFileSync(disabledWorkflowUrl, "utf8");

    for (const token of [
      "workflow_dispatch:",
      "runs-on: macos-latest",
      "uses: actions/checkout@v6",
      "Prepare standalone key crate",
      'cp -R crates/koushi-key/. "$RUNNER_TEMP/koushi-key/"',
      'KOUSHI_MACOS_KEYCHAIN_QA: "1"',
      'cargo test --manifest-path "$RUNNER_TEMP/koushi-key/Cargo.toml" credential_backend_macos_temporary_keychain_round_trip_is_env_gated -- --nocapture',
      'cargo test --manifest-path "$RUNNER_TEMP/koushi-key/Cargo.toml" credential_backend'
    ]) {
      expect(workflow).toContain(token);
    }

    expect(workflow).not.toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR");
    expect(workflow).not.toContain("submodules:");
  });

  test("release preflight validates linux GUI smoke entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:linux-gui");
  });

  test("release preflight validates real account QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:real-account");
  });

  test("real homeserver QA runner forwards scenario selection to the binary", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--scenario");
    expect(source).toContain("KOUSHI_REAL_QA_SCENARIO");
    expect(source).toContain("compat|space_compat|all");
  });

  test("real homeserver QA binary names the staged real-server scenarios", () => {
    const source = readRealHomeserverProductionSource();

    expect(source).toContain("KOUSHI_REAL_QA_SCENARIO");
    expect(source).toContain("RealQaScenario");
    expect(source).toContain("SpaceCompat");
    expect(source).toContain("All");
  });

  test("real homeserver QA treats space projection as an observation token", () => {
    const source = readRealHomeserverProductionSource();

    expect(source).toContain("real_space_projection=observed");
    expect(source).toContain("real_space_projection=not_observed");
  });

  test("real homeserver QA runner enforces the private-data-free token contract", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("./lib/qa-token-contract.mjs");
    expect(source).toContain("assertNoMatrixIdentifiers");
    expect(source).toContain("assertNoLocalPaths");
    expect(source).toContain("assertNoRawSdkErrors");
    expect(source).toContain("assertRequiredTokens");
    expect(source).toContain("requiredTokensForScenario");
  });

  test("real homeserver QA runner checks private data before writing artifacts", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    const writeLogOffset = source.indexOf("writeFileSync(logPath");
    const matrixIdCheckOffset = source.indexOf("assertNoMatrixIdentifiers(combinedOutput");
    const localPathCheckOffset = source.indexOf("assertNoLocalPaths(combinedOutput");

    expect(matrixIdCheckOffset).toBeGreaterThan(-1);
    expect(localPathCheckOffset).toBeGreaterThan(-1);
    expect(writeLogOffset).toBeGreaterThan(-1);
    expect(matrixIdCheckOffset).toBeLessThan(writeLogOffset);
    expect(localPathCheckOffset).toBeLessThan(writeLogOffset);
  });

  test("real homeserver QA runner stdout omits local paths and raw child output", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).not.toContain("run dir = ${runDir}");
    expect(source).not.toContain("credentials file = ${credentialsPath}");
    expect(source).not.toContain("stdout: ${stdout");
    expect(source).not.toContain("stderr: ${stderr");
    expect(source).not.toContain("log: ${logPath}");
    expect(source).not.toContain("PASSED. Log");
    expect(source).toContain("child output omitted after private-data validation");
  });

  test("real homeserver QA binary emits private-data-free tokens (no Matrix ids)", () => {
    const source = readRealHomeserverProductionSource();

    // No token line or summary may interpolate a Matrix identifier.
    expect(source).not.toContain("event_id={");
    expect(source).not.toContain("user_id={");
    expect(source).not.toContain("room_id={");
    expect(source).not.toContain("space_id={");
    expect(source).not.toContain("user={user_id}");
    expect(source).not.toContain("{expected_event_id}");
    expect(source).not.toContain("{space_id}");
    expect(source).not.toContain("{child_room_id}");
    expect(source).not.toContain("space={ev_space}");
    expect(source).not.toContain("child={ev_child}");
  });

  test("qa token contract helper exposes token and private-data assertions", () => {
    const source = readFileSync(
      new URL("../../../../scripts/lib/qa-token-contract.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("export function tokensFromOutput");
    expect(source).toContain("export function assertRequiredTokens");
    expect(source).toContain("export function assertNoMatrixIdentifiers");
    expect(source).toContain("export function assertNoLocalPaths");
    expect(source).toContain("export function assertNoRawSdkErrors");
    expect(source).not.toContain("${match[1]}");
  });

  test("assertNoRawSdkErrors rejects raw SDK shapes without echoing payloads", async () => {
    const { assertNoRawSdkErrors } = await import(
      "../../../../scripts/lib/qa-token-contract.mjs"
    );
    const sentinel = "PRIVATE-SENTINEL-IDENTIFIER:example.invalid";
    const cases = [
      `warning: matrix_sdk::${sentinel}`,
      `matrix_sdk_base::${sentinel}`,
      `ruma::${sentinel}`,
      `reqwest::${sentinel}`,
      `hyper::${sentinel}`,
      `SdkError ${sentinel}`,
      `HttpError ${sentinel}`,
      `ClientApiError ${sentinel}`,
      `StoreError ${sentinel}`,
      `ServerError ${sentinel}`,
      `M_UNKNOWN ${sentinel}`
    ];
    for (const input of cases) {
      let message = "";
      try {
        assertNoRawSdkErrors(input, "test");
      } catch (error) {
        message = String((error as Error).message);
      }
      expect(message).toContain("raw SDK diagnostic leaked");
      // The rejection must never echo the source line/payload.
      expect(message).not.toContain(sentinel);
    }
    // Clean output passes.
    expect(() => assertNoRawSdkErrors("clean QA output", "test")).not.toThrow();
  });

  test("release preflight validates headless local QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:headless-local");
  });

  test("package scripts expose the headless basic QA aggregators", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    const localHeadlessCoreReleaseQa =
      "node ../../scripts/desktop-headless-local-qa.mjs --run --server=both --core --scenario=login_sync,directory,timeline_reconnect,send_queue --timeout-ms=600000 --cargo-profile=release";

    expect(packageJson.scripts?.["qa:headless-basic:local"]).toBe(localHeadlessCoreReleaseQa);
    expect(packageJson.scripts?.["qa:headless-basic:real"]).toBe(
      "node ../../scripts/desktop-real-homeserver-qa.mjs --run --scenario=space_compat"
    );
  });

  test("redact/edit convergence QA is registered with its closed token", () => {
    const registry = readFileSync(
      new URL("../../../../crates/koushi-core/src/bin/headless_core_qa/registry.rs", import.meta.url),
      "utf8"
    );
    const runner = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );
    const qaLanes = readFileSync(
      new URL("../../../../docs/agents/qa-lanes.md", import.meta.url),
      "utf8"
    );

    expect(registry).toContain("RedactEditConvergence");
    expect(registry).toContain('"redact_edit_convergence"');
    expect(runner).toContain("scenario redact_edit_convergence");
    expect(qaLanes).toContain("redact_edit_convergence=ok");
  });

  test("headless basic operations docs list the default real space_compat tokens", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    for (const token of [
      "login=ok",
      "sync=running",
      "real_reply=ok",
      "real_space_create=ok",
      "real_space_child=ok",
      "real_space_cleanup=ok",
      "logout=ok",
      "post_logout_restore=not_found"
    ]) {
      expect(docs).toContain(token);
    }
  });

  test("headless basic operations docs list the Phase 11 local thread tokens", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    for (const token of [
      "thread_hidden=ok",
      "thread_summary=ok",
      "thread_recv=ok",
      "thread_paginate=end_reached"
    ]) {
      expect(docs).toContain(token);
    }
    expect(docs).not.toContain("thread=ok");
  });

  test("package scripts expose the linux GUI smoke runner", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );

    expect(packageJson.scripts?.["qa:linux-gui"]).toBe(
      "node ../../scripts/desktop-linux-gui-qa.mjs --run"
    );
  });

  test("linux GUI smoke script lists the expected foundation checks", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", ["--list"]);

    for (const check of [
      "verify Xvfb virtual display",
      "verify tauri-driver and WebKitWebDriver",
      "verify debug Tauri build",
      "drive WebdriverIO session",
      "exercise real IPC and DOM smoke",
      "optional local homeserver login via FIFO",
      "clean process teardown"
    ]) {
      expect(output).toContain(check);
    }
  });

  test("linux GUI smoke lists the local-login and local-send scenarios", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", ["--list"]);

    for (const token of ["signed-out", "local-login", "local-send"]) {
      expect(output).toContain(token);
    }
  });

  test("linux GUI smoke lists the local basic-operation scenarios", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", ["--list"]);

    for (const token of [
      "scenario local-create-room",
      "scenario local-create-space",
      "scenario local-invites-dm",
      "scenario local-reply",
      "scenario local-media",
      "scenario local-room-tags",
      "scenario local-room-management",
      "scenario local-explore",
      "scenario local-message-actions",
      "scenario local-pins",
      "scenario local-composer",
      "scenario local-scheduled-send",
      "scenario local-timeline-navigation",
      "scenario local-alias",
      "scenario local-cjk",
      "scenario local-settings",
      "verify local-settings trust section"
    ]) {
      expect(output).toContain(token);
    }
  });

  test("linux GUI smoke supports the fast skip-build inner loop", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("--skip-build");
    expect(source).toContain("--app-binary");
    expect(source).toContain("async function ensureAppBinary(");
  });

  test("linux GUI smoke source emits the basic-operation success tokens", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("gui_local_create_room=ok");
    expect(source).toContain("gui_local_create_space=ok");
    expect(source).toContain("gui_local_invite_accept=ok");
    expect(source).toContain("gui_local_dm_start=ok");
    expect(source).toContain("gui_local_reply=ok");
    expect(source).toContain("gui_local_media=ok");
    expect(source).toContain("gui_local_room_tag_set=ok");
    expect(source).toContain("gui_local_room_tag_removed=ok");
    expect(source).toContain("gui_local_room_topic=ok");
    expect(source).toContain("gui_local_room_kick=ok");
    expect(source).toContain("gui_local_message_source=ok");
    expect(source).toContain("gui_local_message_forward=ok");
    expect(source).toContain("gui_local_hide_redacted=ok");
    expect(source).toContain("gui_local_mention=ok");
    expect(source).toContain("gui_local_markdown=ok");
    expect(source).toContain("gui_local_slash=ok");
    expect(source).toContain("gui_local_scheduled_create=ok");
    expect(source).toContain("gui_local_scheduled_reschedule=ok");
    expect(source).toContain("gui_local_scheduled_cancel=ok");
    expect(source).toContain("gui_local_settings=ok");
    expect(source).toContain("gui_local_trust_settings=ok");
  });

  test("linux GUI composer smoke drives real controls without IPC mocking", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("export async function runLocalComposerScenario()");
    expect(source).toContain('.composer-inline-editor[role="textbox"]');
    expect(source).not.toContain('textarea[aria-label="Message composer"]');
    expect(source).toContain("range.selectNodeContents(editor)");
    expect(source).toContain('editable.textContent ?? ""');
    expect(source).toContain("expected private-safe editable state");
    expect(source).not.toContain("waitForTextareaValue");
    expect(source).not.toContain("innerText");
    expect(source).not.toContain("setSelectionRange");
    expect(source).toContain('button[role="option"]');
    expect(source).toContain('button[aria-label="Bold"]');
    expect(source).toContain("Mention Helper");
    expect(source).toContain("sendRoomMessage(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI room-tag smoke drives context menu and Rust-owned section movement", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("export async function runLocalRoomTagsScenario()");
    expect(source).toContain('button[data-testid="room-item"]');
    expect(source).toContain('button[role="menuitem"]');
    expect(source).toContain("Add to Favourites");
    expect(source).toContain("Remove from Favourites");
    expect(source).toContain('data-room-section="favourites"');
    expect(source).toContain('data-room-section="rooms"');
    expect(source).toContain("waitForRoomInSection(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI room-management smoke drives Rust-owned settings and member state", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("export async function runLocalRoomManagementScenario()");
    expect(source).toContain('textarea[aria-label="Room topic"]');
    expect(source).toContain("Save topic");
    expect(source).toContain(".settings-detail-row");
    expect(source).toContain(".room-member-row");
    expect(source).toContain('button[data-action="kick"]');
    expect(source).toContain("waitForRoomManagementTopic(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI message-action smoke drives real action menu controls", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("export async function runLocalMessageActionsScenario()");
    expect(source).toContain("waitForLatestMessageActionButton(");
    expect(source).toContain('button[aria-label="Message actions"]');
    expect(source).toContain("View source");
    expect(source).toContain("Message source");
    expect(source).toContain("Forward");
    expect(source).toContain("Redact message");
    expect(source).toContain("Hide deleted messages");
    expect(source).toContain('.message[data-redacted="true"]');
    expect(source).toContain("QA Seed Room");
    expect(source).toContain("QA message action seed");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI media smoke drives the hidden file input without a native dialog", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("setSyntheticFileInput(");
    expect(source).toContain("makeFileInputInteractable(");
    expect(source).toContain("dispatchFileInputChange(");
    expect(source).toContain("DataTransfer");
    expect(source).toContain(".message-media");
    expect(source).toContain("Download ${filename}");
    expect(source).not.toContain("verifyTauriInvokeRecorder(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("headless basic operations docs mention the local create, reply, and media GUI scenarios", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    expect(docs).toContain("--scenario=local-create-room");
    expect(docs).toContain("--scenario=local-create-space");
    expect(docs).toContain("--scenario=local-invites-dm");
    expect(docs).toContain("--scenario=local-reply");
    expect(docs).toContain("--scenario=local-media");
    expect(docs).toContain("--scenario=local-room-tags");
    expect(docs).toContain("--scenario=local-room-management");
    expect(docs).toContain("--scenario=local-explore");
    expect(docs).toContain("--scenario=local-message-actions");
    expect(docs).toContain("--scenario=local-pins");
    expect(docs).toContain("--scenario=local-composer");
    expect(docs).toContain("--scenario=local-scheduled-send");
    expect(docs).toContain("--scenario=local-timeline-navigation");
    expect(docs).toContain("--scenario=local-alias");
    expect(docs).toContain("--scenario=local-cjk");
    expect(docs).toContain("--scenario=local-settings");
    expect(docs).toContain("gui_local_create_room=ok");
    expect(docs).toContain("gui_local_invite_accept=ok");
    expect(docs).toContain("gui_local_dm_start=ok");
    expect(docs).toContain("gui_local_reply=ok");
    expect(docs).toContain("gui_local_media=ok");
    expect(docs).toContain("gui_local_room_tag_set=ok");
    expect(docs).toContain("gui_local_room_tag_removed=ok");
    expect(docs).toContain("gui_local_room_topic=ok");
    expect(docs).toContain("gui_local_room_kick=ok");
    expect(docs).toContain("gui_local_message_source=ok");
    expect(docs).toContain("gui_local_message_forward=ok");
    expect(docs).toContain("gui_local_hide_redacted=ok");
    expect(docs).toContain("gui_local_mention=ok");
    expect(docs).toContain("gui_local_scheduled_create=ok");
    expect(docs).toContain("gui_local_scheduled_reschedule=ok");
    expect(docs).toContain("gui_local_scheduled_cancel=ok");
    expect(docs).toContain("gui_local_markdown=ok");
    expect(docs).toContain("gui_local_slash=ok");
    expect(docs).toContain("gui_local_alias_set=ok");
    expect(docs).toContain("gui_local_alias_clear=ok");
    expect(docs).toContain("gui_local_cjk=ok");
    expect(docs).toContain("gui_local_settings=ok");
    expect(docs).toContain("gui_local_trust_settings=ok");
  });
});
