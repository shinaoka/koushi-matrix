import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";

const repoRoot = new URL("../../../../", import.meta.url).pathname;

function runScript(script: string, args: string[] = []): string {
  return execFileSync(process.execPath, [script, ...args], {
    cwd: repoRoot,
    encoding: "utf8"
  });
}

describe("desktop release scripts", () => {
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
      "--qa-title-panel=matrix-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings"
    ]);

    expect(output.trim()).toBe("keyboardSettings");
  });

  test("mac GUI smoke only skips panel checks while recovery owns the panel", () => {
    const readyRecoveryPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=matrix-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=recovery",
      "--required-panel=keyboardSettings"
    ]);
    const recoveryPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=matrix-desktop qa session=needsRecovery sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=recovery",
      "--required-panel=keyboardSettings"
    ]);
    const keyboardPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=matrix-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);
    const erroredPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=matrix-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=1 panel=keyboardSettings",
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

  test("release preflight validates linux GUI smoke entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:linux-gui");
  });

  test("release preflight validates real account QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:real-account");
  });

  test("release preflight validates headless local QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:headless-local");
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

  test("linux GUI smoke parses the attention baseline title token", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-attention-ready=matrix-desktop qa session=signedOut sync=stopped rooms=0 spaces=0 active_room=false timeline_subscribed=false timeline_items=0 errors=0 unread=0 badge=0 notify=none"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("linux GUI smoke validates the persisted window-state path contract", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-window-state-ready=/tmp/matrix-desktop/app-shell/window-state.json"
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
          MATRIX_DESKTOP_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("MATRIX_DESKTOP_DATA_DIR=");
    expect(output).toContain("MATRIX_DESKTOP_QA_TITLE=1");
    expect(output).toContain("VITE_MATRIX_DESKTOP_QA_TITLE=1");
    expect(output).toContain("MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=1");
    expect(output).toContain("MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1");
    expect(output).toContain("MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain("/qa-credential-store");
    expect(output).toContain("NO_COLOR=1");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("MATRIX_DESKTOP_TEST_SECRET");
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
    expect(source).toContain("MATRIX_DESKTOP_QA_LOGIN_PIPE");
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
      "RUST_TOOLCHAIN=${RUST_TOOLCHAIN}",
      '--default-toolchain "${RUST_TOOLCHAIN}"',
      'rustup default "${RUST_TOOLCHAIN}"',
      'RUSTUP_TOOLCHAIN="${RUST_TOOLCHAIN}"',
      "libwebkit2gtk-4.1-dev",
      "libayatana-appindicator3-dev",
      "webkit2gtk-driver",
      "xvfb",
      "fonts-noto-color-emoji",
      "cargo install tauri-driver --locked",
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
    expect(agents).toContain("-v /tmp/matrix-desktop-cargo-home:/tmp/cargo-home");
    expect(agents).toContain("-v /tmp/matrix-desktop-gui-target:/tmp/matrix-desktop-gui-target");
    expect(agents).toContain("-v /tmp/matrix-desktop-npm-cache:/tmp/npm-cache");
    expect(agents).toContain("CARGO_HOME=/tmp/cargo-home");
    expect(agents).toContain("CARGO_TARGET_DIR=/tmp/matrix-desktop-gui-target");
    expect(agents).toContain("NPM_CONFIG_CACHE=/tmp/npm-cache");
    expect(agents).toContain("--artifact-dir=/work/artifacts/linux-gui-qa");
    expect(agents).toContain("--timeout-ms=180000");
    expect(agents).toContain("PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
    expect(agents).toContain("RUSTC=\"$(rustup which rustc)\"");
    expect(agents).toContain("RUSTDOC=\"$(rustup which rustdoc)\"");
  });

  test("linux GUI smoke QA title helpers match the mac runner contract", () => {
    const ready = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);
    const readyRecovered = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready-require-recovered=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);
    const panel = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-panel=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings"
    ]);
    const panelReady = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-panel-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);
    const sendReady = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-send-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 send=sent panel=closed"
    ]);

    expect(ready.trim()).toBe("ready");
    expect(readyRecovered.trim()).toBe("ready");
    expect(panel.trim()).toBe("keyboardSettings");
    expect(panelReady.trim()).toBe("ready");
    expect(sendReady.trim()).toBe("ready");
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
          MATRIX_DESKTOP_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("PATH");
    expect(output).toContain("MATRIX_DESKTOP_RESTORE_SESSION");
    expect(output).toContain("MATRIX_DESKTOP_SKIP_SAVED_SESSIONS");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("MATRIX_DESKTOP_TEST_SECRET");
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
          MATRIX_DESKTOP_DEBUG_SDK_ERROR: "synthetic-secret-value"
        }
      }
    );

    expect(output).toContain("MATRIX_DESKTOP_DEBUG_SDK_ERROR=1");
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

    expect(output).toContain("VITE_MATRIX_DESKTOP_QA_TITLE");
    expect(output).toContain("MATRIX_DESKTOP_QA_TITLE");
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
    expect(source).toContain("MATRIX_DESKTOP_QA_LOGIN_PIPE");
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

  test("mac GUI smoke real login disables keychain persistence for unattended QA", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env-keys",
      "--real-login-from-stdin"
    ]);

    expect(output).toContain("MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE");
  });

  test("mac GUI smoke reusable profile keeps restore and saved sessions enabled", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--qa-profile=agent-sync"
    ]);

    expect(output).toContain("MATRIX_DESKTOP_RESTORE_SESSION=1");
    expect(output).toContain("MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=0");
    expect(output).toContain(".local-secrets/qa-profiles/agent-sync/data");
    expect(output).toContain("MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain(".local-secrets/qa-profiles/agent-sync/data/qa-credential-store");
    expect(output).not.toContain("MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE");
  });

  test("mac GUI smoke send smoke mode passes only a synthetic body through child env", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--send-smoke-message=Matrix Desktop synthetic QA send"
    ]);
    const sendLine = output
      .split("\n")
      .find((line) => line.startsWith("VITE_MATRIX_DESKTOP_QA_SEND_SMOKE_MESSAGE="));

    expect(sendLine).toBe(
      "VITE_MATRIX_DESKTOP_QA_SEND_SMOKE_MESSAGE=Matrix Desktop synthetic QA send"
    );
    expect(sendLine).not.toContain("password");
  });

  test("QA file credential store is gated to debug and test builds in core", () => {
    // The credential store moved into matrix-desktop-core (StoreActor) when
    // src-tauri became a pure transport adapter; the compile-time gate lives
    // there now.
    const coreStore = readFileSync(
      new URL(
        "../../../../crates/matrix-desktop-core/src/store.rs",
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
    expect(adapter).not.toContain("MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR");
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
      "--qa-title-ready=matrix-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=recovery"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("mac GUI smoke can relax timeline item count for sparse QA accounts", () => {
    const strict = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 panel=closed"
    ]);
    const relaxed = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--allow-empty-timeline",
      "--qa-title-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 panel=closed"
    ]);

    expect(strict.trim()).toBe("not-ready");
    expect(relaxed.trim()).toBe("ready");
  });

  test("mac GUI smoke rejects ready titles with backend errors", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=1 panel=closed"
    ]);

    expect(output.trim()).toBe("not-ready");
  });

  test("mac GUI smoke waits for send smoke success token", () => {
    const pending = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 send=sending panel=closed"
    ]);
    const sent = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=2 errors=0 send=sent panel=closed"
    ]);
    const failed = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=matrix-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=1 send=failed panel=closed"
    ]);

    expect(pending.trim()).toBe("not-ready");
    expect(sent.trim()).toBe("ready");
    expect(failed.trim()).toBe("not-ready");
  });

  test("mac GUI smoke requires ready session when recovery code is supplied", () => {
    const waiting = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=matrix-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=recovery"
    ]);
    const recovered = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=matrix-desktop qa session=ready sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=keyboardSettings"
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
});
