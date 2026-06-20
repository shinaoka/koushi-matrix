import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";

const repoRoot = new URL("../../../../", import.meta.url).pathname;

function runScript(script: string, args: string[] = []): string {
  return execFileSync(process.execPath, [script, ...args], {
    cwd: repoRoot,
    encoding: "utf8"
  });
}

function gitTrackedFiles(): string[] {
  return execFileSync("git", ["ls-files"], {
    cwd: repoRoot,
    encoding: "utf8"
  })
    .split("\n")
    .map((file) => file.trim())
    .filter(Boolean);
}

describe("desktop release scripts", () => {
  test("tracked text artifacts contain no previous branding residue", () => {
    const oldLatinBrand = "Ru" + "ri";
    const oldLowerBrand = oldLatinBrand.toLowerCase();
    const oldJapaneseBrand = "瑠" + "璃";
    const pattern = new RegExp(`${oldLatinBrand}|${oldLowerBrand}|${oldJapaneseBrand}`);
    const binaryExtensions = new Set([
      ".png",
      ".jpg",
      ".jpeg",
      ".gif",
      ".webp",
      ".ico",
      ".icns",
      ".woff",
      ".woff2",
      ".ttf",
      ".otf",
      ".zst"
    ]);
    // Files that intentionally mention prior branding for documentation/history.
    const intentionalPreviousBrandReferences = new Set(["README.md"]);
    const findings: string[] = [];

    for (const file of gitTrackedFiles()) {
      const extension = file.includes(".") ? file.slice(file.lastIndexOf(".")).toLowerCase() : "";
      if (binaryExtensions.has(extension)) {
        continue;
      }
      if (intentionalPreviousBrandReferences.has(file)) {
        continue;
      }
      let contents: string;
      try {
        contents = readFileSync(new URL(`../../../../${file}`, import.meta.url), "utf8");
      } catch {
        continue;
      }
      if (pattern.test(contents)) {
        findings.push(file);
      }
    }

    expect(findings).toEqual([]);
  });

  test("release preflight validates installer and signing preparation", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("bundle.active");
    expect(output).toContain("dmg");
    expect(output).toContain("msi");
    expect(output).toContain("nsis");
    expect(output).toContain("macOS.hardenedRuntime");
    expect(output).toContain("windows.signCommand");
    expect(output).toContain("windows.wix.upgradeCode");
  });

  test("manual QA script lists every Milestone 9 flow", () => {
    const output = runScript("scripts/desktop-manual-qa.mjs", ["--list"]);

    for (const flow of [
      "login",
      "restore",
      "recovery",
      "search",
      "edit",
      "redaction",
      "logout",
      "account switch",
      "shortcut parity",
      "right-panel behavior",
      "settings placement",
      "Space info/settings"
    ]) {
      expect(output).toContain(flow);
    }
  });

  test("mac GUI smoke script lists automated first-run checks", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--list"]);

    for (const check of [
      "launch Tauri dev shell",
      "verify main window",
      "optional real login from stdin",
      "optional reusable QA profile for restored sync state",
      "optional synthetic send smoke message",
      "verify QA title panel token after shortcuts",
      "open Keyboard settings shortcut",
      "open User settings shortcut",
      "capture private-data-free screenshots",
      "stop app process group"
    ]) {
      expect(output).toContain(check);
    }
  });

  test("mac GUI smoke script parses the QA panel token without launching the GUI", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings"
    ]);

    expect(output.trim()).toBe("keyboardSettings");
  });

  test("mac GUI smoke only skips panel checks while recovery owns the panel", () => {
    const readyRecoveryPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=recovery",
      "--required-panel=keyboardSettings"
    ]);
    const recoveryPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=needsRecovery sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=recovery",
      "--required-panel=keyboardSettings"
    ]);
    const keyboardPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);
    const erroredPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=1 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);

    expect(readyRecoveryPanel.trim()).toBe("not-ready");
    expect(recoveryPanel.trim()).toBe("ready");
    expect(keyboardPanel.trim()).toBe("ready");
    expect(erroredPanel.trim()).toBe("not-ready");
  });

  test("release preflight validates mac GUI smoke entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:mac-gui");
  });

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
    const source = readFileSync(
      new URL("../../../../crates/koushi-core/src/bin/real-homeserver-qa.rs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("KOUSHI_REAL_QA_SCENARIO");
    expect(source).toContain("RealQaScenario");
    expect(source).toContain("SpaceCompat");
    expect(source).toContain("All");
  });

  test("real homeserver QA treats space projection as an observation token", () => {
    const source = readFileSync(
      new URL("../../../../crates/koushi-core/src/bin/real-homeserver-qa.rs", import.meta.url),
      "utf8"
    );

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
    const source = readFileSync(
      new URL("../../../../crates/koushi-core/src/bin/real-homeserver-qa.rs", import.meta.url),
      "utf8"
    );

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

  test("release preflight validates headless local QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:headless-local");
  });

  test("package scripts expose the headless basic QA aggregators", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );

    expect(packageJson.scripts?.["qa:headless-basic:local"]).toBe(
      "node ../../scripts/desktop-headless-local-qa.mjs --run --server=both --core --scenario=all --timeout-ms=240000"
    );
    expect(packageJson.scripts?.["qa:headless-basic:real"]).toBe(
      "node ../../scripts/desktop-real-homeserver-qa.mjs --run --scenario=space_compat"
    );
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
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--skip-build");
    expect(source).toContain("--app-binary");
    expect(source).toContain("async function ensureAppBinary(");
  });

  test("linux GUI smoke source emits the basic-operation success tokens", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

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
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalComposerScenario()");
    expect(source).toContain('textarea[aria-label="Message composer"]');
    expect(source).toContain('button[role="option"]');
    expect(source).toContain('button[aria-label="Bold"]');
    expect(source).toContain("Mention Helper");
    expect(source).toContain("sendRoomMessage(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI room-tag smoke drives context menu and Rust-owned section movement", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalRoomTagsScenario()");
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
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalRoomManagementScenario()");
    expect(source).toContain('textarea[aria-label="Room topic"]');
    expect(source).toContain("Save topic");
    expect(source).toContain(".settings-detail-row");
    expect(source).toContain(".room-member-row");
    expect(source).toContain('button[data-action="kick"]');
    expect(source).toContain("waitForRoomManagementTopic(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI message-action smoke drives real action menu controls", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalMessageActionsScenario()");
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
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

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

  test("linux GUI smoke resolves relative artifact dirs from the repo root", () => {
    const output = execFileSync(
      process.execPath,
      [
        "../../scripts/desktop-linux-gui-qa.mjs",
        "--print-artifact-root",
        "--artifact-dir=artifacts/linux-gui-local-login"
      ],
      {
        cwd: `${repoRoot}apps/desktop`,
        encoding: "utf8"
      }
    );

    expect(output.trim()).toBe(
      new URL("../../../../artifacts/linux-gui-local-login", import.meta.url).pathname
    );
  });

  test("linux GUI smoke source emits the local scenario success tokens", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("gui_local_login=ok");
    expect(source).toContain("gui_local_send=ok");
    expect(source).toContain("gui_local_logout=ok");
    expect(source).toContain("gui_local_relogin=ok");
    expect(source).toContain("gui_local_spaces_home=ok");
    expect(source).toContain("gui_local_spaces_nav=ok");
    expect(source).toContain("gui_local_spaces_info=ok");
    expect(source).toContain("gui_local_explore_query=ok");
    expect(source).toContain("gui_local_explore_join=ok");
    expect(source).toContain("gui_local_room_topic=ok");
    expect(source).toContain("gui_local_room_kick=ok");
    expect(source).toContain("gui_local_alias_set=ok");
    expect(source).toContain("gui_local_alias_clear=ok");
    expect(source).toContain("gui_local_scheduled_create=ok");
    expect(source).toContain("gui_local_scheduled_cancel=ok");
    expect(source).toContain("gui_local_timeline_unread_jump=ok");
    expect(source).toContain("gui_local_timeline_date_jump=ok");
    expect(source).toContain("waitForTimelineFocusedContextReady");
    expect(source).toContain("timelineDateJumpDiagnostics");
    expect(source).toContain("setDatetimeLocalValue");
    expect(source).toContain("gui_local_cjk=ok");
  });

  test("linux GUI local logout/relogin uses the gated QA control pipe", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("local-logout-relogin");
    expect(source).toContain("KOUSHI_QA_CONTROL_PIPE");
    expect(source).toContain("qa-control.pipe");
    expect(source).toContain('JSON.stringify({ command: "logout" })');
    expect(source).toContain("requestQaLogout");
    expect(source).toContain("submitLoginForm");
    expect(source).toMatch(
      /function childEnvironment\(dataDir, qaLoginPipePath = null, qaControlPipePath = null\)/
    );
    expect(source).toMatch(
      /if \(qaControlPipePath\) \{[\s\S]*env\.KOUSHI_QA_CONTROL_PIPE = qaControlPipePath;/
    );
  });

  test("linux GUI local spaces navigation checks rail selection and space info", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("local-spaces-nav");
    expect(source).toContain("waitForWorkspaceActive");
    expect(source).toContain("clickWorkspaceButton");
    expect(source).toContain("gui_local_spaces_home=ok");
    expect(source).toContain("gui_local_spaces_nav=ok");
    expect(source).toContain("gui_local_spaces_info=ok");
  });

  test("linux GUI local scenarios also emit DBus and window-state evidence", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("recordLocalGuiEvidence");
    expect(source).toContain("notification_dbus=ok");
    expect(source).toContain("window_state_path_contract=ok");
    expect(source).toContain("run_dir=artifact");
    expect(source).not.toContain("window_state_path=${");
    expect(source).not.toContain("run_dir=${");
    expect(source).toMatch(
      /async function runLocalLoginScenario\(\)[\s\S]*await recordLocalGuiEvidence\(session\);[\s\S]*gui_local_login=ok/
    );
    expect(source).toMatch(
      /async function runLocalSendScenario\(\)[\s\S]*await recordLocalGuiEvidence\(session\);[\s\S]*gui_local_send=ok/
    );
  });

  test("linux GUI local login selects the first room when timeline subscription is still missing", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("shouldSelectFirstRoom(status, selectedRoom)");
    expect(source).toMatch(
      /function shouldSelectFirstRoom\(status, selectedRoom\)[\s\S]*status\.active_room === false \|\| status\.timeline_subscribed === false/
    );
    expect(source).toMatch(
      /if \(shouldSelectFirstRoom\(status, selectedRoom\)\) \{[\s\S]*selectedRoom = await selectFirstRoom\(browser\);/
    );
  });

  test("linux GUI smoke parses the attention baseline title token", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-attention-ready=koushi-desktop qa session=signedOut sync=stopped rooms=0 spaces=0 active_room=false timeline_subscribed=false timeline_items=0 errors=0 unread=0 badge=0 notify=none"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("linux GUI smoke validates the persisted window-state path contract", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-window-state-ready=/tmp/koushi-desktop/app-shell/window-state.json"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("linux GUI smoke wires dbus notification evidence into the signed-out run path", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("dbus-daemon");
    expect(source).toContain("--session");
    expect(source).toContain("--address");
    expect(source).toContain("dbus-monitor");
    expect(source).toContain("NSS_WRAPPER_PASSWD");
    expect(source).toContain("notification_dbus=ok");
    expect(source).toContain("triggerNotificationSmoke");
  });

  test("linux GUI smoke child environment filters secrets and enables QA file credentials", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-linux-gui-qa.mjs", "--child-env"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("KOUSHI_DATA_DIR=");
    expect(output).toContain("KOUSHI_QA_TITLE=1");
    expect(output).toContain("VITE_KOUSHI_QA_TITLE=1");
    expect(output).toContain("KOUSHI_SKIP_SAVED_SESSIONS=1");
    expect(output).toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain("/qa-credential-store");
    expect(output).toContain("NO_COLOR=1");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("linux GUI smoke child environment exposes only safe QA keys for local login", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-linux-gui-qa.mjs", "--child-env-keys", "--real-login-from-stdin"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("KOUSHI_DATA_DIR");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR");
    expect(output).toContain("KOUSHI_QA_LOGIN_PIPE");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("linux GUI smoke source wires the shared local homeserver helper module", () => {
    const guiSource = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );
    const sharedSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(guiSource).toContain("local-homeserver-qa.mjs");
    expect(guiSource).toContain("local-login");
    expect(guiSource).toContain("local-send");
    expect(guiSource).not.toContain("--password");
    expect(sharedSource).toContain("checkInstalledHomeserver");
    expect(sharedSource).toContain("registerUser");
    expect(sharedSource).toContain("stopProcess");
  });

  test("local Synapse QA config relaxes room creation limits for synthetic stress seeds", () => {
    const sharedSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(sharedSource).toContain("rc_room_creation:");
    expect(sharedSource).toMatch(/rc_room_creation:\n\s+per_second: 1000\n\s+burst_count: 1000/);
  });

  test("local Synapse QA config allows synthetic public room directory publication", () => {
    const sharedSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(sharedSource).toContain("room_list_publication_rules:");
    expect(sharedSource).toMatch(/room_list_publication_rules:\n\s+- action: allow/);
  });

  test("linux GUI local setup keeps homeserver data separate and cleanup covers setup failures", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("serverDataDir");
    expect(source).toContain("homeserver-data");
    expect(source).toContain("const session = {");
    expect(source).toContain("await cleanupLocalGuiScenario(session)");
  });

  test("linux GUI local setup defines the safe timestamp helper it uses for synthetic users", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const userSuffix = safeTimestamp();");
    expect(source).toContain("function safeTimestamp()");
    expect(source).toContain("replaceAll(\"-\", \"_\")");
  });

  test("linux GUI smoke real login transport is FIFO and the script avoids password args", () => {
    const transport = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--print-real-login-transport"
    ]);
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(transport.trim()).toBe("fifo");
    expect(source).toContain("KOUSHI_QA_LOGIN_PIPE");
    expect(source).not.toContain("--password");
  });

  test("linux GUI smoke prints WebDriver capabilities for the app binary", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--print-webdriver-capabilities",
      "--app-binary=/tmp/app"
    ]);

    expect(JSON.parse(output)).toEqual(
      expect.objectContaining({
        browserName: "wry",
        "wdio:enforceWebDriverClassic": true,
        "tauri:options": expect.objectContaining({
          application: "/tmp/app"
        })
      })
    );
    expect(JSON.parse(output)["tauri:options"]).not.toHaveProperty("args");
  });

  test("linux GUI smoke run path now wires WebdriverIO and the signed-out screenshot", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("webdriverio");
    expect(source).toContain("createRequire(new URL(\"../apps/desktop/package.json\"");
    expect(source).toContain("importDesktopWebdriverio");
    expect(source).toContain("remote({");
    expect(source).toContain("screenshots/01-signed-out.png");
    expect(source).not.toContain("foundation is wired, but the WebDriver session");
  });

  test("linux GUI smoke launches Xvfb with the sanitized child environment", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const xvfb = await startXvfb(logPath, buildEnv);");
    expect(source).toContain("async function startXvfb(logPath, buildEnv)");
    expect(source).toContain("env: buildEnv");
    expect(source).not.toContain("env: process.env");
  });

  test("linux GUI Docker recipe pins Rust 1.96.0 and keeps the tauri-driver mitigation", () => {
    const dockerfile = readFileSync(
      new URL("../../../../docker/linux-gui.Dockerfile", import.meta.url),
      "utf8"
    );

    for (const token of [
      "node:22.22.3-bookworm",
      "ARG RUST_TOOLCHAIN=1.96.0",
      "ARG CONDUIT_URL=https://gitlab.com/api/v4/projects/famedly%2Fconduit/jobs/artifacts/master/raw/x86_64-unknown-linux-musl?job=artifacts",
      "ARG TUWUNEL_VERSION=v1.7.1",
      "ARG TUWUNEL_ZST_URL=https://github.com/matrix-construct/tuwunel/releases/download/v1.7.1/v1.7.1-release-all-x86_64-v1-linux-gnu-tuwunel.zst",
      "RUST_TOOLCHAIN=${RUST_TOOLCHAIN}",
      '--default-toolchain "${RUST_TOOLCHAIN}"',
      'rustup default "${RUST_TOOLCHAIN}"',
      'RUSTUP_TOOLCHAIN="${RUST_TOOLCHAIN}"',
      "libwebkit2gtk-4.1-dev",
      "libayatana-appindicator3-dev",
      "zstd",
      "webkit2gtk-driver",
      "xvfb",
      "fonts-noto-color-emoji",
      "cargo install tauri-driver --locked",
      "curl --proto '=https' --tlsv1.2 -fsSLo /usr/local/bin/conduit",
      "curl --proto '=https' --tlsv1.2 -fsSLo /tmp/tuwunel.zst",
      "unzstd -f -o /usr/local/bin/tuwunel /tmp/tuwunel.zst",
      "conduit --version",
      "tuwunel --version",
      "RUSTC=\"$(rustup which rustc)\"",
      "RUSTDOC=\"$(rustup which rustdoc)\""
    ]) {
      expect(dockerfile).toContain(token);
    }
  });

  test("linux GUI container docs use bash -c and the audited artifact lane", () => {
    const agents = readFileSync(new URL("../../../../AGENTS.md", import.meta.url), "utf8");

    expect(agents).toContain("bash -c");
    expect(agents).not.toContain("bash -lc");
    expect(agents).toContain('-u "$(id -u):$(id -g)"');
    expect(agents).toContain("-v /tmp/koushi-desktop-cargo-home:/tmp/cargo-home");
    expect(agents).toContain("-v /tmp/koushi-desktop-gui-target:/tmp/koushi-desktop-gui-target");
    expect(agents).toContain("-v /tmp/koushi-desktop-npm-cache:/tmp/npm-cache");
    expect(agents).toContain("CARGO_HOME=/tmp/cargo-home");
    expect(agents).toContain("CARGO_TARGET_DIR=/tmp/koushi-desktop-gui-target");
    expect(agents).toContain("NPM_CONFIG_CACHE=/tmp/npm-cache");
    expect(agents).toContain("koushi-desktop-linux-gui:basic-ops");
    expect(agents).toContain("--scenario=local-send");
    expect(agents).toContain("--server=conduit");
    expect(agents).toContain("--artifact-dir=/work/artifacts/linux-gui-local-send-docker");
    expect(agents).toContain("--timeout-ms=180000");
    expect(agents).toContain("conduit");
    expect(agents).toContain("tuwunel");
    expect(agents).toContain("zstd");
    expect(agents).toContain("PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
    expect(agents).toContain("RUSTC=\"$(rustup which rustc)\"");
    expect(agents).toContain("RUSTDOC=\"$(rustup which rustdoc)\"");
  });

  test("linux GUI smoke QA title helpers match the mac runner contract", () => {
    const ready = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);
    const readyRecovered = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready-require-recovered=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);
    const panel = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-panel=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings"
    ]);
    const panelReady = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);
    const sendReady = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 send=sent panel=closed"
    ]);

    expect(ready.trim()).toBe("ready");
    expect(readyRecovered.trim()).toBe("ready");
    expect(panel.trim()).toBe("keyboardSettings");
    expect(panelReady.trim()).toBe("ready");
    expect(sendReady.trim()).toBe("ready");
  });

  test("linux GUI smoke QA title contract uses the local send statuses", () => {
    const titleSource = readFileSync(
      new URL("../../../../apps/desktop/src/domain/qaTitle.ts", import.meta.url),
      "utf8"
    );
    const sendSource = readFileSync(
      new URL("../../../../apps/desktop/src/domain/qaSendSmoke.ts", import.meta.url),
      "utf8"
    );

    expect(titleSource).toContain("send=");
    expect(sendSource).toContain('"idle"');
    expect(sendSource).toContain('"pending"');
    expect(sendSource).toContain('"sent"');
    expect(sendSource).toContain('"failed"');
  });

  test("app wires Tauri CoreEvent send completion into the QA send title token", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(source).toContain("qaSendCompletionStatusFromCoreEvent");
    expect(source).toContain("SendCompleted");
    expect(source).toContain("OperationFailed");
    expect(source).toContain("setQaSendStatus(eventStatus)");
  });

  test("app lets Tauri snapshot errors fail the QA send title token", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(source).toContain('completionStatus !== "failed"');
    expect(source).toMatch(
      /isTauriRuntime\(\) &&\s*completionStatus !== "failed"[\s\S]*return;/
    );
  });

  test("app keeps Tauri send completion listener mounted and gates events with a pending ref", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const qaSendPending = useRef(false)");
    expect(source).toMatch(
      /useEffect\(\(\) => \{[\s\S]*if \(!isTauriRuntime\(\)\) \{[\s\S]*listen<CoreEventPayload>\(CORE_EVENT_NAME,[\s\S]*qaSendPending\.current[\s\S]*qaSendCompletionStatusFromCoreEvent[\s\S]*setQaSendStatus\(eventStatus\);[\s\S]*\}, \[\]\);/
    );
    expect(source).toMatch(
      /qaSendStarted\.current = true;[\s\S]*qaSendPending\.current = true;[\s\S]*setQaSendStatus\("pending"\);/
    );
    expect(source).toMatch(
      /async function sendText\(\)[\s\S]*qaSendPending\.current = true;[\s\S]*setQaSendStatus\("pending"\);/
    );
  });

  test("linux GUI local login retries room selection until a displayed row is clicked", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("selectedRoom = await selectFirstRoom(browser);");
    expect(source).toMatch(
      /async function selectFirstRoom\(browser\)[\s\S]*return false;[\s\S]*await roomItems\[0\]\.click\(\);[\s\S]*return true;/
    );
  });

  test("headless local QA script lists homeserver and SDK checks", () => {
    const output = runScript("scripts/desktop-headless-local-qa.mjs", ["--list"]);

    for (const check of [
      "verify installed Conduit binary",
      "verify installed Tuwunel binary",
      "start disposable local homeserver",
      "register two synthetic local users",
      "run headless Matrix SDK operations",
      "stop disposable local homeserver"
    ]) {
      expect(output).toContain(check);
    }
  });

  test("headless local QA script imports the shared local homeserver helper module", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("local-homeserver-qa.mjs");
    expect(source).toContain("checkInstalledHomeserver");
    expect(source).toContain("registerUser");
    expect(source).toContain("stopProcess");
  });

  test("headless local QA script lists staged scenarios", () => {
    const output = runScript("scripts/desktop-headless-local-qa.mjs", ["--list"]);

    for (const scenario of [
      "scenario safety",
      "scenario login_sync",
      "scenario room_space",
      "scenario directory",
      "scenario room_management",
      "scenario timeline",
      "scenario composer",
      "scenario credential_health",
      "scenario reply",
      "scenario media",
      "scenario thread",
      "scenario edit_redact_search",
      "scenario restore_cleanup"
    ]) {
      expect(output).toContain(scenario);
    }
  });

  test("headless local QA forwards the selected scenario to core QA", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--scenario");
    expect(source).toContain("KOUSHI_QA_SCENARIO");
  });

  test("headless local QA can replay a saved Synapse fixture without mutating the source data", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--fixture-run");
    expect(source).toContain("loadQaFixture");
    expect(source).toContain("copyFixtureDataDir");
    expect(source).toContain("KOUSHI_QA_STRESS_REPLAY_EXISTING");
    expect(source).toMatch(/cpSync\(fixture\.dataDir,\s*dataDir,\s*\{[\s\S]*recursive: true/);
    expect(source).not.toContain("-v `${fixture.dataDir}:/data`");
  });

  test("headless local QA stores fixture credentials only under the ignored local secrets run dir", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("fixture.json");
    expect(source).toContain("writeQaFixture");
    expect(source).toContain("serverName");
    expect(source).toContain("passwordA");
    expect(source).toContain("passwordB");
    expect(source).toContain(".local-secrets");
    expect(source).not.toContain("console.log(fixture");
  });

  test("headless local QA runner validates child output before writing artifacts", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    const firstValidation = source.indexOf("assertQaOutputIsPrivate(");
    const firstAppend = source.indexOf("appendQaOutput(");

    expect(source).toContain("./lib/qa-token-contract.mjs");
    expect(source).toContain("assertNoMatrixIdentifiers");
    expect(source).toContain("assertNoLocalPaths");
    expect(source).toContain("assertNoRawSdkErrors");
    expect(firstValidation).toBeGreaterThan(-1);
    expect(firstAppend).toBeGreaterThan(-1);
    expect(firstValidation).toBeLessThan(firstAppend);
  });

  test("headless local QA failure messages do not replay raw child output or paths", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).not.toContain("stdout=${stdout");
    expect(source).not.toContain("stderr=${stderr");
    expect(source).not.toContain("see ${logPath}");
    expect(source).toContain("child output omitted after private-data validation");
  });

  test("headless local QA configs bind only to loopback disposable stores", () => {
    const conduit = runScript("scripts/desktop-headless-local-qa.mjs", [
      "--print-conduit-config"
    ]);
    const tuwunel = runScript("scripts/desktop-headless-local-qa.mjs", [
      "--print-tuwunel-config"
    ]);

    expect(conduit).toContain('address = "127.0.0.1"');
    expect(conduit).toContain('database_path = "/tmp/conduit-data"');
    expect(conduit).toContain("allow_federation = false");
    expect(tuwunel).toContain('address = ["127.0.0.1"]');
    expect(tuwunel).toContain('database_path = "/tmp/tuwunel-data"');
    expect(tuwunel).toContain("allow_federation = false");
  });

  test("headless basic operations docs mention the Linux GUI local scenarios and aggregators", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    expect(docs).toContain("qa:headless-basic:local");
    expect(docs).toContain("qa:linux-gui");
    expect(docs).toContain("--scenario=local-login");
    expect(docs).toContain("--scenario=local-send");
    expect(docs).toContain("gui_local_login=ok");
    expect(docs).toContain("gui_local_send=ok");
  });

  test("headless basic operations docs describe the bundled Linux GUI homeserver binaries", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    expect(docs).toContain("conduit");
    expect(docs).toContain("tuwunel");
    expect(docs).toContain("zstd");
    expect(docs).toContain("unzstd");
  });

  test("mac GUI smoke child environment excludes secret-like variables", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-mac-gui-smoke.mjs", "--child-env-keys"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("PATH");
    expect(output).toContain("KOUSHI_RESTORE_SESSION");
    expect(output).toContain("KOUSHI_SKIP_SAVED_SESSIONS");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("mac GUI smoke can opt into SDK error diagnostics without forwarding secret env values", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-mac-gui-smoke.mjs", "--child-env"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          KOUSHI_DEBUG_SDK_ERROR: "synthetic-secret-value"
        }
      }
    );

    expect(output).toContain("KOUSHI_DEBUG_SDK_ERROR=1");
    expect(output).not.toContain("synthetic-secret-value");
  });

  test("mac GUI smoke real login mode enables QA title without exposing credentials in args", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env-keys",
      "--real-login-from-stdin"
    ]);
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(output).toContain("VITE_KOUSHI_QA_TITLE");
    expect(output).toContain("KOUSHI_QA_TITLE");
    expect(source).toContain("--real-login-from-stdin");
    expect(source).not.toContain("--password");
  });

  test("mac GUI smoke real login uses FIFO transport instead of credential keystrokes", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--print-real-login-transport"
    ]);
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(output.trim()).toBe("fifo");
    expect(source).toContain("KOUSHI_QA_LOGIN_PIPE");
    expect(source).not.toContain("clickAndReplace");
  });

  test("mac GUI smoke real login avoids post-login screenshot artifacts", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("skip real login screenshot");
    expect(source).toContain("skip profile screenshot");
    expect(source).toContain("allowPrivateScreenshots");
    expect(source).toContain("postLoginScreenshotsAreAllowed");
    expect(source).not.toContain("02-real-login.png");
  });

  test("linux GUI smoke can drive real login diagnostics without post-login screenshots", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("runRealLoginScenario");
    expect(source).toContain("readRealLoginCredentials");
    expect(source).toContain("writeRealLoginPipe");
    expect(source).toContain("waitForRealLoginReady");
    expect(source).toContain("collectRealLoginDiagnostics");
    expect(source).toContain("withRealLoginStage");
    expect(source).toContain("real_login_stage=${stage}:start");
    expect(source).toContain("real_login_stage=${stage}:ok");
    expect(source).toContain('withRealLoginStage("auth_screen"');
    expect(source).toContain('withRealLoginStage("write_login_pipe"');
    expect(source).toContain("requestQaLogout");
    expect(source).toContain("skip real login screenshot");
    expect(source).not.toContain("real-login.png");
  });

  test("mac GUI smoke can update the native QA title from the frontend", () => {
    const capability = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
      "utf8"
    );

    expect(capability).toContain("core:window:allow-set-title");
  });

  test("mac GUI smoke has a frontend boot error title before App imports", () => {
    const mainSource = readFileSync(
      new URL("../../../../apps/desktop/src/main.tsx", import.meta.url),
      "utf8"
    );
    const bootCaptureSource = readFileSync(
      new URL("../../../../apps/desktop/src/bootErrorCapture.ts", import.meta.url),
      "utf8"
    );
    const bootImportOffset = mainSource.indexOf("./bootErrorCapture");
    const appImportOffset = mainSource.indexOf("./App");

    expect(bootImportOffset).toBeGreaterThanOrEqual(0);
    expect(appImportOffset).toBeGreaterThanOrEqual(0);
    expect(bootImportOffset).toBeLessThan(appImportOffset);
    expect(bootCaptureSource).toContain("session=booting");
    expect(bootCaptureSource).toContain("session=boot_error");
    expect(bootCaptureSource).toContain("error_kind=");
  });

  test("Tauri dev capability explicitly grants the Vite dev URL", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
        "utf8"
      )
    );

    expect(capability.remote.urls).toContain("http://127.0.0.1:5173/*");
  });

  test("Tauri launch explicitly makes the main WebView window visible", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );
    const setupSource = source
      .split(".setup(move |app|")
      .at(1)
      ?.split(".on_window_event")
      .at(0);

    expect(source).toContain("ensure_main_window_visible");
    expect(setupSource).toContain("ensure_main_window_visible(app)");
    expect(source).toContain("set_activation_policy");
    expect(source).toContain("run_on_main_thread");
    expect(source).toContain("activateIgnoringOtherApps");
    expect(source).toContain("makeKeyAndOrderFront");
    expect(source).toContain("orderFrontRegardless");
    expect(source).toContain("qa_window_visibility_mode_enabled");
    expect(source).toContain("set_visible_on_all_workspaces(true)");
    expect(source).toContain("window.unminimize()");
    expect(source).toContain("window.show()");
    expect(source).toContain("window.set_focus()");
  });

  test("Tauri repeats main window activation after the WebView page loads", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );
    const pageLoadSource = source
      .split(".on_page_load(")
      .at(1)
      ?.split(".on_window_event")
      .at(0);

    expect(pageLoadSource).toContain("ensure_main_window_visible");
    expect(pageLoadSource).toContain('webview.label() == "main"');
  });

  test("mac GUI smoke real login uses the QA file store instead of macOS Keychain", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--real-login-from-stdin"
    ]);

    expect(output).toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain("qa-credential-store");
  });

  test("mac GUI smoke drives a logout cleanup over the QA control pipe for real login", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    // A second debug/test-only FIFO carries control commands to the app.
    expect(source).toContain("KOUSHI_QA_CONTROL_PIPE");
    expect(source).toContain("qa-control.pipe");
    // The runner writes a logout command and waits for a signed-out QA title
    // before terminating the process group (no stale device survives the run).
    expect(source).toContain('JSON.stringify({ command: "logout" })');
    expect(source).toContain("requestQaLogout");
    expect(source).toContain("waitForQaSignedOut");
    expect(source).toContain("--keep-session");
    // The cleanup runs in teardown after credentials were handed to the app:
    // a failed ready gate can still leave a real device/session behind.
    expect(source).toMatch(
      /finally \{[\s\S]*if \(qaControlPipePath && realLoginCleanupRequired && !keepSession\)[\s\S]*requestQaLogout\(qaControlPipePath\);[\s\S]*waitForQaSignedOut\(timeoutMs, diagnostics\);[\s\S]*terminateProcessGroup\(child, "SIGTERM"\);/
    );
  });

  test("mac GUI smoke control pipe rides the filtered child environment, not the parent env", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    // The control pipe path is threaded through the allow-listed childEnvironment
    // helper, never via process.env passthrough.
    expect(source).toContain(
      "childEnvironment(dataDir, qaLoginPipePath, qaControlPipePath)"
    );
    expect(source).toMatch(
      /function childEnvironment\(dataDir, qaLoginPipePath = null, qaControlPipePath = null\)/
    );
    expect(source).toMatch(
      /if \(qaControlPipePath\) \{[\s\S]*env\.KOUSHI_QA_CONTROL_PIPE = qaControlPipePath;/
    );
  });

  test("mac GUI smoke reusable profile keeps restore and saved sessions enabled", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--qa-profile=agent-sync"
    ]);

    expect(output).toContain("KOUSHI_RESTORE_SESSION=1");
    expect(output).toContain("KOUSHI_SKIP_SAVED_SESSIONS=0");
    expect(output).toContain(".local-secrets/qa-profiles/agent-sync/data");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain(".local-secrets/qa-profiles/agent-sync/data/qa-credential-store");
    expect(output).not.toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE");
  });

  test("Tauri debug runtime honors the keychain persistence bypass env", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE");
    expect(source).toContain("keychain_persistence_disabled_from_env");
    expect(source).toContain("CoreRuntime::start_with_data_dir(data_dir.clone())");
    expect(source).toContain("CoreRuntime::start_with_data_dir_and_os_backend");
  });

  test("desktop package exposes a local DMG build script", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    const scriptPath = new URL("../../../../scripts/desktop-build-dmg.mjs", import.meta.url);
    const source = readFileSync(scriptPath, "utf8");

    expect(packageJson.scripts["build:dmg"]).toBe(
      "node ../../scripts/desktop-build-dmg.mjs"
    );
    expect(source).toContain("tauri");
    expect(source).toContain("build");
    expect(source).toContain("--bundles");
    expect(source).toContain("dmg");
    expect(source).toContain("Application Support/koushi-desktop");
    expect(source).toContain("koushi-desktop");
    expect(source).not.toContain("Application Support/matrix-desktop");
  });

  test("active runtime storage identifiers use Koushi without matrix-desktop compatibility", () => {
    const activeSourceFiles = [
      "apps/desktop/src/App.tsx",
      "apps/desktop/src/bootErrorCapture.ts",
      "apps/desktop/src-tauri/src/lib.rs",
      "apps/desktop/src-tauri/src/commands/mod.rs",
      "crates/koushi-core/src/store.rs",
      "crates/koushi-core/src/runtime.rs",
      "crates/koushi-core/src/sync.rs",
      "crates/koushi-core/src/bin/headless-core-qa.rs",
      "crates/koushi-core/src/bin/real-homeserver-qa.rs",
      "crates/koushi-sdk/src/lib.rs",
      "crates/koushi-key/src/lib.rs",
      "scripts/desktop-build-dmg.mjs",
      "scripts/desktop-headless-local-qa.mjs",
      "scripts/desktop-linux-gui-qa.mjs",
      "scripts/desktop-mac-gui-smoke.mjs",
      "scripts/desktop-real-homeserver-qa.mjs"
    ];

    for (const file of activeSourceFiles) {
      const source = readFileSync(new URL(`../../../../${file}`, import.meta.url), "utf8");
      expect(source, file).not.toContain("MATRIX_DESKTOP_");
      expect(source, file).not.toContain("VITE_MATRIX_DESKTOP_");
      expect(source, file).not.toContain("matrix-desktop://");
      expect(source, file).not.toContain("matrix-desktop:");
      expect(source, file).not.toContain("LEGACY_DATA_DIR_NAME");
      expect(source, file).not.toContain("LEGACY_CREDENTIAL_STORE_SERVICE_NAME");
      expect(source, file).not.toContain("migrate_app_data_dir_if_needed");
      expect(source, file).not.toContain("app.kagome");
      expect(source, file).not.toContain("RURI-");
    }
  });

  test("mac GUI smoke send smoke mode passes only a synthetic body through child env", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--send-smoke-message=Koushi synthetic QA send"
    ]);
    const sendLine = output
      .split("\n")
      .find((line) => line.startsWith("VITE_KOUSHI_QA_SEND_SMOKE_MESSAGE="));

    expect(sendLine).toBe(
      "VITE_KOUSHI_QA_SEND_SMOKE_MESSAGE=Koushi synthetic QA send"
    );
    expect(sendLine).not.toContain("password");
  });

  test("mac GUI smoke can target a real DM user for synthetic send smoke", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--send-smoke-message=Koushi synthetic QA send",
      "--send-smoke-user-id=@hiroshi.shinaoka:matrix.org"
    ]);
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(output).toContain(
      "VITE_KOUSHI_QA_SEND_SMOKE_USER_ID=@hiroshi.shinaoka:matrix.org"
    );
    expect(source).toContain("qaSendSmokeTargetUserId");
    expect(source).toContain("api.startDirectMessage(targetUserId)");
    expect(source).toContain("api.selectRoom(targetRoom.room_id)");
  });

  test("mac GUI smoke send smoke uses a bounded send timeout separate from login", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const sendTimeoutMs");
    expect(source).toContain('optionValue("--send-timeout-ms") ?? "30000"');
    expect(source).toContain("waitForQaSend(sendTimeoutMs, diagnostics)");
  });

  test("mac GUI smoke defaults the real-login wait to thirty seconds", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain('optionValue("--timeout-ms") ?? "30000"');
  });

  test("mac GUI smoke fails fast when QA title reports errors during ready wait", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("qaStatusHasBlockingError");
    expect(source).toContain("QA reported an error before ready");
  });

  test("mac GUI smoke verbose mode records private-data-free QA diagnostics", () => {
    const usage = runScript("scripts/desktop-mac-gui-smoke.mjs");
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(usage).toContain("--verbose");
    expect(source).toContain("const verbose = args.has(\"--verbose\")");
    expect(source).toContain("qa-diagnostics.log");
    expect(source).toContain("recordQaPoll");
    expect(source).toContain("diagnostics path:");
  });

  test("mac GUI smoke keeps target DM encryption diagnostics in summaries", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("\"target_dm\"");
    expect(source).toContain("\"target_selected\"");
    expect(source).toContain("\"target_members\"");
  });

  test("mac GUI smoke keeps timeline and crawler counters in diagnostics summaries", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    for (const key of [
      "timeline_visible",
      "timeline_dl",
      "timeline_backfill",
      "crawler_running",
      "crawler_completed",
      "crawler_failed",
      "crawler_processed",
      "crawler_indexed"
    ]) {
      expect(source).toContain(`"${key}"`);
    }
  });

  test("mac GUI smoke keeps rendered DOM counters in diagnostics summaries", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    for (const key of ["dom_screen", "dom_root_children", "dom_text_len"]) {
      expect(source).toContain(`"${key}"`);
    }
  });

  test("Tauri dev uses a refresh-free Vite mode compatible with the desktop CSP", () => {
    const tauriConfig = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/tauri.conf.json", import.meta.url),
        "utf8"
      )
    );
    const packageJson = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/package.json", import.meta.url),
        "utf8"
      )
    );
    const viteConfig = readFileSync(
      new URL("../../../../apps/desktop/vite.config.ts", import.meta.url),
      "utf8"
    );

    expect(tauriConfig.build.beforeDevCommand).toBe("npm run dev:tauri");
    expect(packageJson.scripts["dev:tauri"]).toContain("--mode tauri");
    expect(viteConfig).toContain("mode === \"tauri\"");
    expect(viteConfig).toContain("hmr: false");
    expect(tauriConfig.app.security.devCsp).toContain("http://127.0.0.1:5173");
    expect(tauriConfig.app.security.devCsp).toContain("ws://127.0.0.1:5173");
    for (const csp of [
      tauriConfig.app.security.csp,
      tauriConfig.app.security.devCsp
    ]) {
      expect(csp).toContain("img-src");
      expect(csp).toContain("asset:");
      expect(csp).toContain("http://asset.localhost");
    }
  });

  test("QA file credential store is gated to debug and test builds in core", () => {
    // The credential store moved into koushi-core (StoreActor) when
    // src-tauri became a pure transport adapter; the compile-time gate lives
    // there now.
    const coreStore = readFileSync(
      new URL(
        "../../../../crates/koushi-core/src/store.rs",
        import.meta.url
      ),
      "utf8"
    );

    expect(coreStore).toContain("const ENV_FILE_CREDENTIAL_STORE_DIR");
    expect(coreStore).toMatch(
      /#\[cfg\(any\(debug_assertions, test\)\)\]\nconst ENV_FILE_CREDENTIAL_STORE_DIR/
    );
    expect(coreStore).toMatch(
      /#\[cfg\(any\(debug_assertions, test\)\)\]\npub struct FileCredentialStore/
    );

    // The transport adapter must not read the credential store at all — not
    // even the QA file-dir override env.
    const adapter = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );
    expect(adapter).not.toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR");
    expect(adapter).not.toContain("CredentialStore");
  });

  test("mac GUI smoke rejects unsafe reusable profile names", () => {
    for (const profileName of ["", "../secret"]) {
      const result = spawnSync(
        process.execPath,
        ["scripts/desktop-mac-gui-smoke.mjs", "--child-env", `--qa-profile=${profileName}`],
        {
          cwd: repoRoot,
          encoding: "utf8"
        }
      );

      expect(result.status).not.toBe(0);
      expect(result.stderr).toContain(
        "qa profile must be 1-64 characters of letters, numbers, underscore, or dash"
      );
    }
  });

  test("mac GUI smoke accepts recovery-required sessions after room timeline QA is ready", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=recovery"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("mac GUI smoke can relax timeline item count for sparse QA accounts", () => {
    const strict = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 panel=closed"
    ]);
    const relaxed = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--allow-empty-timeline",
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 panel=closed"
    ]);

    expect(strict.trim()).toBe("not-ready");
    expect(relaxed.trim()).toBe("ready");
  });

  test("mac GUI smoke rejects ready titles with backend errors", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=1 panel=closed"
    ]);

    expect(output.trim()).toBe("not-ready");
  });

  test("mac GUI smoke waits for send smoke success token", () => {
    const pending = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 send=pending panel=closed"
    ]);
    const sent = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=2 errors=0 send=sent panel=closed"
    ]);
    const failed = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=1 send=failed panel=closed"
    ]);

    expect(pending.trim()).toBe("not-ready");
    expect(sent.trim()).toBe("ready");
    expect(failed.trim()).toBe("not-ready");
  });

  test("mac GUI smoke requires ready session when recovery code is supplied", () => {
    const waiting = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=koushi-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=recovery"
    ]);
    const recovered = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=koushi-desktop qa session=ready sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=keyboardSettings"
    ]);

    expect(waiting.trim()).toBe("not-ready");
    expect(recovered.trim()).toBe("ready");
  });

  test("mac GUI smoke uses whose clauses for variable process names", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--print-window-query-script"
    ]);

    expect(output).toContain("first process whose name is candidateName");
    expect(output).not.toContain("exists process candidateName");
    expect(output).not.toContain("tell process candidateName");
  });

  test("mac GUI smoke captures only the app window bounds", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--print-screenshot-args"
    ]);

    expect(output).toContain("-R");
    expect(output).toContain("10,20,300,400");
    expect(output).not.toContain("fullscreen");
  });

  test("mac GUI smoke does not send Cmd+Q while cleaning up", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("terminateProcessGroup");
    expect(source).not.toContain('keystroke "q" using command down');
  });

  test("GUI smoke FIFO writers use node:fs/promises open and never spawn tee", () => {
    const linuxSource = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );
    const macSource = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    for (const source of [linuxSource, macSource]) {
      // The sensitive-payload writer must use a direct node:fs/promises FIFO
      // write so no child process inherits the parent environment.
      expect(source).toContain('import { open } from "node:fs/promises";');
      expect(source).toContain("async function writeSensitivePayloadToPath(path, payload, timeout)");
      expect(source).toContain("await open(path, ");
      // No `tee` helper process anywhere (it would inherit the parent env).
      expect(source).not.toContain('spawn("tee"');
      expect(source).not.toContain('"tee"');
    }
  });
});
