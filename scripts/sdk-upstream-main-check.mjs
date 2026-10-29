#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { applyIssueActions, listIssues } from "./lib/github-issues.mjs";

// Measures how far the pinned `vendor/matrix-rust-sdk` fork commit trails
// upstream `main` and keeps the single open `sdk-update` issue in step with
// it. The fork merges upstream but is not a GitHub fork, so the GitHub compare
// API cannot relate the two; both histories are fetched (without blobs) into
// one scratch repository and compared there.
const defaultUpstream = "https://github.com/matrix-org/matrix-rust-sdk.git";
const upstreamWeb = "https://github.com/matrix-org/matrix-rust-sdk";
const issueLabel = "sdk-update";
const issueTitle = "Sync the vendored Matrix Rust SDK fork with upstream main";
const upstreamRef = "refs/upstream/main";

const options = parseArguments(process.argv.slice(2));

try {
  const pinned = options.pinned ?? pinnedGitlink();
  const fork = options.fork ?? submoduleUrl();
  const workDir = options.workDir ?? join(mkdtempSync(join(tmpdir(), "koushi-sdk-main-")), "work.git");
  fetchHistories({ workDir, upstream: options.upstream, fork, pinned });

  const lag = measureLag(workDir, pinned);
  for (const [name, value] of Object.entries({
    pinned,
    upstream_head: lag.head.sha,
    merge_base: lag.mergeBase,
    upstream_ahead: lag.ahead.length,
    fork_only: lag.forkOnly,
  })) {
    console.log(`${name}=${value}`);
  }
  for (const [crate, count] of lag.crates) {
    console.log(`crate_touched=${crate}:${count}`);
  }

  const openIssues = listIssues({ label: issueLabel, issuesJson: options.issuesJson }).filter(
    (issue) => issue.state === "OPEN"
  );
  applyIssueActions(planIssueActions(openIssues, pinned, lag), { dryRun: options.dryRun });
} catch (error) {
  console.error(`sdk-upstream-main-check: ${error.message}`);
  process.exit(1);
}

function parseArguments(argumentsList) {
  const parsed = {
    upstream: defaultUpstream,
    fork: null,
    pinned: null,
    workDir: null,
    issuesJson: null,
    dryRun: false,
  };
  const valued = ["--upstream", "--fork", "--pinned", "--work-dir", "--issues-json"];
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (valued.includes(argument)) {
      const value = argumentsList[index + 1];
      if (!value) {
        throw new Error(`${argument} requires a value`);
      }
      parsed[argument.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = value;
      index += 1;
    } else if (argument === "--dry-run") {
      parsed.dryRun = true;
    } else if (argument === "--help") {
      console.log(
        "Usage: node scripts/sdk-upstream-main-check.mjs [--upstream URL] [--fork URL] " +
          "[--pinned SHA] [--work-dir DIR] [--issues-json PATH] [--dry-run]"
      );
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  return parsed;
}

function git(args, { cwd } = {}) {
  const result = spawnSync("git", args, { cwd, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  if (result.status !== 0) {
    throw new Error(`git ${args[0]} failed: ${result.stderr?.trim() || `exit ${result.status}`}`);
  }
  return result.stdout.trim();
}

/** The SDK commit the koushi checkout pins; the gitlink is the only SDK pin. */
function pinnedGitlink() {
  const entry = git(["ls-tree", "HEAD", "vendor/matrix-rust-sdk"]);
  const sha = /^160000 commit ([0-9a-f]{40})\t/.exec(entry)?.[1];
  if (!sha) {
    throw new Error("HEAD has no vendor/matrix-rust-sdk gitlink");
  }
  return sha;
}

function submoduleUrl() {
  return git(["config", "-f", ".gitmodules", "--get", "submodule.vendor/matrix-rust-sdk.url"]);
}

function fetchHistories({ workDir, upstream, fork, pinned }) {
  if (!existsSync(workDir)) {
    git(["init", "-q", "--bare", workDir]);
  }
  const fetch = ["fetch", "-q", "--no-tags", "--filter=blob:none"];
  git([...fetch, upstream, `+refs/heads/main:${upstreamRef}`], { cwd: workDir });
  git([...fetch, fork, pinned], { cwd: workDir });
}

function measureLag(workDir, pinned) {
  const run = (args) => git(args, { cwd: workDir });
  const range = `${pinned}..${upstreamRef}`;
  const ahead = run(["log", "--reverse", "--format=%H %cs", range])
    .split("\n")
    .filter(Boolean)
    .map((line) => {
      const [sha, date] = line.split(" ");
      return { sha, date };
    });
  const [headSha, headDate] = run(["log", "-1", "--format=%H %cs", upstreamRef]).split(" ");
  return {
    ahead,
    head: { sha: headSha, date: headDate },
    mergeBase: run(["merge-base", pinned, upstreamRef]),
    forkOnly: Number(run(["rev-list", "--count", `${upstreamRef}..${pinned}`])),
    crates: cratesTouched(run(["log", "--format=%x00", "--name-only", range])),
  };
}

/** Upstream commits per `crates/<name>`, most touched first. */
function cratesTouched(log) {
  const counts = new Map();
  for (const commit of log.split("\0")) {
    const crates = new Set(
      commit
        .split("\n")
        .map((path) => /^crates\/([^/]+)\//.exec(path)?.[1])
        .filter(Boolean)
    );
    for (const crate of crates) {
      counts.set(crate, (counts.get(crate) ?? 0) + 1);
    }
  }
  return [...counts].sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]));
}

function planIssueActions(openIssues, pinned, lag) {
  if (lag.ahead.length === 0) {
    return openIssues.map((issue) => ({
      kind: "close",
      number: issue.number,
      comment: `The pinned fork commit \`${pinned}\` now contains upstream \`main\` (\`${lag.head.sha}\`).`,
    }));
  }
  const body = issueBody(pinned, lag);
  const [current, ...duplicates] = [...openIssues].sort((left, right) => left.number - right.number);
  if (!current) {
    return [{ kind: "create", title: issueTitle, body, labels: [issueLabel] }];
  }
  const actions = duplicates.map((issue) => ({
    kind: "close",
    number: issue.number,
    comment: `Duplicate of #${current.number}, which tracks the upstream lag.`,
  }));
  if (current.title !== issueTitle || current.body.trim() !== body.trim()) {
    actions.unshift({ kind: "edit", number: current.number, title: issueTitle, body });
  }
  return actions;
}

function issueBody(pinned, lag) {
  const oldest = lag.ahead[0];
  const newest = lag.ahead[lag.ahead.length - 1];
  const short = (sha) => sha.slice(0, 12);
  const crates = lag.crates.length
    ? lag.crates.map(([crate, count]) => `- \`${crate}\`: ${count}`).join("\n")
    : "- none under `crates/`";
  return `The pinned \`vendor/matrix-rust-sdk\` fork commit lags upstream \`matrix-org/matrix-rust-sdk\` \`main\`.

- pinned fork commit: \`${pinned}\`
- upstream \`main\`: \`${lag.head.sha}\` (${lag.head.date})
- upstream commits not in the pinned fork: **${lag.ahead.length}** (oldest \`${short(oldest.sha)}\` ${oldest.date}, newest \`${short(newest.sha)}\` ${newest.date})
- fork-only commits (the fork delta): ${lag.forkOnly}
- upstream range: ${upstreamWeb}/compare/${lag.mergeBase}...${lag.head.sha}

Upstream commits per crate:

${crates}

Merge upstream \`main\` into the fork and move the gitlink as described in
\`docs/agents/environment.md\` (Following upstream). The
\`sdk-upstream-main-check\` workflow updates this issue and closes it once the
pinned fork commit contains upstream \`main\`.
`;
}
