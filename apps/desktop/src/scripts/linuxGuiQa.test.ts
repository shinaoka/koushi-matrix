import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { describe,expect,test } from "vitest";

import { readLinuxProductionSource,repoRoot,runScript } from "./releaseTestSupport";

describe("desktop release scripts", () => {
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
    const source = readLinuxProductionSource();

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
    const source = readLinuxProductionSource();

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
    const source = readLinuxProductionSource();

    expect(source).toContain("local-spaces-nav");
    expect(source).toContain("waitForWorkspaceActive");
    expect(source).toContain("clickWorkspaceButton");
    expect(source).toContain("gui_local_spaces_home=ok");
    expect(source).toContain("gui_local_spaces_nav=ok");
    expect(source).toContain("gui_local_spaces_info=ok");
  });

  test("linux GUI local scenarios also emit DBus and window-state evidence", () => {
    const source = readLinuxProductionSource();

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

  test("linux GUI local login selects the first room when the room pane is not ready", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("shouldSelectFirstRoom(status, selectedRoom, composerVisible)");
    expect(source).toMatch(
      /function shouldSelectFirstRoom\(status, selectedRoom, composerVisible\)[\s\S]*status\.session !== "ready" \|\| status\.rooms <= 0[\s\S]*!composerVisible \|\| status\.active_room === false \|\| status\.timeline_subscribed === false/
    );
    expect(source).toMatch(
      /if \(shouldSelectFirstRoom\(status, selectedRoom, composerVisible\)\) \{[\s\S]*selectedRoom = await selectFirstRoom\(browser\);/
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
    const source = readLinuxProductionSource();

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
          KOUSHI_CORE_ACTOR_TRACE: "1",
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
    expect(output).toContain("KOUSHI_CORE_ACTOR_TRACE=1");
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
    expect(output).toContain("KOUSHI_QA_CONTROL_PIPE");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("Tauri crate is owned by the root Cargo workspace", () => {
    const rootCargo = readFileSync(new URL("../../../../Cargo.toml", import.meta.url), "utf8");
    const tauriCargo = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/Cargo.toml", import.meta.url),
      "utf8"
    );
    const releaseGate = readFileSync(
      new URL("../../../../scripts/desktop-release-gate-check.mjs", import.meta.url),
      "utf8"
    );

    expect(rootCargo).toContain('"apps/desktop/src-tauri"');
    expect(tauriCargo).not.toMatch(/^\[workspace\]$/m);
    expect(releaseGate).toContain('"koushi-desktop"');
    expect(releaseGate).not.toContain('"apps", "desktop", "src-tauri"');
  });

  test("local and real homeserver QA preserve shared Cargo target dir", () => {
    const localQaSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );
    const realQaSource = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(localQaSource).toMatch(/"CARGO_TARGET_DIR"/);
    expect(realQaSource).toMatch(/"CARGO_TARGET_DIR"/);
  });

  test("linux GUI smoke source wires the shared local homeserver helper module", () => {
    const guiSource = readLinuxProductionSource();
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
    const source = readLinuxProductionSource();

    expect(source).toContain("serverDataDir");
    expect(source).toContain("homeserver-data");
    expect(source).toContain("const session = {");
    expect(source).toContain("await cleanupLocalGuiScenario(session)");
  });

  test("linux GUI local setup defines the safe timestamp helper it uses for synthetic users", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("const userSuffix = safeTimestamp();");
    expect(source).toContain("function safeTimestamp()");
    expect(source).toContain('replaceAll("-", "_")');
  });

  test("linux GUI smoke real login transport is FIFO and the script avoids password args", () => {
    const transport = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--print-real-login-transport"
    ]);
    const source = readLinuxProductionSource();

    expect(transport.trim()).toBe("fifo");
    expect(source).toContain("readRealLoginCredentials");
    expect(source).toContain("writeRealLoginPipe");
    expect(source).toContain("requestQaLogout(qaControlPipePath)");
    expect(source).toContain("KOUSHI_QA_LOGIN_PIPE");
    expect(source).toContain("KOUSHI_QA_CONTROL_PIPE");
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
    const source = readLinuxProductionSource();

    expect(source).toContain("webdriverio");
    expect(source).toContain('createRequire(new URL("../../apps/desktop/package.json"');
    expect(source).toContain("importDesktopWebdriverio");
    expect(source).toContain("remote({");
    expect(source).toContain("screenshots/01-signed-out.png");
    expect(source).not.toContain("foundation is wired, but the WebDriver session");
  });

  test("linux GUI smoke launches Xvfb with the sanitized child environment", () => {
    const source = readLinuxProductionSource();

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
      "curl --proto '=https' --tlsv1.2 -fsSLo /tmp/tuwunel.zst",
      "unzstd -f -o /usr/local/bin/tuwunel /tmp/tuwunel.zst",
      "tuwunel --version",
      'RUSTC="$(rustup which rustc)"',
      'RUSTDOC="$(rustup which rustdoc)"'
    ]) {
      expect(dockerfile).toContain(token);
    }
  });

  test("linux GUI container docs use bash -c and the audited artifact lane", () => {
    const agents = readFileSync(
      new URL("../../../../docs/agents/environment.md", import.meta.url),
      "utf8"
    );

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
    expect(agents).toContain("--server=tuwunel");
    expect(agents).toContain("--artifact-dir=/work/artifacts/linux-gui-local-send-docker");
    expect(agents).toContain("--timeout-ms=180000");
    expect(agents).toContain("tuwunel");
    expect(agents).toContain("zstd");
    expect(agents).toContain(
      "PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    );
    expect(agents).toContain('RUSTC="$(rustup which rustc)"');
    expect(agents).toContain('RUSTDOC="$(rustup which rustdoc)"');
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
    const mismatchedTimeline = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_room=true timeline_matches_active=false timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);

    expect(ready.trim()).toBe("ready");
    expect(readyRecovered.trim()).toBe("ready");
    expect(panel.trim()).toBe("keyboardSettings");
    expect(panelReady.trim()).toBe("ready");
    expect(sendReady.trim()).toBe("ready");
    expect(mismatchedTimeline.trim()).toBe("not-ready");
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
    expect(source).toMatch(/isTauriRuntime\(\) &&\s*completionStatus !== "failed"[\s\S]*return;/);
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
      /async function sendText\([^)]*\)[\s\S]*qaSendPending\.current = true;[\s\S]*setQaSendStatus\("pending"\);/
    );
  });

  test("linux GUI local login retries room selection until a displayed row is clicked", () => {
    const source = readLinuxProductionSource();

    expect(source).toContain("selectedRoom = await selectFirstRoom(browser);");
    expect(source).toMatch(
      /async function selectFirstRoom\(browser\)[\s\S]*return false;[\s\S]*await roomItems\[0\]\.click\(\);[\s\S]*return true;/
    );
  });

  test("headless local QA script lists homeserver and SDK checks", () => {
    const output = runScript("scripts/desktop-headless-local-qa.mjs", ["--list"]);

    for (const check of [
      "verify installed Tuwunel binary",
      "verify local Synapse Docker runtime when --server=synapse",
      "start disposable local homeserver",
      "register synthetic local users",
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

  test("headless local QA forwards explicit Rust diagnostics env to core QA", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("KOUSHI_QA_RUST_LOG");
    expect(source).toContain("KOUSHI_QA_RUST_BACKTRACE");
    expect(source).toContain("env.RUST_LOG = process.env.KOUSHI_QA_RUST_LOG");
    expect(source).not.toContain('"RUST_LOG",');
    expect(source).not.toContain('"RUST_BACKTRACE",');
    expect(source).toContain("KOUSHI_QA_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND");
  });

  test("headless local QA exposes strict E2EE multi-device options", () => {
    const usage = runScript("scripts/desktop-headless-local-qa.mjs");
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(usage).toContain("--e2ee-recipient-second-device");
    expect(usage).toContain("--e2ee-pause-sync-before-multi-device-send");
    expect(source).toContain("e2eeRecipientSecondDeviceOption");
    expect(source).toContain('env.KOUSHI_QA_E2EE_RECIPIENT_SECOND_DEVICE = "true"');
    expect(source).toContain('env.KOUSHI_QA_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND = "true"');
    expect(source.indexOf("if (e2eeRecipientSecondDeviceOption)")).toBeGreaterThan(
      source.indexOf("for (const name of [")
    );
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

  test("linux GUI local login completes only the new-identity bootstrap form without retaining secrets", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa/local-session.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("completeNewIdentityBootstrapIfOffered");
    expect(source).toContain('status.session === "awaitingVerification"');
    expect(source).toContain('"Recovery key destination"');
    expect(source).toContain('"Backup passphrase"');
    expect(source).toContain('"Create secure backup"');
    expect(source).toContain('"I saved the recovery key"');
    expect(source).toContain("MESSAGE_COMPOSER_SELECTOR");
    expect(source).toContain("mkdtempSync(join(tmpdir()");
    expect(source).toContain("randomBytes(32).toString(\"base64url\")");
    expect(source).toContain("bootstrapAttempt.attempted = true");
    expect(source).toContain("let bootstrapDir");
    expect(source).toContain("bootstrapTempDirs.add");
    expect(source).toContain("if (bootstrapDir)");
    expect(source).toContain("bootstrapTempDirs.delete");
    expect(source).toContain("rmSync(bootstrapDir, { recursive: true, force: true })");
    expect(source).not.toContain("console.log(bootstrapPassphrase");
    expect(source).not.toContain("process.env.KOUSHI_QA_BOOTSTRAP");
    expect(source).not.toContain("invoke(\"start_session_bootstrap\"");

    const runnerSource = readLinuxProductionSource();
    expect(runnerSource).toMatch(
      /runLocalLogoutReloginScenario\(\)[\s\S]*session\.allowNewIdentityBootstrap = false;[\s\S]*submitLoginForm/
    );
  });

  test("headless local QA routes SDK and Core output through the validated artifact boundary", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain('from "./lib/qa-output-artifacts.mjs"');
    expect(source.match(/writeValidatedQaOutputFiles\(\{/g)).toHaveLength(2);
    expect(source).toContain('label: "sdk"');
    expect(source).toContain("label: `core-${qaLabel}`");
    expect(source.match(/validate: \(output\) =>/g)).toHaveLength(2);
    expect(source.match(/assertQaOutputIsPrivate\(/g)?.length ?? 0).toBeGreaterThanOrEqual(2);
    expect(source).toContain("requiredTokensForHeadlessScenario(scenario)");
    expect(source).toContain("assertRequiredTokens(result.stdout");
  });
});
