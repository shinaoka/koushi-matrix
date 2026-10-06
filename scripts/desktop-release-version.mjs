#!/usr/bin/env node

import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, posix, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

/**
 * Every tracked file that carries the desktop release version, with how to
 * read and rewrite it. This table is the single source for the consistency
 * check, `--set`, and the unguarded-file discovery below.
 *
 * Lockfiles are part of the release change, not incidental artifacts. A
 * lockfile left behind by a bump is rewritten by the next tool invocation on
 * any branch and lands the release bump in an unrelated PR: `Cargo.lock` after
 * v0.11.1 (#955), and `apps/desktop/package-lock.json`, whose root version
 * `npm ci` never validates, through v0.18.0 and v0.19.0 (#1137).
 */
const VERSION_FILES = [
  {
    path: "apps/desktop/package.json",
    read: (source) => ({ package: parseJsonVersion(source) }),
    write: (source, version) => replaceTopLevelJsonVersion(source, version),
  },
  {
    path: "apps/desktop/package-lock.json",
    read: parsePackageLockVersions,
    write: (source, version) => {
      const lock = JSON.parse(source);
      lock.version = version;
      lock.packages[""].version = version;
      return `${JSON.stringify(lock, null, 2)}\n`;
    },
  },
  {
    path: "apps/desktop/src-tauri/tauri.conf.json",
    read: (source) => ({ tauri: parseJsonVersion(source) }),
    write: (source, version) => replaceTopLevelJsonVersion(source, version),
  },
  {
    path: "apps/desktop/src-tauri/Cargo.toml",
    read: (source) => ({ cargo: parseCargoVersion(source) }),
    write: replaceCargoVersion,
  },
  {
    path: "Cargo.lock",
    read: (source) => ({ lock: parseLockVersion(source) }),
    write: replaceLockVersion,
  },
];

const DESKTOP_PACKAGE_NAME = "koushi-desktop";
const DESKTOP_TAURI_DIRECTORY = "apps/desktop/src-tauri";
const DESKTOP_TAURI_IDENTIFIER = "chat.koushi.desktop";

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const options = parseArguments(process.argv.slice(2));
const repoRoot = resolve(options.root ?? defaultRoot);

try {
  if (options.set) {
    parseSemVer(options.set);
    writeVersionsToDisk(repoRoot, options.set);
  }
  const current = readVersionsFromDisk(repoRoot);
  const version = requireConsistentSemVer(current, "current");
  requireEveryVersionFileGuarded(repoRoot);

  let proceed = true;
  if (options.before) {
    const previous = readVersionsFromGit(repoRoot, options.before);
    const previousVersion = requireConsistentSemVer(previous, options.before);
    const comparison = compareSemVer(version, previousVersion);
    if (comparison < 0) {
      throw new Error(
        `release version must increase: ${previousVersion} -> ${version}`
      );
    }
    if (comparison === 0) {
      // A manifest edit without a version bump (e.g. a dependency change in
      // Cargo.toml) is not a release request: skip instead of failing.
      console.error(`desktop-release-version: release version unchanged: ${version}`);
      proceed = false;
    }
  }

  const values = {
    version,
    tag: `v${version}`,
    prerelease: String(parseSemVer(version).prerelease.length > 0),
    proceed: String(proceed),
  };
  for (const [name, value] of Object.entries(values)) {
    console.log(`${name}=${value}`);
  }
  if (options.githubOutput) {
    const outputPath = process.env.GITHUB_OUTPUT;
    if (!outputPath) {
      throw new Error("GITHUB_OUTPUT is required with --github-output");
    }
    appendFileSync(
      outputPath,
      Object.entries(values)
        .map(([name, value]) => `${name}=${value}\n`)
        .join(""),
      "utf8"
    );
  }
} catch (error) {
  console.error(`desktop-release-version: ${error.message}`);
  process.exit(1);
}

function parseArguments(argumentsList) {
  const parsed = { root: null, before: null, set: null, githubOutput: false };
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (argument === "--root" || argument === "--before" || argument === "--set") {
      const value = argumentsList[index + 1];
      if (!value) {
        throw new Error(`${argument} requires a value`);
      }
      parsed[argument.slice(2)] = value;
      index += 1;
    } else if (argument === "--github-output") {
      parsed.githubOutput = true;
    } else if (argument === "--help") {
      console.log(
        "Usage: node scripts/desktop-release-version.mjs [--root PATH] [--set VERSION | --before GIT_REF] [--github-output]"
      );
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  if (parsed.set && parsed.before) {
    throw new Error("--set cannot be combined with --before");
  }
  return parsed;
}

function readVersionsFromDisk(root) {
  return Object.assign(
    {},
    ...VERSION_FILES.map(({ path, read }) => read(readFileSync(join(root, path), "utf8")))
  );
}

function writeVersionsToDisk(root, version) {
  // Render every file before writing any, so a parse failure leaves the tree untouched.
  const updates = VERSION_FILES.map(({ path, write }) => {
    const file = join(root, path);
    return [file, write(readFileSync(file, "utf8"), version)];
  });
  for (const [file, contents] of updates) {
    writeFileSync(file, contents, "utf8");
  }
}

/**
 * Fail when a tracked file declares the desktop package's version but is not
 * listed in VERSION_FILES. Without this, the next version-bearing file (a new
 * lockfile, a platform Tauri config that overrides `version`, a second package
 * manifest) would drift silently exactly as package-lock.json did (#1137).
 */
function requireEveryVersionFileGuarded(root) {
  const guarded = new Set(VERSION_FILES.map(({ path }) => path));
  const unguarded = listTrackedFiles(root).filter(
    (path) => !guarded.has(path) && declaresDesktopVersion(path, () => readFileSync(join(root, path), "utf8"))
  );
  if (unguarded.length > 0) {
    throw new Error(
      `unguarded desktop version file(s): ${unguarded.join(", ")}; ` +
        "add each to VERSION_FILES in scripts/desktop-release-version.mjs"
    );
  }
}

function listTrackedFiles(root) {
  const result = spawnSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error("cannot list tracked files to discover version-bearing files");
  }
  return result.stdout.split("\0").filter(Boolean);
}

function declaresDesktopVersion(path, readSource) {
  const name = posix.basename(path);
  if (name === "package.json" || name === "package-lock.json" || name === "npm-shrinkwrap.json") {
    const manifest = parseJsonOrNull(readSource());
    const root = manifest?.packages?.[""];
    return (
      (manifest?.name === DESKTOP_PACKAGE_NAME && typeof manifest.version === "string") ||
      (root?.name === DESKTOP_PACKAGE_NAME && typeof root.version === "string")
    );
  }
  if (name === "Cargo.toml") {
    return cargoPackageName(readSource()) === DESKTOP_PACKAGE_NAME;
  }
  if (name === "Cargo.lock") {
    return lockEntryPattern().test(readSource().replace(/\r\n/g, "\n"));
  }
  if (/^tauri(\.[^.]+)?\.conf\.json5?$/.test(name) || /^Tauri(\.[^.]+)?\.toml$/.test(name)) {
    const source = readSource();
    const config = parseJsonOrNull(source);
    const isDesktop =
      posix.dirname(path) === DESKTOP_TAURI_DIRECTORY || config?.identifier === DESKTOP_TAURI_IDENTIFIER;
    const hasVersion = config
      ? Object.hasOwn(config, "version")
      : /^\s*"?version"?\s*[:=]/m.test(source);
    return isDesktop && hasVersion;
  }
  return false;
}

function parseJsonOrNull(source) {
  try {
    return JSON.parse(source);
  } catch {
    return null;
  }
}

/**
 * Read the previous commit's manifests.
 *
 * The lockfiles are deliberately excluded here: this path only needs a version
 * to compare against, and a historical commit predating a lockfile rule must
 * not hard-fail the release workflow. The current-state read above enforces
 * them.
 */
function readVersionsFromGit(root, reference) {
  return {
    package: parseJsonVersion(readGitFile(root, reference, "apps/desktop/package.json")),
    tauri: parseJsonVersion(
      readGitFile(root, reference, "apps/desktop/src-tauri/tauri.conf.json")
    ),
    cargo: parseCargoVersion(
      readGitFile(root, reference, "apps/desktop/src-tauri/Cargo.toml")
    ),
  };
}

function readGitFile(root, reference, path) {
  if (!/^[0-9a-f]{40}$/i.test(reference)) {
    throw new Error("--before must be a full Git commit SHA");
  }
  const result = spawnSync("git", ["show", `${reference}:${path}`], {
    cwd: root,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`cannot read ${path} at the previous commit`);
  }
  return result.stdout;
}

function parseJsonVersion(source) {
  const version = JSON.parse(source).version;
  if (typeof version !== "string") {
    throw new Error("JSON manifest has no string version");
  }
  return version;
}

function parsePackageLockVersions(source) {
  const lock = JSON.parse(source);
  const rootPackage = lock.packages?.[""];
  if (typeof lock.version !== "string" || typeof rootPackage?.version !== "string") {
    throw new Error('package-lock.json has no root and packages[""] version');
  }
  return { npmLock: lock.version, npmLockPackage: rootPackage.version };
}

function replaceTopLevelJsonVersion(source, version) {
  // Rewrite the line in place so hand-formatted JSON (tauri.conf.json) keeps
  // its layout; the re-parse guards against touching a nested `version`.
  const pattern = /^( {2}"version"\s*:\s*")[^"]*(")/m;
  if (!pattern.test(source)) {
    throw new Error("JSON manifest has no top-level version line");
  }
  const updated = source.replace(pattern, `$1${version}$2`);
  if (parseJsonVersion(updated) !== version) {
    throw new Error("JSON manifest version rewrite did not reach the top-level version");
  }
  return updated;
}

function cargoPackageSection(source) {
  const packageHeader = /^\[package\]\s*$/m.exec(source);
  if (!packageHeader) {
    return null;
  }
  const start = packageHeader.index + packageHeader[0].length;
  const remainder = source.slice(start);
  const nextSection = /^\[/m.exec(remainder);
  return { start, end: start + (nextSection ? nextSection.index : remainder.length) };
}

function cargoPackageName(source) {
  const section = cargoPackageSection(source);
  return section
    ? /^name\s*=\s*"([^"]+)"\s*$/m.exec(source.slice(section.start, section.end))?.[1]
    : undefined;
}

function parseCargoVersion(source) {
  const section = cargoPackageSection(source);
  if (!section) {
    throw new Error("Cargo manifest has no [package] section");
  }
  const packageSection = source.slice(section.start, section.end);
  const version = /^version\s*=\s*"([^"]+)"\s*$/m.exec(packageSection)?.[1];
  if (!version) {
    throw new Error("Cargo manifest has no [package] version");
  }
  return version;
}

function replaceCargoVersion(source, version) {
  parseCargoVersion(source);
  const section = cargoPackageSection(source);
  const packageSection = source
    .slice(section.start, section.end)
    .replace(/^(version\s*=\s*")[^"]+(")/m, `$1${version}$2`);
  return source.slice(0, section.start) + packageSection + source.slice(section.end);
}

function lockEntryPattern() {
  return /(\[\[package\]\]\r?\nname = "koushi-desktop"\r?\nversion = ")([^"]+)("\r?\n)/;
}

function parseLockVersion(source) {
  const entry = lockEntryPattern().exec(source);
  if (!entry) {
    throw new Error("Cargo.lock has no koushi-desktop package version");
  }
  return entry[2];
}

function replaceLockVersion(source, version) {
  parseLockVersion(source);
  return source.replace(lockEntryPattern(), `$1${version}$3`);
}

function requireConsistentSemVer(versions, label) {
  const entries = Object.entries(versions);
  const uniqueVersions = new Set(entries.map(([, version]) => version));
  if (uniqueVersions.size !== 1) {
    const summary = entries.map(([manifest, version]) => `${manifest}=${version}`).join(", ");
    const hint = versions.lock
      ? " (refresh Cargo.lock with `cargo metadata --format-version 1 >/dev/null`," +
        " or rewrite every version file with `node scripts/desktop-release-version.mjs --set <version>`)"
      : "";
    throw new Error(`release versions do not match (${label}): ${summary}${hint}`);
  }
  const version = entries[0][1];
  parseSemVer(version);
  return version;
}

function compareSemVer(left, right) {
  const a = parseSemVer(left);
  const b = parseSemVer(right);
  for (const key of ["major", "minor", "patch"]) {
    if (a[key] !== b[key]) {
      return a[key] < b[key] ? -1 : 1;
    }
  }
  if (a.prerelease.length === 0 || b.prerelease.length === 0) {
    return a.prerelease.length === b.prerelease.length ? 0 : a.prerelease.length === 0 ? 1 : -1;
  }
  const length = Math.max(a.prerelease.length, b.prerelease.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = a.prerelease[index];
    const rightPart = b.prerelease[index];
    if (leftPart === undefined || rightPart === undefined) {
      return leftPart === rightPart ? 0 : leftPart === undefined ? -1 : 1;
    }
    if (leftPart === rightPart) {
      continue;
    }
    const leftNumeric = /^\d+$/.test(leftPart);
    const rightNumeric = /^\d+$/.test(rightPart);
    if (leftNumeric && rightNumeric) {
      return Number(leftPart) < Number(rightPart) ? -1 : 1;
    }
    if (leftNumeric !== rightNumeric) {
      return leftNumeric ? -1 : 1;
    }
    return leftPart < rightPart ? -1 : 1;
  }
  return 0;
}

function parseSemVer(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(
    version
  );
  if (!match) {
    throw new Error(`invalid release SemVer: ${version}`);
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease: match[4] ? match[4].split(".") : [],
  };
}
