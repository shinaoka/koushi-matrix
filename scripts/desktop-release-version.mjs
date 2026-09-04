#!/usr/bin/env node

import { appendFileSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const options = parseArguments(process.argv.slice(2));
const repoRoot = resolve(options.root ?? defaultRoot);

try {
  const current = readVersionsFromDisk(repoRoot);
  const version = requireConsistentSemVer(current, "current");

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
  const parsed = { root: null, before: null, githubOutput: false };
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (argument === "--root" || argument === "--before") {
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
        "Usage: node scripts/desktop-release-version.mjs [--root PATH] [--before GIT_REF] [--github-output]"
      );
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  return parsed;
}

function readVersionsFromDisk(root) {
  return {
    package: parseJsonVersion(readFileSync(join(root, "apps/desktop/package.json"), "utf8")),
    tauri: parseJsonVersion(
      readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8")
    ),
    cargo: parseCargoVersion(
      readFileSync(join(root, "apps/desktop/src-tauri/Cargo.toml"), "utf8")
    ),
  };
}

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

function parseCargoVersion(source) {
  const packageHeader = /^\[package\]\s*$/m.exec(source);
  if (!packageHeader) {
    throw new Error("Cargo manifest has no [package] section");
  }
  const remainder = source.slice(packageHeader.index + packageHeader[0].length);
  const nextSection = /^\[/m.exec(remainder);
  const packageSection = nextSection ? remainder.slice(0, nextSection.index) : remainder;
  const version = packageSection && /^version\s*=\s*"([^"]+)"\s*$/m.exec(packageSection)?.[1];
  if (!version) {
    throw new Error("Cargo manifest has no [package] version");
  }
  return version;
}

function requireConsistentSemVer(versions, label) {
  const entries = Object.entries(versions);
  const uniqueVersions = new Set(entries.map(([, version]) => version));
  if (uniqueVersions.size !== 1) {
    const summary = entries.map(([manifest, version]) => `${manifest}=${version}`).join(", ");
    throw new Error(`release versions do not match (${label}): ${summary}`);
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
