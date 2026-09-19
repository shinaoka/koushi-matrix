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

function workflowJobSource(workflow, jobName) {
  const startMarker = `\n  ${jobName}:\n`;
  const start = workflow.indexOf(startMarker);
  assert.ok(start >= 0, `expected CI job ${jobName}`);
  const remainder = workflow.slice(start + startMarker.length);
  const nextJob = /\n  [a-z0-9_-]+:\n/i.exec(remainder);
  const end = nextJob ? start + startMarker.length + nextJob.index : workflow.length;
  return workflow.slice(start, end);
}

test("root workspace owns the desktop Tauri crate and the only lockfile", () => {
  const rootCargo = readRepoFile("Cargo.toml");
  const tauriCargo = readRepoFile("apps/desktop/src-tauri/Cargo.toml");

  assert.match(rootCargo, /"apps\/desktop\/src-tauri"/);
  assert.doesNotMatch(tauriCargo, /^\[workspace\]$/m);
  assert.equal(existsSync(join(repoRoot, "Cargo.lock")), true);
  assert.equal(existsSync(join(repoRoot, "apps/desktop/src-tauri/Cargo.lock")), false);
});

test("vendored Matrix SDK crates are consumed through root submodule paths", () => {
  const rootCargo = readRepoFile("Cargo.toml");
  const sdkCargo = readRepoFile("crates/koushi-sdk/Cargo.toml");
  const coreCargo = readRepoFile("crates/koushi-core/Cargo.toml");
  const gitmodules = readRepoFile(".gitmodules");
  const engineeringRules = readRepoFile("docs/policies/engineering-rules.md");
  const buildRulesStart = engineeringRules.indexOf("## Build, Dependencies, QA Gates");
  const buildRulesEnd = engineeringRules.indexOf("\n## ", buildRulesStart);

  assert.ok(buildRulesStart >= 0 && buildRulesEnd > buildRulesStart);
  const buildRules = engineeringRules.slice(buildRulesStart, buildRulesEnd);
  const sdkPolicyStart = buildRules.indexOf("\n1. ");
  const sdkPolicyEnd = buildRules.indexOf("\n2. ", sdkPolicyStart);

  assert.ok(sdkPolicyStart >= 0 && sdkPolicyEnd > sdkPolicyStart);
  const sdkDependencyPolicy = buildRules.slice(sdkPolicyStart, sdkPolicyEnd);

  assert.match(rootCargo, /^\[workspace\.dependencies\]$/m);
  assert.match(rootCargo, /matrix-sdk = \{ path = "vendor\/matrix-rust-sdk\/crates\/matrix-sdk"/);
  assert.match(rootCargo, /matrix-sdk-ui = \{ path = "vendor\/matrix-rust-sdk\/crates\/matrix-sdk-ui"/);
  assert.doesNotMatch(rootCargo, /^matrix-sdk(?:-[a-z]+)?\s*=\s*\{[^}]*\b(?:git|rev)\s*=/m);
  assert.match(gitmodules, /url = https:\/\/github\.com\/shinaoka\/matrix-rust-sdk-work\.git/);
  assert.doesNotMatch(gitmodules, /^\s*branch\s*=/m);
  assert.doesNotMatch(sdkCargo, /vendor\/matrix-rust-sdk/);
  assert.doesNotMatch(coreCargo, /vendor\/matrix-rust-sdk/);
  assert.doesNotMatch(sdkDependencyPolicy, /rev-pinned\s+git\s+dependency/);
  assert.doesNotMatch(sdkDependencyPolicy, /pinned\s+git\s+revision/);
  assert.doesNotMatch(sdkDependencyPolicy, /github\.com\/[^\s`]*matrix-rust-sdk/);
  assert.doesNotMatch(sdkDependencyPolicy, /app crates must not depend on it by local path/);
  assert.match(sdkDependencyPolicy, /root workspace Matrix SDK\s+dependencies/);
  assert.match(
    sdkDependencyPolicy,
    /use their exact paths beneath\s+`vendor\/matrix-rust-sdk`/
  );
});

test("toolchain and dev dependency profile are pinned for stable incremental builds", () => {
  const rootCargo = readRepoFile("Cargo.toml");
  const toolchain = readRepoFile("rust-toolchain.toml");
  const desktopPackage = JSON.parse(readRepoFile("apps/desktop/package.json"));
  const desktopLock = JSON.parse(readRepoFile("apps/desktop/package-lock.json"));

  assert.match(toolchain, /channel = "1\.96\.0"/);
  assert.match(toolchain, /targets = \["wasm32-unknown-unknown"\]/);
  assert.match(rootCargo, /^\[profile\.dev\.package\."\*"\]$/m);
  assert.match(rootCargo, /^debug = false$/m);
  assert.match(rootCargo, /^\[profile\.ci\]$/m);
  assert.match(rootCargo, /^inherits = "test"$/m);
  assert.match(rootCargo, /^debug = 0$/m);
  assert.match(rootCargo, /^debug-assertions = true$/m);
  assert.match(rootCargo, /^overflow-checks = true$/m);
  assert.match(rootCargo, /^incremental = false$/m);
  assert.equal(desktopPackage.overrides["deepmerge-ts"], "8.0.1");
  assert.equal(desktopLock.packages["node_modules/deepmerge-ts"].version, "8.0.1");
});

test("CI and npm scripts use the unified workspace contracts", () => {
  const packageJson = readRepoFile("apps/desktop/package.json");
  const ci = readRepoFile(".github/workflows/ci.yml");
  const rustJob = workflowJobSource(ci, "rust");
  const releaseGate = readRepoFile("scripts/desktop-release-gate-check.mjs");

  assert.doesNotMatch(packageJson, /--manifest-path src-tauri\/Cargo\.toml/);
  assert.match(packageJson, /cargo test -p koushi-desktop/);
  assert.doesNotMatch(ci, /apps\/desktop\/src-tauri\s*$/m);
  assert.match(
    rustJob,
    /cargo test --profile ci --workspace --exclude koushi-core-testkit --exclude koushi-desktop --exclude sidebar-composition --exclude key-management/,
  );
  assert.match(rustJob, /cargo test -p koushi-core-testkit --profile ci/);
  assert.match(rustJob, /cargo test -p koushi-desktop --profile ci/);
  assert.match(rustJob, /node --test scripts\/ci-rust-cache-report\.test\.mjs/);
  assert.match(rustJob, /node scripts\/ci-rust-cache-report\.mjs/);
  assert.match(ci, /node --test scripts\/check-rust-test-structure\.test\.mjs/);
  assert.match(ci, /node scripts\/check-rust-test-structure\.mjs/);
  assert.match(ci, /node --test[^\n]*check-leaf-crate-boundaries\.test\.mjs/);
  assert.match(ci, /node scripts\/check-leaf-crate-boundaries\.mjs/);
  assert.match(packageJson, /"lint:domain-deps": "[^"]*check-leaf-crate-boundaries\.mjs"/);
  assert.match(packageJson, /"lint:rust-test-structure": "node \.\.\/\.\.\/scripts\/check-rust-test-structure\.mjs"/);
  assert.match(releaseGate, /cargo check[\s\S]*-p[\s\S]*koushi-desktop/);
});

test("CI gates positive invitations on exactly Tuwunel and Synapse", () => {
  const ci = readRepoFile(".github/workflows/ci.yml");
  const invitationJob = workflowJobSource(ci, "core-invites");
  const binaryJob = workflowJobSource(ci, "core-homeserver");

  assert.match(invitationJob, /^\s{8}server: \[tuwunel, synapse\]$/m);
  assert.match(
    invitationJob,
    /--server=\$\{\{ matrix\.server \}\}[\s\S]*--scenario=invites_dm/
  );
  assert.doesNotMatch(invitationJob, /--core-backend|KOUSHI_QA_FORCE_SYNC_BACKEND/);
  assert.match(invitationJob, /if: matrix\.server == 'tuwunel'[\s\S]*actions\/cache@/);
  assert.match(invitationJob, /matrix-construct\/tuwunel\/releases\/download\/v1\.7\.1\//);
  assert.match(
    invitationJob,
    /64d6b60a781e2dad74e840ed6e211eced8c4206ce2d307fe62bbc62e3ffcc983/
  );
  assert.match(
    invitationJob,
    /key: tuwunel-v1\.7\.1-x86_64-v1-linux-gnu-64d6b60a781e2dad74e840ed6e211eced8c4206ce2d307fe62bbc62e3ffcc983/
  );
  const checksum = invitationJob.indexOf("sha256sum --check");
  const decompress = invitationJob.indexOf("unzstd");
  assert.ok(checksum >= 0 && decompress > checksum, "Tuwunel checksum must precede decompression");
  assert.match(invitationJob, /if: matrix\.server == 'synapse'\n\s+run: docker version/);
  assert.match(invitationJob, /name: core-invites-\$\{\{ matrix\.server \}\}-qa-logs/);
  assert.doesNotMatch(invitationJob, /(?:server:\s*\[[^\]]*conduit|--server=conduit)/i);
  assert.doesNotMatch(invitationJob, /(?:\|\s*(?:tee|grep)|\|\|\s*true|;\s*true)/);

  const actionUses = invitationJob.match(/^\s+(?:- )?uses: .+$/gm) ?? [];
  assert.equal(actionUses.length, 5);
  for (const actionUse of actionUses) {
    assert.match(actionUse, /@[0-9a-f]{40}\s+#\s+(?:v\d+|1\.96\.0)$/);
  }

  assert.match(binaryJob, /name: Core QA binary tests/);
  assert.match(binaryJob, /cargo test --profile ci -p koushi-qa --features qa-bin --bin headless-core-qa/);
  assert.doesNotMatch(binaryJob, /Conduit|conduit|--server=/);
});

test("submodule guard is wired into commit and QA entrypoints", () => {
  const preCommit = readRepoFile(".githooks/pre-commit");
  const headless = readRepoFile("scripts/desktop-headless-local-qa.mjs");
  const real = readRepoFile("scripts/desktop-real-homeserver-qa.mjs");
  const linux = [
    "scripts/desktop-linux-gui-qa.mjs",
    "scripts/desktop-linux-gui-qa/main.mjs",
    "scripts/desktop-linux-gui-qa/registry.mjs"
  ].map(readRepoFile).join("\n");
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
  assert.match(headless, /--cargo-profile=dev\|ci\|release/);
  assert.match(headless, /--core-backend is obsolete/);
  assert.match(headless, /KOUSHI_QA_FORCE_SYNC_BACKEND is obsolete/);
  assert.doesNotMatch(headless, /explicitCoreBackendOption/);
  assert.doesNotMatch(headless, /defaultCoreBackendForScenario/);
  assert.match(headless, /function selectedScenarios/);
  assert.match(headless, /for \(const scenario of scenarios\)/);
  assert.match(headless, /KOUSHI_QA_SCENARIO: scenario/);
  assert.match(
    packageJson,
    /"qa:headless-core": "node \.\.\/\.\.\/scripts\/desktop-headless-local-qa\.mjs --run --server=both --core --scenario=login_sync,directory,timeline_reconnect,send_queue --timeout-ms=600000 --cargo-profile=release"/
  );
  assert.match(
    packageJson,
    /"qa:headless-basic:local": "node \.\.\/\.\.\/scripts\/desktop-headless-local-qa\.mjs --run --server=both --core --scenario=login_sync,directory,timeline_reconnect,send_queue --timeout-ms=600000 --cargo-profile=release"/
  );
});

test("Rust CI cache paths match each job's explicit target directory", () => {
  const ci = readRepoFile(".github/workflows/ci.yml");
  assert.match(workflowJobSource(ci, "rust"), /CARGO_TARGET_DIR: \$\{\{ github\.workspace \}\}\/target-ci[\s\S]*workspaces: \. -> target-ci/);
  assert.match(workflowJobSource(ci, "macos-cargo-check"), /CARGO_TARGET_DIR: \$\{\{ github\.workspace \}\}\/target-macos-check[\s\S]*workspaces: \. -> target-macos-check/);
  assert.match(workflowJobSource(ci, "core-invites"), /CARGO_TARGET_DIR: \$\{\{ github\.workspace \}\}\/target-ci[\s\S]*workspaces: \. -> target-ci/);
  assert.match(workflowJobSource(ci, "core-homeserver"), /CARGO_TARGET_DIR: \$\{\{ github\.workspace \}\}\/target-ci[\s\S]*workspaces: \. -> target-ci/);
  assert.match(workflowJobSource(ci, "windows-overlay-acl"), /CARGO_TARGET_DIR: \$\{\{ github\.workspace \}\}\/target-windows-overlay[\s\S]*workspaces: \. -> target-windows-overlay/);
  assert.match(readRepoFile(".github/workflows/issue-738-flake-probe.yml"), /CARGO_TARGET_DIR: \$\{\{ github\.workspace \}\}\/target-ci[\s\S]*workspaces: \. -> target-ci/);
  assert.match(readRepoFile(".github/workflows/build-windows.yml"), /workspaces: \. -> target/);
  assert.match(
    readRepoFile(".github/workflows/issue-947-ci-benchmark.yml"),
    /cargo test --profile ci --workspace --exclude koushi-core-testkit --exclude koushi-desktop --exclude sidebar-composition --exclude key-management/,
  );
  assert.match(readRepoFile(".github/workflows/issue-947-ci-benchmark.yml"), /actions\/cache\/(?:save|restore)@/);
});
