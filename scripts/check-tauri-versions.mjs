#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// #1752: #1749 landed `@tauri-apps/api` 2.11.0 next to `tauri` 2.12.5 and CLI
// 2.12.1, and every required job stayed green because none of them installed
// the desktop workspace from scratch. The JS API and the CLI are compiled into
// the app the crate builds, so all three stay on one minor line.
const API_PACKAGE = "apps/desktop/node_modules/@tauri-apps/api/package.json";
const CLI_PACKAGE = "apps/desktop/node_modules/@tauri-apps/cli/package.json";

export function tauriCrateVersion(lockfile) {
  const block = lockfile
    .split(/^\[\[package\]\]$/m)
    .find((chunk) => /^name = "tauri"$/m.test(chunk));
  if (!block) throw new Error("Cargo.lock has no `tauri` package");
  const version = /^version = "([^"]+)"$/m.exec(block);
  if (!version) throw new Error("`tauri` package in Cargo.lock has no version");
  return version[1];
}

export function minorLine(version) {
  const [major, minor] = String(version).split(".");
  if (!major || !minor) throw new Error(`not a major.minor version: ${version}`);
  return `${major}.${minor}`;
}

export function checkTauriVersions({ crate, api, cli }) {
  const expected = minorLine(crate);
  return [
    ["@tauri-apps/api", api],
    ["@tauri-apps/cli", cli],
  ]
    .filter(([, version]) => minorLine(version) !== expected)
    .map(([name, version]) => `${name} ${version} is not on the tauri ${crate} minor line (${expected})`);
}

function packageVersion(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8")).version;
  } catch (error) {
    throw new Error(`cannot read ${path} (run \`npm ci\` in apps/desktop first): ${error.message}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const rootFlag = process.argv.indexOf("--root");
  const root = rootFlag === -1 ? fileURLToPath(new URL("..", import.meta.url)) : resolve(process.argv[rootFlag + 1]);
  const crate = tauriCrateVersion(readFileSync(resolve(root, "Cargo.lock"), "utf8"));
  const api = packageVersion(resolve(root, API_PACKAGE));
  const cli = packageVersion(resolve(root, CLI_PACKAGE));
  const problems = checkTauriVersions({ crate, api, cli });
  if (problems.length) {
    console.error(`Tauri versions disagree:\n  ${problems.join("\n  ")}`);
    console.error("Move the offending package to the crate's minor line and refresh apps/desktop/package-lock.json.");
    process.exitCode = 1;
  } else {
    console.log(`tauri ${crate}, @tauri-apps/api ${api}, @tauri-apps/cli ${cli} agree on minor ${minorLine(crate)}.`);
  }
}
