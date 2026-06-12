import { execFileSync } from "node:child_process";
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
      "open Keyboard settings shortcut",
      "open User settings shortcut",
      "capture private-data-free screenshots",
      "stop app process group"
    ]) {
      expect(output).toContain(check);
    }
  });

  test("release preflight validates mac GUI smoke entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:mac-gui");
  });

  test("release preflight validates real account QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:real-account");
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

  test("mac GUI smoke real login disables keychain persistence for unattended QA", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env-keys",
      "--real-login-from-stdin"
    ]);

    expect(output).toContain("MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE");
  });

  test("mac GUI smoke accepts recovery-required sessions after room timeline QA is ready", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=matrix-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("mac GUI smoke requires ready session when recovery code is supplied", () => {
    const waiting = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=matrix-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0"
    ]);
    const recovered = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=matrix-desktop qa session=ready sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0"
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
