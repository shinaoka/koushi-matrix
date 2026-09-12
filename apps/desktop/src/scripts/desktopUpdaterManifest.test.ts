import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, test } from "vitest";

import { repoRoot } from "./releaseTestSupport";

describe("desktop updater manifest", () => {
  test("writes a Tauri static manifest for the macOS archive", () => {
    const directory = mkdtempSync(join(tmpdir(), "koushi-updater-manifest-"));
    const signaturePath = join(directory, "update.sig");
    const outputPath = join(directory, "latest.json");
    writeFileSync(signaturePath, "signed-update\n", "utf8");
    try {
      const result = spawnSync(
        process.execPath,
        [
          "scripts/desktop-updater-manifest.mjs",
          "--version",
          "1.2.3",
          "--target",
          "darwin-aarch64",
          "--url",
          "https://example.invalid/Koushi.app.tar.gz",
          "--signature-file",
          signaturePath,
          "--output",
          outputPath
        ],
        { cwd: repoRoot, encoding: "utf8" }
      );

      expect(result.status).toBe(0);
      const manifest = JSON.parse(readFileSync(outputPath, "utf8"));
      expect(manifest.version).toBe("1.2.3");
      expect(manifest.platforms["darwin-aarch64"]).toEqual({
        signature: "signed-update",
        url: "https://example.invalid/Koushi.app.tar.gz"
      });
      expect(Number.isNaN(Date.parse(manifest.pub_date))).toBe(false);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
