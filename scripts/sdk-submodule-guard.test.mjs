#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  assertSdkSubmoduleSynced,
  parseSubmoduleStatus,
  readPinnedSdkRevision,
} from "./lib/sdk-submodule-status.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

test("parseSubmoduleStatus accepts only initialized in-sync SDK status", () => {
  assert.deepEqual(
    parseSubmoduleStatus(" 18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n"),
    {
      ok: true,
      state: "synced",
      path: "vendor/matrix-rust-sdk",
      revision: "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
    },
  );

  assert.deepEqual(
    parseSubmoduleStatus("-18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n"),
    {
      ok: false,
      state: "uninitialized",
      path: "vendor/matrix-rust-sdk",
      revision: "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
    },
  );

  assert.deepEqual(
    parseSubmoduleStatus("+18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n"),
    {
      ok: false,
      state: "stale",
      path: "vendor/matrix-rust-sdk",
      revision: "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
    },
  );
});

test("readPinnedSdkRevision reads one shared SDK revision from the root workspace", () => {
  assert.equal(readPinnedSdkRevision({ repoRoot }), "7e370d82965b3159776311bfa6cddb3aed1d402c");
});

test("assertSdkSubmoduleSynced rejects a gitlink that differs from the pinned SDK revision", () => {
  const fixtureDir = mkdtempSync(join(tmpdir(), "koushi-sdk-submodule-mismatch-"));
  const fixturePath = join(fixtureDir, "status.txt");
  writeFileSync(
    fixturePath,
    " f13238024a83d5d6fd03540e023aed1e54fc7393 vendor/matrix-rust-sdk\n",
  );

  assert.throws(
    () =>
      assertSdkSubmoduleSynced({
        repoRoot,
        fixturePath,
        expectedRevision: "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
      }),
    /does not match the pinned SDK revision/,
  );
});

test("check-sdk-submodule CLI fails with a private-data-free diagnostic for stale status", () => {
  const fixtureDir = mkdtempSync(join(tmpdir(), "koushi-sdk-submodule-"));
  const fixturePath = join(fixtureDir, "status.txt");
  writeFileSync(
    fixturePath,
    "+18cdc0ceab8aacce1a57953f897d7f7a3e88834e vendor/matrix-rust-sdk\n",
  );

  const result = spawnSync(
    process.execPath,
    ["scripts/check-sdk-submodule.mjs", "--status-fixture", fixturePath],
    { cwd: repoRoot, encoding: "utf8" },
  );

  assert.equal(result.status, 1);
  assert.match(result.stderr, /vendor Matrix SDK submodule is stale/);
  assert.doesNotMatch(result.stderr, /18cdc0ce/);
});

test("check-sdk-submodule CLI fails with a private-data-free diagnostic for mismatched revision", () => {
  const fixtureDir = mkdtempSync(join(tmpdir(), "koushi-sdk-submodule-mismatch-cli-"));
  const fixturePath = join(fixtureDir, "status.txt");
  writeFileSync(
    fixturePath,
    " f13238024a83d5d6fd03540e023aed1e54fc7393 vendor/matrix-rust-sdk\n",
  );

  const result = spawnSync(
    process.execPath,
    [
      "scripts/check-sdk-submodule.mjs",
      "--status-fixture",
      fixturePath,
      "--expected-rev",
      "18cdc0ceab8aacce1a57953f897d7f7a3e88834e",
    ],
    { cwd: repoRoot, encoding: "utf8" },
  );

  assert.equal(result.status, 1);
  assert.match(result.stderr, /does not match the pinned SDK revision/);
  assert.doesNotMatch(result.stderr, /18cdc0ce/);
  assert.doesNotMatch(result.stderr, /f1323802/);
});
