import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkTauriVersions, minorLine, tauriCrateVersion } from "./check-tauri-versions.mjs";

const script = fileURLToPath(new URL("./check-tauri-versions.mjs", import.meta.url));

const lockfile = (...blocks) => blocks.map((block) => `[[package]]\n${block}`).join("\n");

function fixture({ crate, api, cli }) {
  const root = mkdtempSync(join(tmpdir(), "koushi-tauri-versions-"));
  writeFileSync(join(root, "Cargo.lock"), lockfile(`name = "tauri"\nversion = "${crate}"\nsource = "registry"`));
  for (const [path, version] of [
    ["apps/desktop/node_modules/@tauri-apps/api/package.json", api],
    ["apps/desktop/node_modules/@tauri-apps/cli/package.json", cli],
  ]) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), JSON.stringify({ name: path, version }));
  }
  return root;
}

test("reads the tauri crate version, not a sibling tauri-* crate", () => {
  const source = lockfile(
    `name = "tauri-build"\nversion = "2.10.0"`,
    `name = "tauri-utils"\nversion = "2.11.0"`,
    `name = "tauri-plugin-dialog"\nversion = "2.9.0"`,
    `name = "tauri"\nversion = "2.12.5"`,
  );
  assert.equal(tauriCrateVersion(source), "2.12.5");
});

test("compares the major.minor line, not the patch", () => {
  assert.equal(minorLine("2.12.5"), "2.12");
  assert.deepEqual(checkTauriVersions({ crate: "2.12.5", api: "2.12.4", cli: "2.12.1" }), []);
});

test("reports each package that left the crate's minor line", () => {
  assert.deepEqual(checkTauriVersions({ crate: "2.12.5", api: "2.11.0", cli: "2.12.1" }), [
    "@tauri-apps/api 2.11.0 is not on the tauri 2.12.5 minor line (2.12)",
  ]);
  assert.equal(checkTauriVersions({ crate: "2.12.5", api: "2.11.0", cli: "2.13.0" }).length, 2);
});

test("exits 0 when the installed packages match the lockfile", () => {
  const result = spawnSync(process.execPath, [script, "--root", fixture({ crate: "2.12.5", api: "2.12.4", cli: "2.12.1" })], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /agree on minor 2\.12/);
});

test("exits 1 on a stale @tauri-apps/api minor (#1749)", () => {
  const result = spawnSync(process.execPath, [script, "--root", fixture({ crate: "2.12.5", api: "2.11.0", cli: "2.12.1" })], { encoding: "utf8" });
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, /@tauri-apps\/api 2\.11\.0 is not on the tauri 2\.12\.5 minor line \(2\.12\)/);
});
