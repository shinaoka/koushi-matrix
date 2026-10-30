#!/usr/bin/env node
// Record a passing nightly run on each open `nightly-qa` issue it covers, and
// close an issue once its scenario has passed in PASSES_TO_CLOSE consecutive
// nightly runs after a fix landed (REPOSITORY_RULES "GitHub Issue Closure").
//
//   node scripts/nightly-qa-record-passes.mjs --server=tuwunel --run-id=<id> \
//     --failures=failures.json --run-url=<url>        # focused job
//   node scripts/nightly-qa-record-passes.mjs --server=tuwunel --run-id=<id> \
//     --scenario=aggregate --run-url=<url>            # aggregate job, on success
//
// An issue's streak is read back from its own comments: the trailing workflow
// pass records (one per run id, so a re-run of the same run counts once). Any
// other workflow comment (the failure comment the same workflow posts on a
// recurrence) ends the streak, so one failure resets the count without a
// separate state store. A pass counts only when the checked-out `main` already
// has a commit that references the issue: a failure that stops recurring on its
// own, with no fix, stays open for triage.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { NIGHTLY_SCENARIOS } from "./lib/headless-qa-lanes.mjs";

export const PASSES_TO_CLOSE = 3;
export const PASS_MARKER = "<!-- nightly-qa:pass";
const WORKFLOW_AUTHORS = new Set(["github-actions", "github-actions[bot]", "app/github-actions"]);

function optionValue(name) {
  const prefix = `${name}=`;
  return process.argv.slice(2).find((arg) => arg.startsWith(prefix))?.slice(prefix.length);
}

export function passRecord(runId, fixLanded) {
  return `${PASS_MARKER} run=${runId} fix=${fixLanded ? 1 : 0} -->`;
}

function parsePassRecord(body) {
  const match = body.match(/<!-- nightly-qa:pass run=(\d+) fix=([01]) -->/);
  return match === null ? undefined : { runId: match[1], fixLanded: match[2] === "1" };
}

// The run ids of the consecutive post-fix passing runs recorded at the end of an
// issue's comment history. Comments by people are ignored: only the workflow's
// own records count. A pass recorded before a fix landed ends the streak.
export function passStreak(comments) {
  const runIds = new Set();
  for (const comment of [...comments].reverse()) {
    if (!WORKFLOW_AUTHORS.has(comment.author)) continue;
    const record = parsePassRecord(comment.body);
    if (record === undefined || !record.fixLanded) break;
    runIds.add(record.runId);
  }
  return runIds;
}

export function recordedRunIds(comments) {
  return new Set(
    comments
      .filter((comment) => WORKFLOW_AUTHORS.has(comment.author))
      .map((comment) => parsePassRecord(comment.body)?.runId)
      .filter((runId) => runId !== undefined)
  );
}

// The focused job's passing scenarios: the nightly lane minus its recorded
// failures. With no failure record the lane runner itself did not finish, and
// nothing is known to have passed.
export function passedScenarios(failures) {
  if (failures === undefined) return [];
  const failed = new Set(failures.map(({ scenario }) => scenario));
  return NIGHTLY_SCENARIOS.filter((scenario) => !failed.has(scenario));
}

function gh(args) {
  const result = spawnSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  if (result.status !== 0) throw new Error(`gh ${args[0]} ${args[1]} failed`);
  return result.stdout;
}

// Whether the checked-out history has a commit that references issue `number`.
function fixLandedFor(number) {
  const result = spawnSync(
    "git",
    ["log", "-E", `--grep=#${number}([^0-9]|$)`, "--format=%H", "-1", "HEAD"],
    { encoding: "utf8" }
  );
  if (result.status !== 0) throw new Error("git log failed");
  return result.stdout.trim() !== "";
}

function recordPass(server, scenario, runId, runUrl) {
  const signature = `nightly-qa/${server}/${scenario}`;
  const open = JSON.parse(
    gh(["issue", "list", "--label", "nightly-qa", "--label", signature, "--state", "open",
      "--limit", "100", "--json", "number"])
  );
  for (const { number } of open) {
    const { comments } = JSON.parse(gh(["issue", "view", String(number), "--json", "comments"]));
    const history = comments.map((comment) => ({
      author: comment.author?.login ?? "",
      body: comment.body ?? ""
    }));
    if (recordedRunIds(history).has(runId)) continue;
    const fixLanded = fixLandedFor(number);
    const streak = fixLanded ? passStreak(history).add(runId).size : 0;
    const progress = fixLanded
      ? `${streak} of ${PASSES_TO_CLOSE} consecutive passing runs since a fix landed`
      : "not counted: no commit on `main` references this issue yet";
    gh(["issue", "comment", String(number), "--body",
      `${passRecord(runId, fixLanded)}\nThe nightly \`${scenario}\` scenario passed on ` +
        `\`${server}\` (${progress}): ${runUrl}`]);
    if (streak >= PASSES_TO_CLOSE) {
      gh(["issue", "close", String(number), "--reason", "completed", "--comment",
        `Closing: \`${scenario}\` passed on \`${server}\` in ${PASSES_TO_CLOSE} consecutive ` +
        "nightly runs after its fix landed. A recurrence reopens this issue."]);
    }
    console.log(`nightly-qa-record-passes: #${number} ${signature} streak=${streak}`);
  }
}

function main() {
  const server = optionValue("--server");
  const runUrl = optionValue("--run-url");
  const runId = optionValue("--run-id");
  const scenario = optionValue("--scenario");
  const failuresPath = optionValue("--failures");
  if (!["tuwunel", "synapse"].includes(server) || runUrl === undefined
    || !/^\d+$/.test(runId ?? "")
    || (scenario === undefined) === (failuresPath === undefined)) {
    console.error(
      "usage: nightly-qa-record-passes.mjs --server=tuwunel|synapse --run-id=<id> --run-url=<url> " +
        "(--scenario=aggregate | --failures=<failures.json>)"
    );
    process.exit(2);
  }
  const scenarios = scenario !== undefined
    ? [scenario]
    : passedScenarios(existsSync(failuresPath)
      ? JSON.parse(readFileSync(failuresPath, "utf8"))
      : undefined);
  for (const passed of scenarios) recordPass(server, passed, runId, runUrl);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
