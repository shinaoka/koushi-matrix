#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  assertSdkSubmoduleSynced,
  assertSdkWorkspaceUsesSubmodulePaths,
  parseSubmoduleStatus,
} from "./lib/sdk-submodule-status.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const VALID_SDK_DEPENDENCIES = `
[workspace.dependencies]
matrix-sdk = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk" }
matrix-sdk-base = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-base" }
matrix-sdk-search = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-search" }
matrix-sdk-test = { path = "vendor/matrix-rust-sdk/testing/matrix-sdk-test" }
matrix-sdk-ui = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-ui" }
`;

function writeManifest(source) {
  const fixtureDir = mkdtempSync(join(tmpdir(), "koushi-sdk-manifest-"));
  const fixturePath = join(fixtureDir, "Cargo.toml");
  writeFileSync(fixturePath, source);
  return fixturePath;
}

test("parseSubmoduleStatus accepts only initialized in-sync SDK status", () => {
  assert.deepEqual(
    parseSubmoduleStatus(" 18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n"),
    {
      ok: true,
      state: "synced",
      path: "vendor/matrix-rust-sdk",
      revision: "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
    },
  );

  assert.deepEqual(
    parseSubmoduleStatus("-18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n"),
    {
      ok: false,
      state: "uninitialized",
      path: "vendor/matrix-rust-sdk",
      revision: "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
    },
  );

  assert.deepEqual(
    parseSubmoduleStatus("+18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n"),
    {
      ok: false,
      state: "stale",
      path: "vendor/matrix-rust-sdk",
      revision: "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
    },
  );
});

test("workspace Matrix SDK dependencies resolve only through submodule paths", () => {
  assert.doesNotThrow(() => assertSdkWorkspaceUsesSubmodulePaths({ repoRoot }));
  assert.doesNotThrow(() =>
    assertSdkWorkspaceUsesSubmodulePaths({
      repoRoot,
      manifestPath: writeManifest(VALID_SDK_DEPENDENCIES),
    }),
  );
});

test("workspace guard rejects Git-backed, wrong, missing, duplicate, and mixed SDK declarations", () => {
  const gitBacked = VALID_SDK_DEPENDENCIES.replace(
    'path = "vendor/matrix-rust-sdk/crates/matrix-sdk"',
    'git = "https://example.invalid/sdk.git", rev = "0123456789012345678901234567890123456789"',
  );
  const wrongPath = VALID_SDK_DEPENDENCIES.replace(
    "vendor/matrix-rust-sdk/crates/matrix-sdk-ui",
    "vendor/other-sdk/crates/matrix-sdk-ui",
  );
  const missing = VALID_SDK_DEPENDENCIES.replace(/^matrix-sdk-search.*\n/m, "");
  const duplicate = `${VALID_SDK_DEPENDENCIES}\nmatrix-sdk-ui = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-ui" }\n`;
  const mixed = VALID_SDK_DEPENDENCIES.replace(
    'matrix-sdk-base = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-base" }',
    'matrix-sdk-base = { path = "vendor/matrix-rust-sdk/crates/matrix-sdk-base", git = "https://example.invalid/sdk.git" }',
  );

  for (const manifest of [gitBacked, wrongPath, missing, duplicate, mixed]) {
    assert.throws(
      () =>
        assertSdkWorkspaceUsesSubmodulePaths({
          repoRoot,
          manifestPath: writeManifest(manifest),
        }),
      /must resolve from vendor Matrix SDK submodule paths/,
    );
  }
});

test("check-sdk-submodule CLI fails with a private-data-free diagnostic for stale status", () => {
  const fixtureDir = mkdtempSync(join(tmpdir(), "koushi-sdk-submodule-"));
  const fixturePath = join(fixtureDir, "status.txt");
  writeFileSync(
    fixturePath,
    "+18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n",
  );

  const result = spawnSync(
    process.execPath,
    ["scripts/check-sdk-submodule.mjs", "--status-fixture", fixturePath],
    { cwd: repoRoot, encoding: "utf8" },
  );

  assert.equal(result.status, 1);
  assert.match(result.stderr, /vendor Matrix SDK submodule is stale/);
  assert.doesNotMatch(result.stderr, /18cdc0ce/);
});

test("check-sdk-submodule CLI rejects a Git-backed manifest without printing its URL or revision", () => {
  const manifestPath = writeManifest(
    VALID_SDK_DEPENDENCIES.replace(
      'path = "vendor/matrix-rust-sdk/crates/matrix-sdk"',
      'git = "https://private.invalid/sdk.git", rev = "0123456789012345678901234567890123456789"',
    ),
  );

  const result = spawnSync(
    process.execPath,
    [
      "scripts/check-sdk-submodule.mjs",
      "--manifest-fixture",
      manifestPath,
    ],
    { cwd: repoRoot, encoding: "utf8" },
  );

  assert.equal(result.status, 1);
  assert.match(result.stderr, /must resolve from vendor Matrix SDK submodule paths/);
  assert.doesNotMatch(result.stderr, /private\.invalid/);
  assert.doesNotMatch(result.stderr, /01234567/);
});

test("ordinary Tauri dev and build entrypoints run the shared SDK guard first", () => {
  const packageJson = JSON.parse(
    readFileSync(join(repoRoot, "apps", "desktop", "package.json"), "utf8"),
  );
  const tauriConfig = JSON.parse(
    readFileSync(
      join(repoRoot, "apps", "desktop", "src-tauri", "tauri.conf.json"),
      "utf8",
    ),
  );
  const runScript = readFileSync(join(repoRoot, "scripts", "run.sh"), "utf8");

  assert.equal(
    packageJson.scripts["guard:sdk"],
    "node ../../scripts/check-sdk-submodule.mjs",
  );
  assert.equal(
    tauriConfig.build.beforeDevCommand,
    "npm run guard:sdk && npm run dev:tauri",
  );
  assert.equal(
    tauriConfig.build.beforeBuildCommand,
    "npm run guard:sdk && npm run build",
  );
  assert.match(
    runScript,
    /\bnpm\s+--prefix\s+apps\/desktop\s+run\s+tauri\s+--\s+dev(?:\s|$)/,
  );
});
