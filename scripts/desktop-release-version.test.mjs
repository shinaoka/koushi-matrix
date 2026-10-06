#!/usr/bin/env node

import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const script = resolve(dirname(fileURLToPath(import.meta.url)), "desktop-release-version.mjs");

/** Write the five synchronized release files into a throwaway Git root. */
function fixtureRoot({
  packageVersion,
  tauriVersion,
  cargoVersion,
  lockVersion,
  npmLockVersion = packageVersion,
  npmLockPackageVersion = npmLockVersion
}) {
  const root = mkdtempSync(join(tmpdir(), "koushi-release-version-"));
  mkdirSync(join(root, "apps/desktop/src-tauri"), { recursive: true });
  writeFileSync(
    join(root, "apps/desktop/package.json"),
    `${JSON.stringify({ name: "koushi-desktop", version: packageVersion, private: true }, null, 2)}\n`
  );
  writeFileSync(
    join(root, "apps/desktop/package-lock.json"),
    `${JSON.stringify(
      {
        name: "koushi-desktop",
        version: npmLockVersion,
        lockfileVersion: 3,
        requires: true,
        packages: {
          "": { name: "koushi-desktop", version: npmLockPackageVersion },
          "node_modules/left-pad": { version: "1.3.0" }
        }
      },
      null,
      2
    )}\n`
  );
  writeFileSync(
    join(root, "apps/desktop/src-tauri/tauri.conf.json"),
    `{\n  "productName": "Koushi",\n  "version": "${tauriVersion}",\n  "identifier": "chat.koushi.desktop",\n  "plugins": { "updater": { "version": "keep" } }\n}\n`
  );
  writeFileSync(
    join(root, "apps/desktop/src-tauri/Cargo.toml"),
    `[package]\nname = "koushi-desktop"\nversion = "${cargoVersion}"\nedition = "2024"\n\n[dependencies]\nserde = { version = "1" }\n`
  );
  writeFileSync(
    join(root, "Cargo.lock"),
    `version = 4\n\n[[package]]\nname = "koushi-state"\nversion = "0.1.0"\n\n[[package]]\nname = "koushi-desktop"\nversion = "${lockVersion}"\ndependencies = [\n "serde",\n]\n`
  );
  track(root);
  return root;
}

/** Discovery reads `git ls-files`, so fixture files must be tracked. */
function track(root) {
  const init = spawnSync("git", ["init", "-q"], { cwd: root });
  assert.equal(init.status, 0);
  const add = spawnSync("git", ["add", "-A"], { cwd: root });
  assert.equal(add.status, 0);
}

function writeTracked(root, path, contents) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), contents);
  track(root);
}

const synchronized = {
  packageVersion: "1.2.3",
  tauriVersion: "1.2.3",
  cargoVersion: "1.2.3",
  lockVersion: "1.2.3"
};

function run(root, ...extra) {
  return spawnSync(process.execPath, [script, "--root", root, ...extra], { encoding: "utf8" });
}

test("accepts a release whose lockfile matches the three manifests", () => {
  const result = run(
    fixtureRoot({
      packageVersion: "1.2.3",
      tauriVersion: "1.2.3",
      cargoVersion: "1.2.3",
      lockVersion: "1.2.3"
    })
  );
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^version=1\.2\.3$/m);
  assert.match(result.stdout, /^tag=v1\.2\.3$/m);
});

test("rejects a release that bumped the manifests but left Cargo.lock behind", () => {
  const result = run(
    fixtureRoot({
      packageVersion: "1.2.3",
      tauriVersion: "1.2.3",
      cargoVersion: "1.2.3",
      lockVersion: "1.2.2"
    })
  );
  assert.equal(result.status, 1);
  assert.match(result.stderr, /release versions do not match \(current\)/);
  assert.match(result.stderr, /lock=1\.2\.2/);
  assert.match(result.stderr, /refresh Cargo\.lock/);
});

test("still rejects a manifest that disagrees with the others", () => {
  const result = run(
    fixtureRoot({
      packageVersion: "1.2.3",
      tauriVersion: "1.2.4",
      cargoVersion: "1.2.3",
      lockVersion: "1.2.3"
    })
  );
  assert.equal(result.status, 1);
  assert.match(result.stderr, /tauri=1\.2\.4/);
});

test("reports a lockfile with no koushi-desktop package", () => {
  const root = fixtureRoot({
    packageVersion: "1.2.3",
    tauriVersion: "1.2.3",
    cargoVersion: "1.2.3",
    lockVersion: "1.2.3"
  });
  writeFileSync(join(root, "Cargo.lock"), 'version = 4\n\n[[package]]\nname = "serde"\nversion = "1"\n');
  const result = run(root);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Cargo\.lock has no koushi-desktop package version/);
});

test("rejects a release that left the npm lockfile root version behind (#1137)", () => {
  const result = run(fixtureRoot({ ...synchronized, npmLockVersion: "1.2.2" }));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /release versions do not match \(current\)/);
  assert.match(result.stderr, /npmLock=1\.2\.2/);
});

test('rejects an npm lockfile whose packages[""] entry is stale (#1137)', () => {
  const result = run(fixtureRoot({ ...synchronized, npmLockPackageVersion: "1.2.2" }));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /npmLockPackage=1\.2\.2/);
});

test("rejects a tracked desktop version file that the guard does not list", () => {
  for (const [path, contents] of [
    // A platform Tauri config that overrides the bundle version.
    ["apps/desktop/src-tauri/tauri.macos.conf.json", '{\n  "version": "1.2.3"\n}\n'],
    // A second lockfile for the same npm package.
    [
      "apps/desktop/npm-shrinkwrap.json",
      JSON.stringify({ name: "koushi-desktop", version: "1.2.3", packages: {} })
    ],
    // Another Cargo manifest claiming the desktop crate name.
    ["packaging/Cargo.toml", '[package]\nname = "koushi-desktop"\nversion = "1.2.3"\n']
  ]) {
    const root = fixtureRoot(synchronized);
    writeTracked(root, path, contents);
    const result = run(root);
    assert.equal(result.status, 1, path);
    assert.match(result.stderr, new RegExp(`unguarded desktop version file\\(s\\): ${path}`));
  }
});

test("ignores version-bearing files of other packages", () => {
  const root = fixtureRoot(synchronized);
  writeTracked(
    root,
    "tests/windows-overlay-acl/tauri.conf.json",
    '{\n  "version": "0.0.0",\n  "identifier": "chat.koushi.windows-overlay-acl-test"\n}\n'
  );
  writeTracked(root, "apps/desktop/safe-stubs/stub/package.json", '{"name":"stub","version":"9.9.9"}');
  writeTracked(root, "apps/desktop/src-tauri/tauri.linux.conf.json", '{\n  "bundle": {}\n}\n');
  const result = run(root);
  assert.equal(result.status, 0, result.stderr);
});

test("--set rewrites every version file and nothing else", () => {
  const root = fixtureRoot({ ...synchronized, npmLockVersion: "1.2.0", lockVersion: "1.2.1" });
  const read = (path) => readFileSync(join(root, path), "utf8");
  const before = {
    tauri: read("apps/desktop/src-tauri/tauri.conf.json"),
    cargo: read("apps/desktop/src-tauri/Cargo.toml"),
    npmLock: read("apps/desktop/package-lock.json")
  };
  const result = run(root, "--set", "2.0.0-beta.1");
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^version=2\.0\.0-beta\.1$/m);
  assert.match(result.stdout, /^prerelease=true$/m);
  assert.equal(
    read("apps/desktop/src-tauri/tauri.conf.json"),
    before.tauri.replace('"version": "1.2.3"', '"version": "2.0.0-beta.1"')
  );
  assert.equal(
    read("apps/desktop/src-tauri/Cargo.toml"),
    before.cargo.replace('version = "1.2.3"', 'version = "2.0.0-beta.1"')
  );
  assert.equal(
    read("apps/desktop/package-lock.json"),
    before.npmLock.replace(/"1\.2\.0"/g, '"2.0.0-beta.1"')
  );
  assert.match(read("Cargo.lock"), /name = "koushi-desktop"\nversion = "2\.0\.0-beta\.1"/);
  assert.match(read("Cargo.lock"), /name = "koushi-state"\nversion = "0\.1\.0"/);
  assert.equal(run(root).status, 0);
});

test("--set rejects an invalid SemVer without writing", () => {
  const root = fixtureRoot(synchronized);
  const packageBefore = readFileSync(join(root, "apps/desktop/package.json"), "utf8");
  const result = run(root, "--set", "1.2");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /invalid release SemVer: 1\.2/);
  assert.equal(readFileSync(join(root, "apps/desktop/package.json"), "utf8"), packageBefore);
});

test("the checked-in repository is synchronized and fully guarded", () => {
  const result = spawnSync(process.execPath, [script], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
});
