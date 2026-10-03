import { execFileSync,spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { describe,expect,test } from "vitest";

import { repoRoot,runScript } from "./releaseTestSupport";

describe("desktop release scripts", () => {
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
    const tuwunel = runScript("scripts/desktop-headless-local-qa.mjs", ["--print-tuwunel-config"]);

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

  test("mac GUI smoke preserves shared Cargo target dir without exposing secrets", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-mac-gui-smoke.mjs", "--child-env-keys"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          CARGO_TARGET_DIR: "/tmp/koushi-desktop-shared-target",
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("CARGO_TARGET_DIR");
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
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--print-real-login-transport"]);
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

  test("Tauri opener capability grants both URL command and http scopes", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
        "utf8"
      )
    );

    expect(capability.permissions).toContain("opener:allow-open-url");
    expect(capability.permissions).toContain("opener:allow-default-urls");
  });

  test("Tauri window capability grants custom titlebar dragging", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
        "utf8"
      )
    );

    expect(capability.permissions).toContain("core:window:allow-start-dragging");
  });

  test("Tauri launch explicitly makes the main WebView window visible", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );
    const setupSource = source.split(".setup(move |app|").at(1)?.split(".on_window_event").at(0);

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
    const pageLoadSource = source.split(".on_page_load(").at(1)?.split(".on_window_event").at(0);

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
    expect(source).toContain("childEnvironment(dataDir, qaLoginPipePath, qaControlPipePath)");
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
    expect(source).toContain("CoreRuntime::start_with_data_dir_and_native_artifact_port");
    expect(source).toContain(
      "CoreRuntime::start_with_data_dir_and_os_backend_and_native_artifact_port"
    );
  });

  test("Tauri production adapter does not depend on the fixture backend crate", () => {
    const tauriCargo = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/Cargo.toml", import.meta.url),
      "utf8"
    );
    const tauriLib = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );

    expect(tauriCargo).not.toContain("koushi-backend");
    expect(tauriLib).not.toContain("koushi_backend");
    expect(tauriLib).not.toContain("BackendState");
    expect(tauriLib).not.toContain("TimelineTaskHandle");
    expect(tauriLib).not.toContain("TimelinePaginationRequest");
  });

  test("desktop package exposes a local DMG build script", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    const scriptPath = new URL("../../../../scripts/desktop-build-dmg.mjs", import.meta.url);
    const source = readFileSync(scriptPath, "utf8");

    expect(packageJson.scripts["build:dmg"]).toBe("node ../../scripts/desktop-build-dmg.mjs");
    expect(packageJson.scripts["build:dmg:signed"]).toBe(
      "node ../../scripts/desktop-build-dmg.mjs --signed"
    );
    expect(source).toContain("tauri");
    expect(source).toContain("build");
    expect(source).toContain("--bundles");
    expect(source).toContain("dmg");
    expect(source).toContain("--macos-signing");
    expect(source).toContain("Application Support/koushi-desktop");
    expect(source).toContain("koushi-desktop");
    expect(source).toContain('target", "release", "bundle');
    expect(source).toContain('join(bundleDir, "dmg")');
    expect(source).not.toContain('src-tauri", "target", "release", "bundle');
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
      "crates/koushi-qa/src/bin/headless-core-qa.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/registry.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/event_wait.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/participants.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/fixtures.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/cleanup.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/diagnostics.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/orchestrator.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/scenarios/identity.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/scenarios/rooms.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/scenarios/timeline.rs",
      "crates/koushi-qa/src/bin/headless_core_qa/scenarios/search.rs",
      "crates/koushi-qa/src/bin/real-homeserver-qa.rs",
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

    expect(sendLine).toBe("VITE_KOUSHI_QA_SEND_SMOKE_MESSAGE=Koushi synthetic QA send");
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

    expect(output).toContain("VITE_KOUSHI_QA_SEND_SMOKE_USER_ID=@hiroshi.shinaoka:matrix.org");
    expect(source).toContain("qaSendSmokeTargetUserId");
    expect(source).toContain("api.startDirectMessage(targetUserId)");
    expect(source).toContain("void selectRoom(targetRoom.room_id)");
    expect(source).toContain("await drainActiveComposerScopesForNavigation(true, true)");
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
    expect(source).toContain('const verbose = args.has("--verbose")');
    expect(source).toContain("qa-diagnostics.log");
    expect(source).toContain("recordQaPoll");
    expect(source).toContain("diagnostics path:");
  });

  test("mac GUI smoke keeps target DM encryption diagnostics in summaries", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain('"target_dm"');
    expect(source).toContain('"target_selected"');
    expect(source).toContain('"target_members"');
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
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    const viteConfig = readFileSync(
      new URL("../../../../apps/desktop/vite.config.ts", import.meta.url),
      "utf8"
    );

    expect(tauriConfig.build.beforeDevCommand).toBe(
      "npm run guard:sdk && npm run dev:tauri"
    );
    expect(packageJson.scripts["dev:tauri"]).toContain("--mode tauri");
    expect(viteConfig).toContain('mode === "tauri"');
    expect(viteConfig).toContain("hmr: false");
    expect(tauriConfig.app.security.devCsp).toContain("http://127.0.0.1:5173");
    expect(tauriConfig.app.security.devCsp).toContain("ws://127.0.0.1:5173");
    for (const csp of [tauriConfig.app.security.csp, tauriConfig.app.security.devCsp]) {
      expect(csp).toContain("img-src");
      expect(csp).toContain("asset:");
      expect(csp).toContain("http://asset.localhost");
      expect(csp).toContain("koushi-thumbnail:");
      expect(csp).toContain("http://koushi-thumbnail.localhost");
    }
    expect(tauriConfig.app.security.assetProtocol.scope).toEqual([
      "$LOCALDATA/koushi-desktop/media_downloads/**"
    ]);
  });

  test("QA file credential store is gated to debug, test, and test-hooks builds in koushi-store", () => {
    // StoreActor remains Core's policy owner while the credential mechanism
    // and its compile-time gate live in the persistence leaf.
    const credentialStore = readFileSync(
      new URL("../../../../crates/koushi-store/src/credential_backend.rs", import.meta.url),
      "utf8"
    );

    expect(credentialStore).toContain("const ENV_FILE_CREDENTIAL_STORE_DIR");
    expect(credentialStore).toMatch(
      /#\[cfg\(any\(debug_assertions, test, feature = "test-hooks"\)\)\]\nconst ENV_FILE_CREDENTIAL_STORE_DIR/
    );
    expect(credentialStore).toMatch(
      /#\[cfg\(any\(debug_assertions, test, feature = "test-hooks"\)\)\]\n(?:#\[derive\([^\n]+\)\]\n)?pub struct FileCredentialStore/
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

  test("mac GUI smoke rejects active/timeline room mismatches", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_room=true timeline_matches_active=false timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);

    expect(output.trim()).toBe("not-ready");
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
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--print-window-query-script"]);

    expect(output).toContain("first process whose name is candidateName");
    expect(output).not.toContain("exists process candidateName");
    expect(output).not.toContain("tell process candidateName");
  });

  test("mac GUI smoke captures only the app window bounds", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--print-screenshot-args"]);

    expect(output).toContain("-R");
    expect(output).toContain("10,20,300,400");
    expect(output).not.toContain("fullscreen");
  });

  test("mac GUI smoke drives density and native resize through private viewport evidence", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("waitForViewportRootLayout");
    expect(source).toContain("set size of window 1");
    expect(source).toContain("entire contents of window 1");
    expect(source).toContain('role of candidate is "AXButton"');
    expect(source).toContain('perform action "AXPress" of candidate');
    for (const density of ["Compact", "Default", "Comfortable"]) {
      expect(source).toContain(`\"${density}\"`);
    }
    expect(source).toContain("viewport=aligned");
    expect(source).toContain("viewport_parent=true");
    expect(source).toContain("viewport_webview=true");
    expect(source).toContain("viewport_js=true");
    expect(source).toContain("viewport_root=true");
    expect(source).not.toContain("click at");
  });

  test("mac GUI smoke parser accepts only a complete newer Rust viewport receipt", () => {
    const aligned = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-viewport=koushi-desktop qa viewport=aligned viewport_generation=12 viewport_parent=true viewport_webview=true viewport_js=true viewport_root=true viewport_decision=repair_to_parent_bounds",
      "--minimum-viewport-generation=11"
    ]);
    const stale = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-viewport=koushi-desktop qa viewport=aligned viewport_generation=12 viewport_parent=true viewport_webview=true viewport_js=true viewport_root=true viewport_decision=in_sync",
      "--minimum-viewport-generation=12"
    ]);
    const incomplete = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-viewport=koushi-desktop qa viewport=aligned viewport_generation=13 viewport_parent=true viewport_webview=true viewport_js=false viewport_root=true viewport_decision=in_sync"
    ]);

    expect(aligned.trim()).toBe("ready");
    expect(stale.trim()).toBe("not-ready");
    expect(incomplete.trim()).toBe("not-ready");
  });
});
