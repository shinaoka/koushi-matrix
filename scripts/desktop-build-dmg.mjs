#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const dmgDir = join(desktopDir, "src-tauri", "target", "release", "bundle", "dmg");
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

const buildCommand = ["run", "tauri", "--", "build", "--bundles", "dmg"];
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
    env: process.env
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
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
