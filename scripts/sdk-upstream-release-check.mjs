#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";

// Upstream publishes one GitHub release per crate, so only `matrix-sdk-x.y.z`
// releases carry the version the vendored fork is based on.
const matrixSdkRelease = /^matrix-sdk-(\d+\.\d+\.\d+)$/;
const defaultUpstreamRepo = "matrix-org/matrix-rust-sdk";

const options = parseArguments(process.argv.slice(2));

try {
  const baseVersion = parseCargoPackageVersion(
    readFileSync(requireValue(options.sdkManifest, "--sdk-manifest"), "utf8")
  );
  const latestVersion = latestMatrixSdkRelease(readUpstreamReleases());
  const values = {
    base_version: baseVersion,
    latest_version: latestVersion,
    update_required: String(compareVersions(latestVersion, baseVersion) > 0),
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
  console.error(`sdk-upstream-release-check: ${error.message}`);
  process.exit(1);
}

function parseArguments(argumentsList) {
  const parsed = { sdkManifest: null, releasesJson: null, repository: null, githubOutput: false };
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (
      argument === "--sdk-manifest" ||
      argument === "--releases-json" ||
      argument === "--repository"
    ) {
      const value = argumentsList[index + 1];
      if (!value) {
        throw new Error(`${argument} requires a value`);
      }
      parsed[
        argument
          .slice(2)
          .replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())
      ] = value;
      index += 1;
    } else if (argument === "--github-output") {
      parsed.githubOutput = true;
    } else if (argument === "--help") {
      console.log(
        "Usage: node scripts/sdk-upstream-release-check.mjs --sdk-manifest PATH " +
          "[--releases-json PATH] [--repository OWNER/NAME] [--github-output]"
      );
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  return parsed;
}

function requireValue(value, name) {
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

/** Version of the `matrix-sdk` package the pinned SDK revision declares. */
function parseCargoPackageVersion(source) {
  const packageHeader = /^\[package\]\s*$/m.exec(source);
  if (!packageHeader) {
    throw new Error("SDK manifest has no [package] section");
  }
  const remainder = source.slice(packageHeader.index + packageHeader[0].length);
  const nextSection = /^\[/m.exec(remainder);
  const packageSection = nextSection ? remainder.slice(0, nextSection.index) : remainder;
  const version = packageSection && /^version\s*=\s*"([^"]+)"\s*$/m.exec(packageSection)?.[1];
  if (!version) {
    throw new Error("SDK manifest has no [package] version");
  }
  return version;
}

function readUpstreamReleases() {
  if (options.releasesJson) {
    return JSON.parse(readFileSync(options.releasesJson, "utf8")).flat();
  }
  const repository = options.repository ?? defaultUpstreamRepo;
  const result = spawnSync(
    "gh",
    ["api", "--paginate", "--slurp", `repos/${repository}/releases?per_page=100`],
    { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }
  );
  if (result.status !== 0) {
    throw new Error(
      `gh api failed for ${repository}: ${result.stderr?.trim() || `exit ${result.status}`}`
    );
  }
  return JSON.parse(result.stdout).flat();
}

/** Newest published (non-draft, non-prerelease) `matrix-sdk` release version. */
function latestMatrixSdkRelease(releases) {
  const versions = releases
    .filter((release) => !release.draft && !release.prerelease)
    .map((release) => matrixSdkRelease.exec(release.tag_name ?? "")?.[1])
    .filter(Boolean);
  if (versions.length === 0) {
    throw new Error("no published matrix-sdk release found upstream");
  }
  return versions.reduce((latest, version) =>
    compareVersions(version, latest) > 0 ? version : latest
  );
}

function compareVersions(left, right) {
  const leftParts = versionParts(left);
  const rightParts = versionParts(right);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] !== rightParts[index]) {
      return leftParts[index] < rightParts[index] ? -1 : 1;
    }
  }
  return 0;
}

function versionParts(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!match) {
    throw new Error(`unsupported release version: ${version}`);
  }
  return match.slice(1).map(Number);
}
