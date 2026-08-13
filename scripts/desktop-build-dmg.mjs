#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const dmgDir = join(repoRoot, "target", "release", "bundle", "dmg");
const args = new Set(process.argv.slice(2));

if (args.has("--help")) {
  printUsage();
  process.exit(0);
}

if (process.platform !== "darwin" && !args.has("--print-command")) {
  console.error("desktop-build-dmg: DMG bundling is only available on macOS.");
  process.exit(1);
}

printStorageNotice();

const bundleVersion = macOSBundleVersion();
const buildEnvironment = localSigningEnvironment();
const buildCommand = [
  "run",
  "tauri",
  "--",
  "build",
  "--bundles",
  "dmg",
  "--config",
  JSON.stringify({ bundle: { macOS: { bundleVersion } } })
];
if (args.has("--print-command")) {
  console.log(`desktop-build-dmg: npm ${buildCommand.join(" ")}`);
  process.exit(0);
}

if (args.has("--signed")) {
  run(
    "node",
    ["scripts/desktop-release-preflight.mjs", "--macos-signing"],
    repoRoot
  );
} else if (!args.has("--skip-preflight")) {
  run(
    "node",
    ["scripts/desktop-release-preflight.mjs", "--check-config"],
    repoRoot
  );
}

run("npm", buildCommand, desktopDir);

const dmgFiles = listDmgArtifacts();
if (dmgFiles.length === 0) {
  console.error(`desktop-build-dmg: build finished but no .dmg was found under ${dmgDir}`);
  process.exit(1);
}

console.log("desktop-build-dmg: artifacts");
for (const artifact of dmgFiles) {
  console.log(`  ${artifact}`);
}

function run(command, commandArgs, cwd) {
  const result = spawnSync(command, commandArgs, {
    cwd,
    stdio: "inherit",
    env: buildEnvironment
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function localSigningEnvironment() {
  const environment = { ...process.env };
  if (process.platform !== "darwin") {
    return environment;
  }

  const identities = spawnSync("security", ["find-identity", "-v", "-p", "codesigning"], {
    encoding: "utf8"
  });
  if (identities.status !== 0) {
    if (environment.APPLE_SIGNING_IDENTITY) {
      console.error("desktop-build-dmg: could not validate APPLE_SIGNING_IDENTITY");
      process.exit(1);
    }
    console.warn("desktop-build-dmg: no usable signing identity; falling back to ad-hoc signing");
    return environment;
  }
  // `security find-identity -v` returns only identities whose certificate and
  // private key currently form a valid code-signing identity.  Keep both the
  // fingerprint and display name so an explicit environment value can use
  // either representation without bypassing validation.
  const validIdentities = [...identities.stdout.matchAll(/\b([0-9A-F]{40})\s+"([^"]+)"/g)]
    .map((match) => ({ fingerprint: match[1], name: match[2] }));
  if (environment.APPLE_SIGNING_IDENTITY) {
    const requested = environment.APPLE_SIGNING_IDENTITY;
    if (!validIdentities.some(({ fingerprint, name }) => requested === fingerprint || requested === name)) {
      console.error("desktop-build-dmg: APPLE_SIGNING_IDENTITY is not a valid local code-signing identity");
      process.exit(1);
    }
    console.log("desktop-build-dmg: using validated APPLE_SIGNING_IDENTITY from the environment");
    return environment;
  }

  const developerIds = validIdentities
    .map(({ name }) => name)
    .filter((name) => name.startsWith("Developer ID Application: "));
  const uniqueDeveloperIds = [...new Set(developerIds)];
  if (uniqueDeveloperIds.length !== 1) {
    console.warn(
      `desktop-build-dmg: expected one Developer ID Application identity, found ${uniqueDeveloperIds.length}; ` +
        "set APPLE_SIGNING_IDENTITY to select one, otherwise this build is ad-hoc signed"
    );
    return environment;
  }

  environment.APPLE_SIGNING_IDENTITY = uniqueDeveloperIds[0];
  console.log("desktop-build-dmg: using the locally installed Developer ID Application identity");
  return environment;
}

function listDmgArtifacts() {
  if (!existsSync(dmgDir)) {
    return [];
  }
  return readdirSync(dmgDir)
    .filter((file) => file.endsWith(".dmg"))
    .sort()
    .map((file) => join(dmgDir, file));
}

function macOSBundleVersion() {
  const commitCount = spawnSync("git", ["rev-list", "--count", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8"
  });
  if (commitCount.status !== 0 || !/^\d+$/.test(commitCount.stdout.trim())) {
    console.error("desktop-build-dmg: failed to derive a macOS bundle version from git");
    process.exit(commitCount.status ?? 1);
  }

  const dirty = spawnSync("git", ["status", "--porcelain", "--untracked-files=no"], {
    cwd: repoRoot,
    encoding: "utf8"
  });
  if (dirty.status !== 0) {
    console.error("desktop-build-dmg: failed to inspect the worktree");
    process.exit(dirty.status ?? 1);
  }
  return `${commitCount.stdout.trim()}.${dirty.stdout.trim() ? "1" : "0"}`;
}

function printStorageNotice() {
  console.log("desktop-build-dmg: local installed-app storage");
  console.log("  data: ~/Library/Application Support/koushi-desktop");
  console.log("  encrypted Matrix store/search/cache: data/accounts/<account>/");
  console.log("  credential service: macOS Keychain service koushi-desktop");
}

function printUsage() {
  console.log("Usage: npm --prefix apps/desktop run build:dmg [-- --signed|--skip-preflight]");
  console.log("Builds the local macOS DMG via Tauri: tauri build --bundles dmg");
  printStorageNotice();
}
