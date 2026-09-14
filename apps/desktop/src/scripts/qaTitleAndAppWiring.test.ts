import { execFileSync } from "node:child_process";
import { existsSync,readFileSync } from "node:fs";
import { describe,expect,test } from "vitest";

import { readLinuxProductionSource,repoRoot,runScript } from "./releaseTestSupport";

describe("desktop release scripts", () => {
  test("mac GUI smoke does not send Cmd+Q while cleaning up", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("terminateProcessGroup");
    expect(source).not.toContain('keystroke "q" using command down');
  });

  test("GUI smoke FIFO writers share the direct node writer and never spawn tee", () => {
    const linuxSource = readLinuxProductionSource();
    const macSource = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );
    const fifoSource = readFileSync(
      new URL("../../../../scripts/lib/sensitive-fifo.mjs", import.meta.url),
      "utf8"
    );

    for (const source of [linuxSource, macSource]) {
      expect(source).toContain("writeSensitivePayloadToPath");
      expect(source).toMatch(/from \"(?:\.\/|\.\.\/)?(?:scripts\/)?lib\/sensitive-fifo\.mjs\"/);
      // No `tee` helper process anywhere (it would inherit the parent env).
      expect(source).not.toContain('spawn("tee"');
      expect(source).not.toContain('"tee"');
    }
    expect(fifoSource).toContain('from "node:fs/promises"');
    expect(fifoSource).toContain("await open(path, ");
    expect(fifoSource).not.toContain('spawn("tee"');
  });

  test("app icon family is consistent and the SVG avoids an unintended white rounded frame", () => {
    const tauriDir = new URL("../../../../apps/desktop/src-tauri/", import.meta.url);
    const conf = JSON.parse(readFileSync(new URL("tauri.conf.json", tauriDir), "utf8")) as {
      bundle: { icon: string[] };
    };

    for (const iconPath of conf.bundle.icon) {
      expect(existsSync(new URL(iconPath, tauriDir))).toBe(true);
    }

    const svgPath = new URL("icons/icon.svg", tauriDir);
    expect(existsSync(svgPath)).toBe(true);
    const svg = readFileSync(svgPath, "utf8");

    // The icon must not use a plain white rounded rectangle as its outer frame.
    const whiteFramePattern = /<rect[^>]*\sfill="(#FFFFFF|white|#fff)"[^>]*\brx="/i;
    expect(whiteFramePattern.test(svg)).toBe(false);

    // The icon set referenced by Tauri must include the source SVG.
    expect(conf.bundle.icon).toContain("icons/icon.svg");
  });

  test("local DMG builds use a git-derived macOS bundle version to invalidate stale icons", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-build-dmg.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain('git", ["rev-list", "--count", "HEAD"]');
    const output = runScript("scripts/desktop-build-dmg.mjs", ["--print-command"]);
    const config = JSON.parse(output.match(/--config (.+)/)?.[1] ?? "null");
    const commitCount = execFileSync("git", ["rev-list", "--count", "HEAD"], {
      cwd: repoRoot, encoding: "utf8"
    }).trim();
    const dirty = execFileSync("git", ["status", "--porcelain", "--untracked-files=no"], {
      cwd: repoRoot, encoding: "utf8"
    }).trim();
    expect(config.bundle.macOS.bundleVersion).toBe(`${commitCount}.${dirty ? "1" : "0"}`);
    expect(source).toContain('"find-identity", "-v", "-p", "codesigning"');
    expect(source).toContain("validIdentities.some");
    expect(source).toContain("APPLE_SIGNING_IDENTITY is not a valid local code-signing identity");
    expect(source).toContain("Developer ID Application:");
    expect(source).toContain("environment.APPLE_SIGNING_IDENTITY = uniqueDeveloperIds[0]");
  });
});
