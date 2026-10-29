#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const script = resolve(dirname(fileURLToPath(import.meta.url)), "sdk-upstream-main-check.mjs");

function git(cwd, ...args) {
  const result = spawnSync("git", args, {
    cwd,
    encoding: "utf8",
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: "Fixture",
      GIT_AUTHOR_EMAIL: "fixture@example.invalid",
      GIT_COMMITTER_NAME: "Fixture",
      GIT_COMMITTER_EMAIL: "fixture@example.invalid",
    },
  });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}

function commit(repo, path, message) {
  mkdirSync(dirname(join(repo, path)), { recursive: true });
  writeFileSync(join(repo, path), `${message}\n`);
  git(repo, "add", path);
  git(repo, "commit", "-q", "-m", message);
  return git(repo, "rev-parse", "HEAD");
}

/**
 * An upstream repository and a fork cloned from it. The fork carries one
 * fork-only commit on top of `forkBase`, so the fixture mirrors the real
 * fork's shape: it merges upstream rather than being a GitHub fork.
 */
function fixture({ upstreamAfterFork = [] }) {
  const root = mkdtempSync(join(tmpdir(), "koushi-sdk-main-"));
  const upstream = join(root, "upstream");
  mkdirSync(upstream);
  git(upstream, "init", "-q", "-b", "main");
  commit(upstream, "crates/matrix-sdk/src/lib.rs", "initial");
  const fork = join(root, "fork");
  git(root, "clone", "-q", upstream, fork);
  const pinned = commit(fork, "crates/matrix-sdk-ui/src/fork.rs", "fork-only accessor");
  for (const [path, message] of upstreamAfterFork) {
    commit(upstream, path, message);
  }
  return { root, upstream, fork, pinned };
}

function run({ upstream, fork, pinned, root }, issues = []) {
  const issuesJson = join(root, "issues.json");
  writeFileSync(issuesJson, JSON.stringify(issues));
  return spawnSync(
    process.execPath,
    [
      script,
      "--upstream",
      upstream,
      "--fork",
      fork,
      "--pinned",
      pinned,
      "--work-dir",
      join(root, "work.git"),
      "--issues-json",
      issuesJson,
      "--dry-run",
    ],
    { encoding: "utf8" }
  );
}

const openUpdateIssue = {
  number: 1700,
  title: "Sync the vendored Matrix Rust SDK fork with upstream main",
  state: "OPEN",
  body: "stale",
};

test("up to date: no lag and no issue to open", () => {
  const result = run(fixture({}));
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^upstream_ahead=0$/m);
  assert.match(result.stdout, /^fork_only=1$/m);
  assert.match(result.stdout, /^issue_action=none$/m);
});

test("up to date: closes the open sdk-update issue", () => {
  const result = run(fixture({}), [
    openUpdateIssue,
    { number: 1188, title: "older closed issue", state: "CLOSED", body: "" },
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^issue_action=close #1700$/m);
  assert.doesNotMatch(result.stdout, /#1188/);
});

test("behind: reports the upstream range and opens one issue", () => {
  const repos = fixture({
    upstreamAfterFork: [
      ["crates/matrix-sdk-ui/src/timeline.rs", "ui: fix timeline"],
      ["crates/matrix-sdk/src/client.rs", "sdk: fix client"],
      ["crates/matrix-sdk-ui/src/room_list.rs", "ui: fix room list"],
      ["README.md", "docs"],
    ],
  });
  const result = run(repos, [
    { number: 1188, title: "Update the vendored fork base", state: "CLOSED", body: "" },
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^upstream_ahead=4$/m);
  assert.match(result.stdout, /^fork_only=1$/m);
  assert.match(
    result.stdout,
    /^issue_action=create Sync the vendored Matrix Rust SDK fork with upstream main$/m
  );
  assert.match(result.stdout, /upstream commits not in the pinned fork: \*\*4\*\*/);
  assert.match(result.stdout, /`matrix-sdk-ui`: 2\n.*`matrix-sdk`: 1/);
  assert.match(result.stdout, new RegExp(`pinned fork commit: \`${repos.pinned}\``));
});

test("behind with an open issue: updates it instead of opening another", () => {
  const repos = fixture({
    upstreamAfterFork: [["crates/matrix-sdk/src/client.rs", "sdk: fix client"]],
  });
  const result = run(repos, [openUpdateIssue]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^issue_action=edit #1700$/m);
  assert.doesNotMatch(result.stdout, /^issue_action=create/m);
});

test("behind with an unchanged open issue: leaves it alone", () => {
  const repos = fixture({
    upstreamAfterFork: [["crates/matrix-sdk/src/client.rs", "sdk: fix client"]],
  });
  const first = run(repos, []);
  const body = first.stdout
    .split("\n")
    .filter((line) => line.startsWith("  | "))
    .map((line) => line.slice(4))
    .join("\n");
  const result = run(repos, [{ ...openUpdateIssue, body }]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^issue_action=none$/m);
});

test("behind with duplicate open issues: keeps the oldest and closes the rest", () => {
  const repos = fixture({
    upstreamAfterFork: [["crates/matrix-sdk/src/client.rs", "sdk: fix client"]],
  });
  const result = run(repos, [{ ...openUpdateIssue, number: 1710 }, openUpdateIssue]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^issue_action=edit #1700$/m);
  assert.match(result.stdout, /^issue_action=close #1710$/m);
});

test("fails when the pinned commit is missing from the fork", () => {
  const repos = fixture({});
  const result = run({ ...repos, pinned: "0".repeat(40) });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /sdk-upstream-main-check:/);
});
