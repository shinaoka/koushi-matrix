#!/usr/bin/env node
// Windows x64 build entry point (issue #441 Phase A).
//
// The default mode produces ONE unsigned NSIS trial installer. The explicit
// build-only and bundle-only modes are also used by the future external
// signing pipeline so a signed main executable can be inserted between the
// two Tauri operations.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const nsisDir = join(
  repoRoot,
  "target",
  "x86_64-pc-windows-msvc",
  "release",
  "bundle",
  "nsis"
);
const executable = join(
  repoRoot,
  "target",
  "x86_64-pc-windows-msvc",
  "release",
  "koushi-desktop.exe"
);
const args = new Set(process.argv.slice(2));
const modes = ["--build-only", "--bundle-only"].filter((mode) => args.has(mode));
const mode = modes[0] ?? "--trial";
const signedInput = args.has("--signed-input");
// Windows has no bare npm executable: route child processes through cmd.exe
// (shell: true) so .cmd shims like npm.cmd resolve via PATHEXT. spawnSync
// returns {error} on ENOENT instead of throwing, so surface it explicitly.
const isWindows = process.platform === "win32";

if (args.has("--help")) {
  printUsage();
  process.exit(0);
}

if (modes.length > 1) {
  console.error("desktop-build-windows: choose at most one build mode");
  process.exit(1);
}
if (signedInput && mode !== "--bundle-only") {
  console.error("desktop-build-windows: --signed-input requires --bundle-only");
  process.exit(1);
}

if (process.platform !== "win32" && !args.has("--print-command")) {
  console.error("desktop-build-windows: Windows NSIS bundling is only available on Windows.");
  process.exit(1);
}

printStorageNotice();

const buildCommand = ["run", "tauri", "--"];
if (mode === "--build-only") {
  buildCommand.push("build");
  buildCommand.push("--no-bundle");
} else if (mode === "--bundle-only") {
  buildCommand.push("bundle");
  buildCommand.push("--bundles", "nsis", "--no-binary-patching");
} else {
  buildCommand.push("build");
  buildCommand.push("--bundles", "nsis");
}
buildCommand.push("--target", "x86_64-pc-windows-msvc");
if (args.has("--print-command")) {
  console.log(`desktop-build-windows: npm ${buildCommand.join(" ")}`);
  process.exit(0);
}

if (!args.has("--skip-preflight")) {
  run("node", ["scripts/desktop-release-preflight.mjs", "--check-config"], repoRoot);
}

const buildStartMs = Date.now();
if (mode === "--bundle-only" && signedInput) {
  validateFile(executable, "signed main executable");
}
run("npm", buildCommand, desktopDir);

if (mode === "--build-only") {
  validateFile(executable, "main executable", buildStartMs);
  console.log("desktop-build-windows: build-only main executable");
  console.log("  artifact: " + executable);
  console.log("  sha256:   " + sha256Of(executable));
  process.exit(0);
}

const installers = listNsisInstallers();
if (installers.length !== 1) {
  console.error(
    `desktop-build-windows: expected exactly one NSIS installer under ${nsisDir}, found ${installers.length}`
  );
  process.exit(1);
}

const installer = installers[0];
validateFile(installer, "NSIS installer", buildStartMs);

const sha256 = sha256Of(installer);
console.log(
  mode === "--bundle-only" && signedInput
    ? "desktop-build-windows: NSIS bundle created from verified signed input"
    : "desktop-build-windows: UNSIGNED trial installer (not code-signed; Windows SmartScreen may warn)"
);
console.log("  artifact: " + installer);
console.log("  sha256:   " + sha256);

function run(command, commandArgs, cwd) {
  const result = spawnSync(command, commandArgs, {
    cwd,
    stdio: "inherit",
    env: process.env,
    shell: isWindows
  });
  if (result.error) {
    console.error(`desktop-build-windows: failed to spawn ${command}: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function listNsisInstallers() {
  if (!existsSync(nsisDir)) {
    return [];
  }
  return readdirSync(nsisDir)
    .filter((file) => file.endsWith(".exe"))
    .sort()
    .map((file) => join(nsisDir, file));
}

function validateFile(file, label, minimumMtimeMs = undefined) {
  if (!existsSync(file)) {
    console.error(`desktop-build-windows: ${label} not found: ${file}`);
    process.exit(1);
  }
  const stat = statSync(file);
  if (stat.size === 0) {
    console.error(`desktop-build-windows: ${label} is empty: ${file}`);
    process.exit(1);
  }
  if (minimumMtimeMs !== undefined && stat.mtimeMs < minimumMtimeMs) {
    console.error(`desktop-build-windows: ${label} is stale (not produced by this run): ${file}`);
    process.exit(1);
  }
}

function sha256Of(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function printStorageNotice() {
  console.log("desktop-build-windows: local installed-app storage");
  console.log("  data: %APPDATA%\\koushi-desktop (encrypted Matrix store/search/cache)");
  console.log("  credential service: Windows Credential Manager (koushi-desktop)");
}

function printUsage() {
  console.log(
    "Usage: npm --prefix apps/desktop run build:windows [-- --build-only|--bundle-only [--signed-input]|--print-command|--skip-preflight]"
  );
  console.log("Builds or bundles the Windows x64 NSIS installer via Tauri.");
  printStorageNotice();
}
