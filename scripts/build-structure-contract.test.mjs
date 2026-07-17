#!/usr/bin/env node
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function readRepoFile(path) {
  return readFileSync(join(repoRoot, path), "utf8");
}

test("root workspace owns the desktop Tauri crate and the only lockfile", () => {
  const rootCargo = readRepoFile("Cargo.toml");
  const tauriCargo = readRepoFile("apps/desktop/src-tauri/Cargo.toml");

  assert.match(rootCargo, /"apps\/desktop\/src-tauri"/);
  assert.doesNotMatch(tauriCargo, /^\[workspace\]$/m);
  assert.equal(existsSync(join(repoRoot, "Cargo.lock")), true);
  assert.equal(existsSync(join(repoRoot, "apps/desktop/src-tauri/Cargo.lock")), false);
});

test("vendored Matrix SDK crates are consumed as one rev-pinned git source", () => {
  const rootCargo = readRepoFile("Cargo.toml");
  const sdkCargo = readRepoFile("crates/koushi-sdk/Cargo.toml");
  const coreCargo = readRepoFile("crates/koushi-core/Cargo.toml");
  const gitmodules = readRepoFile(".gitmodules");

  assert.match(rootCargo, /^\[workspace\.dependencies\]$/m);
  assert.match(rootCargo, /matrix-sdk = \{ git = "https:\/\/github\.com\/shinaoka\/matrix-rust-sdk-work\.git", rev = "34ed8377478b47c58f3c08889d69d4ee5a0791ca"/);
  assert.match(rootCargo, /matrix-sdk-ui = \{ git = "https:\/\/github\.com\/shinaoka\/matrix-rust-sdk-work\.git", rev = "34ed8377478b47c58f3c08889d69d4ee5a0791ca"/);
  assert.match(gitmodules, /url = https:\/\/github\.com\/shinaoka\/matrix-rust-sdk-work\.git/);
  assert.doesNotMatch(gitmodules, /^\s*branch\s*=/m);
  assert.doesNotMatch(sdkCargo, /vendor\/matrix-rust-sdk/);
  assert.doesNotMatch(coreCargo, /vendor\/matrix-rust-sdk/);
});

test("toolchain and dev dependency profile are pinned for stable incremental builds", () => {
  const rootCargo = readRepoFile("Cargo.toml");
  const toolchain = readRepoFile("rust-toolchain.toml");

  assert.match(toolchain, /channel = "1\.96\.0"/);
  assert.match(toolchain, /targets = \["wasm32-unknown-unknown"\]/);
  assert.match(rootCargo, /^\[profile\.dev\.package\."\*"\]$/m);
  assert.match(rootCargo, /^debug = false$/m);
});

test("CI and npm scripts use the unified workspace contracts", () => {
  const packageJson = readRepoFile("apps/desktop/package.json");
  const ci = readRepoFile(".github/workflows/ci.yml");
  const releaseGate = readRepoFile("scripts/desktop-release-gate-check.mjs");

  assert.doesNotMatch(packageJson, /--manifest-path src-tauri\/Cargo\.toml/);
  assert.match(packageJson, /cargo test -p koushi-desktop/);
  assert.doesNotMatch(ci, /apps\/desktop\/src-tauri\s*$/m);
  assert.match(ci, /cargo test -p koushi-desktop/);
  assert.match(releaseGate, /cargo check[\s\S]*-p[\s\S]*koushi-desktop/);
});

test("submodule guard is wired into commit and QA entrypoints", () => {
  const preCommit = readRepoFile(".githooks/pre-commit");
  const headless = readRepoFile("scripts/desktop-headless-local-qa.mjs");
  const real = readRepoFile("scripts/desktop-real-homeserver-qa.mjs");
  const linux = readRepoFile("scripts/desktop-linux-gui-qa.mjs");
  const mac = readRepoFile("scripts/desktop-mac-gui-smoke.mjs");
  const releaseGate = readRepoFile("scripts/desktop-release-gate-check.mjs");

  assert.match(preCommit, /check-sdk-submodule\.mjs/);
  for (const source of [headless, real, linux, mac, releaseGate]) {
    assert.match(source, /sdk-submodule-status\.mjs/);
  }
});

test("headless core QA can run cargo binaries with the release profile", () => {
  const packageJson = readRepoFile("apps/desktop/package.json");
  const headless = readRepoFile("scripts/desktop-headless-local-qa.mjs");

  assert.match(headless, /optionValue\("--cargo-profile"\)/);
  assert.match(headless, /cargoProfileArgs/);
  assert.match(headless, /--cargo-profile=dev\|release/);
  assert.match(headless, /explicitCoreBackendOption/);
  assert.match(headless, /defaultCoreBackendForScenario\(scenarioOption, cargoProfileOption\)/);
  assert.match(headless, /--cargo-profile=release cannot run LegacySync/);
  assert.match(headless, /function selectedScenarios/);
  assert.match(headless, /for \(const scenario of scenarios\)/);
  assert.match(headless, /KOUSHI_QA_SCENARIO: scenario/);
  assert.match(
    packageJson,
    /"qa:headless-core": "node \.\.\/\.\.\/scripts\/desktop-headless-local-qa\.mjs --run --server=both --core --scenario=login_sync,directory,timeline_reconnect --timeout-ms=600000 --cargo-profile=release && node \.\.\/\.\.\/scripts\/desktop-headless-local-qa\.mjs --run --server=conduit --core --scenario=send_queue --timeout-ms=600000 --cargo-profile=release"/
  );
  assert.match(
    packageJson,
    /"qa:headless-basic:local": "node \.\.\/\.\.\/scripts\/desktop-headless-local-qa\.mjs --run --server=both --core --scenario=login_sync,directory,timeline_reconnect --timeout-ms=600000 --cargo-profile=release && node \.\.\/\.\.\/scripts\/desktop-headless-local-qa\.mjs --run --server=conduit --core --scenario=send_queue --timeout-ms=600000 --cargo-profile=release"/
  );
});
