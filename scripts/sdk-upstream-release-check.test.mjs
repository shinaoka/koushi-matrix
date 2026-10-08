#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const script = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "sdk-upstream-release-check.mjs"
);

const upstreamReleases = [
  { tag_name: "matrix-sdk-0.19.1", prerelease: false, draft: false },
  { tag_name: "matrix-sdk-0.19.0", prerelease: false, draft: false },
  { tag_name: "matrix-sdk-0.18.0", prerelease: false, draft: false },
  { tag_name: "matrix-sdk-crypto-0.11.1", prerelease: false, draft: false },
  { tag_name: "matrix-sdk-ui-0.19.1", prerelease: false, draft: false },
  { tag_name: "matrix-sdk-0.20.0-rc.1", prerelease: true, draft: false },
  { tag_name: "matrix-sdk-0.20.0", prerelease: false, draft: true },
];

function run({ version, releases = upstreamReleases, githubOutput = true }) {
  const root = mkdtempSync(join(tmpdir(), "koushi-sdk-upstream-"));
  const manifest = join(root, "Cargo.toml");
  const releasesPath = join(root, "releases.json");
  writeFileSync(
    manifest,
    `[package]\nname = "matrix-sdk"\nversion = "${version}"\n\n[dependencies]\nruma = { version = "0.12" }\n`
  );
  writeFileSync(releasesPath, JSON.stringify(releases));
  const outputPath = join(root, "github-output.txt");
  const result = spawnSync(
    process.execPath,
    [
      script,
      "--sdk-manifest",
      manifest,
      "--releases-json",
      releasesPath,
      ...(githubOutput ? ["--github-output"] : []),
    ],
    { encoding: "utf8", env: { ...process.env, GITHUB_OUTPUT: outputPath } }
  );
  return {
    result,
    output: () => readFileSync(outputPath, "utf8"),
  };
}

test("reports an update when upstream released a newer matrix-sdk", () => {
  const { result, output } = run({ version: "0.18.0" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^base_version=0\.18\.0$/m);
  assert.match(result.stdout, /^latest_version=0\.19\.1$/m);
  assert.match(result.stdout, /^update_required=true$/m);
  assert.match(output(), /^update_required=true$/m);
});

test("reports no update when the pinned base matches the latest release", () => {
  const { result, output } = run({ version: "0.19.1" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^latest_version=0\.19\.1$/m);
  assert.match(result.stdout, /^update_required=false$/m);
  assert.match(output(), /^update_required=false$/m);
});

test("reports no update when the base is ahead of the published releases", () => {
  const { result } = run({ version: "0.20.0" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^update_required=false$/m);
});

test("ignores other crates, prereleases, and drafts", () => {
  const { result } = run({
    version: "0.19.1",
    releases: [
      { tag_name: "matrix-sdk-0.19.1", prerelease: false, draft: false },
      { tag_name: "matrix-sdk-0.20.0-rc.1", prerelease: true, draft: false },
      { tag_name: "matrix-sdk-0.20.0", prerelease: false, draft: true },
      { tag_name: "matrix-sdk-crypto-0.30.0", prerelease: false, draft: false },
      { tag_name: "matrix-sdk-ui-0.30.0", prerelease: false, draft: false },
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^latest_version=0\.19\.1$/m);
  assert.match(result.stdout, /^update_required=false$/m);
});

test("fails when the pinned manifest is missing", () => {
  const result = spawnSync(process.execPath, [script], { encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /--sdk-manifest is required/);
});
