import { execFileSync,spawnSync } from "node:child_process";
import { mkdirSync,mkdtempSync,readFileSync,rmSync,writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe,expect,test } from "vitest";

import { gitTrackedFiles,repoRoot,runScript } from "./releaseTestSupport";

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
    expect(output).toContain("security.assetProtocol.enable");
    expect(output).toContain("security.assetProtocol.scope.noBroadAppdata");
    expect(output).toContain("security.assetProtocol.scope.mediaDownloads");
    expect(output).toContain("security.csp.img-src.koushiThumbnail");
  });

  test("macOS signing preflight requires Apple credentials without Windows credentials", () => {
    const script = "scripts/desktop-release-preflight.mjs";
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    expect(packageJson.scripts["release:preflight:macos-signing"]).toBe(
      "node ../../scripts/desktop-release-preflight.mjs --macos-signing"
    );
    const missingEnvironment = { ...process.env };
    delete missingEnvironment.APPLE_SIGNING_IDENTITY;
    delete missingEnvironment.APPLE_ID;
    delete missingEnvironment.APPLE_PASSWORD;
    delete missingEnvironment.APPLE_TEAM_ID;
    delete missingEnvironment.WINDOWS_CERTIFICATE_THUMBPRINT;
    delete missingEnvironment.WINDOWS_SIGN_COMMAND;

    const missing = spawnSync(process.execPath, [script, "--macos-signing"], {
      cwd: repoRoot,
      encoding: "utf8",
      env: missingEnvironment,
    });
    expect(missing.status).toBe(1);
    expect(missing.stderr).toContain("env.APPLE_SIGNING_IDENTITY");
    expect(missing.stderr).toContain("env.appleNotarization");

    const configured = spawnSync(process.execPath, [script, "--macos-signing"], {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...missingEnvironment,
        APPLE_SIGNING_IDENTITY: "Developer ID Application: Synthetic",
        APPLE_ID: "synthetic@example.invalid",
        APPLE_PASSWORD: "synthetic-app-password",
        APPLE_TEAM_ID: "SYNTHETIC",
      },
    });
    expect(configured.status).toBe(0);
    expect(configured.stdout).toContain("ok env.APPLE_SIGNING_IDENTITY");
    expect(configured.stdout).toContain("ok env.appleNotarization");
    expect(configured.stderr).not.toContain("env.windowsSigning");
  });

  test("macOS signing preflight accepts an App Store Connect API key file", () => {
    const temporaryDirectory = mkdtempSync(join(tmpdir(), "koushi-notary-preflight-"));
    const apiKeyPath = join(temporaryDirectory, "AuthKey_SYNTHETIC.p8");
    writeFileSync(apiKeyPath, "synthetic-not-a-real-private-key", { mode: 0o600 });
    try {
      const result = spawnSync(
        process.execPath,
        ["scripts/desktop-release-preflight.mjs", "--macos-signing"],
        {
          cwd: repoRoot,
          encoding: "utf8",
          env: {
            ...process.env,
            APPLE_SIGNING_IDENTITY: "Developer ID Application: Synthetic",
            APPLE_API_ISSUER: "synthetic-issuer",
            APPLE_API_KEY: "SYNTHETIC",
            APPLE_API_KEY_PATH: apiKeyPath,
          },
        }
      );
      expect(result.status).toBe(0);
      expect(result.stdout).toContain("ok env.appleNotarization");
    } finally {
      rmSync(temporaryDirectory, { recursive: true, force: true });
    }
  });

  test("release version validator requires one changed SemVer across every manifest", () => {
    const temporaryDirectory = mkdtempSync(join(tmpdir(), "koushi-release-version-"));
    const desktopDirectory = join(temporaryDirectory, "apps", "desktop");
    const tauriDirectory = join(desktopDirectory, "src-tauri");
    mkdirSync(tauriDirectory, { recursive: true });
    const writeVersions = (packageVersion: string, tauriVersion: string, cargoVersion: string) => {
      writeFileSync(
        join(desktopDirectory, "package.json"),
        JSON.stringify({ version: packageVersion })
      );
      writeFileSync(
        join(tauriDirectory, "tauri.conf.json"),
        JSON.stringify({ version: tauriVersion })
      );
      writeFileSync(
        join(tauriDirectory, "Cargo.toml"),
        `[package]\nname = "fixture"\nversion = "${cargoVersion}"\n`
      );
    };
    writeVersions("1.2.2", "1.2.2", "1.2.2");
    try {
      execFileSync("git", ["init", "-q"], { cwd: temporaryDirectory });
      execFileSync("git", ["config", "user.email", "release-test@example.invalid"], {
        cwd: temporaryDirectory
      });
      execFileSync("git", ["config", "user.name", "Koushi release test"], {
        cwd: temporaryDirectory
      });
      execFileSync("git", ["add", "."], { cwd: temporaryDirectory });
      execFileSync("git", ["commit", "-qm", "fixture release"], { cwd: temporaryDirectory });
      const previousSha = execFileSync("git", ["rev-parse", "HEAD"], {
        cwd: temporaryDirectory,
        encoding: "utf8"
      }).trim();

      const unchanged = spawnSync(
        process.execPath,
        [
          "scripts/desktop-release-version.mjs",
          "--root",
          temporaryDirectory,
          "--before",
          previousSha
        ],
        { cwd: repoRoot, encoding: "utf8" }
      );
      expect(unchanged.status).toBe(0);
      expect(unchanged.stdout).toContain("proceed=false");
      expect(unchanged.stderr).toContain("release version unchanged: 1.2.2");

      writeVersions("1.2.1", "1.2.1", "1.2.1");
      const decreased = spawnSync(
        process.execPath,
        [
          "scripts/desktop-release-version.mjs",
          "--root",
          temporaryDirectory,
          "--before",
          previousSha
        ],
        { cwd: repoRoot, encoding: "utf8" }
      );
      expect(decreased.status).toBe(1);
      expect(decreased.stderr).toContain("release version must increase: 1.2.2 -> 1.2.1");

      writeVersions("1.2.3", "1.2.3", "1.2.4");
      const mismatch = spawnSync(
        process.execPath,
        ["scripts/desktop-release-version.mjs", "--root", temporaryDirectory],
        { cwd: repoRoot, encoding: "utf8" }
      );
      expect(mismatch.status).toBe(1);
      expect(mismatch.stderr).toContain("release versions do not match");

      writeVersions("1.2.3", "1.2.3", "1.2.3");
      const consistent = spawnSync(
        process.execPath,
        [
          "scripts/desktop-release-version.mjs",
          "--root",
          temporaryDirectory,
          "--before",
          previousSha
        ],
        { cwd: repoRoot, encoding: "utf8" }
      );
      expect(consistent.status).toBe(0);
      expect(consistent.stdout).toContain("version=1.2.3");
      expect(consistent.stdout).toContain("tag=v1.2.3");
      expect(consistent.stdout).toContain("proceed=true");
    } finally {
      rmSync(temporaryDirectory, { recursive: true, force: true });
    }
  });

  test("desktop release workflow publishes fixed-name assets only after supported platforms pass", () => {
    const workflow = readFileSync(
      new URL("../../../../.github/workflows/release-desktop.yml", import.meta.url),
      "utf8"
    );

    for (const token of [
      "paths:",
      "scripts/desktop-release-version.mjs",
      "environment: release-macos",
      "APPLE_API_PRIVATE_KEY",
      "npm --prefix apps/desktop audit --package-lock-only --audit-level=high",
      "codesign --verify --deep --strict",
      "xcrun notarytool submit",
      "xcrun stapler staple",
      "xcrun stapler validate",
      "hdiutil attach",
      "spctl --assess",
      "Koushi-macos-arm64.dmg",
      "Koushi-windows-x64-unsigned.exe",
      "build:linux",
      "Koushi-linux-x64.AppImage",
      "Koushi-linux-x64.deb",
      "Koushi-linux-x64.rpm",
      "[System.IO.File]::WriteAllText",
      "gh release create",
      "--draft",
      "gh release edit",
      "--draft=false",
    ]) {
      expect(workflow).toContain(token);
    }
    for (const retiredIntelToken of [
      "macos-15-intel",
      "x86_64-apple-darwin",
      "Koushi-macos-x64.dmg",
    ]) {
      expect(workflow).not.toContain(retiredIntelToken);
    }
    expect(workflow).not.toContain('Set-Content -Path "release-assets/$asset.sha256"');
    expect(workflow).toMatch(
      /publish-release:[\s\S]*needs:\s*\[prepare, build-macos, build-windows, build-linux\]/
    );
  });

  test("desktop release workflow preserves trusted Rust build outputs across retries", () => {
    const workflow = readFileSync(
      new URL("../../../../.github/workflows/release-desktop.yml", import.meta.url),
      "utf8"
    );

    for (const token of [
      "Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32 # v2.9.1",
      "workspaces: . -> target",
      "cache-on-failure: true",
      "cache-all-crates: true",
      "cache-workspace-crates: true",
    ]) {
      expect(workflow.match(new RegExp(token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "g")))
        .toHaveLength(3);
    }
  });

  test("desktop release skill is shared by Codex, Claude Code, OpenCode, and Pi", () => {
    const sharedSkill = readFileSync(
      new URL("../../../../.agents/skills/koushi-release/SKILL.md", import.meta.url),
      "utf8"
    );
    const claudeSkill = readFileSync(
      new URL("../../../../.claude/skills/koushi-release/SKILL.md", import.meta.url),
      "utf8"
    );
    const openCodeSkill = readFileSync(
      new URL("../../../../.opencode/skills/koushi-release/SKILL.md", import.meta.url),
      "utf8"
    );
    const runbook = readFileSync(
      new URL("../../../../docs/releases/desktop-release.md", import.meta.url),
      "utf8"
    );

    expect(claudeSkill).toBe(sharedSkill);
    expect(openCodeSkill).toBe(sharedSkill);
    expect(sharedSkill).toContain("../../../docs/releases/desktop-release.md");
    for (const invocation of [
      "$koushi-release",
      "/koushi-release",
      "/skill:koushi-release"
    ]) {
      expect(runbook).toContain(invocation);
    }
    for (const manifest of [
      "apps/desktop/package.json",
      "apps/desktop/src-tauri/tauri.conf.json",
      "apps/desktop/src-tauri/Cargo.toml"
    ]) {
      expect(runbook).toContain(manifest);
    }
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
});
