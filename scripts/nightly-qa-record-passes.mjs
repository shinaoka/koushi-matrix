#!/usr/bin/env node
// Record a passing nightly run on each open `nightly-qa` issue it covers, and
// close an issue once its scenario has passed in PASSES_TO_CLOSE consecutive
// nightly runs (REPOSITORY_RULES "GitHub Issue Closure").
//
//   node scripts/nightly-qa-record-passes.mjs --server=tuwunel \
//     --failures=failures.json --run-url=<url>        # focused job
//   node scripts/nightly-qa-record-passes.mjs --server=tuwunel \
//     --scenario=aggregate --run-url=<url>            # aggregate job, on success
//
// An issue's streak is read back from its own comments: the trailing run of
// workflow comments that carry PASS_MARKER. Any other workflow comment (the
// failure comment the same workflow posts on a recurrence) ends the streak, so
// one failure resets the count without a separate state store.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { NIGHTLY_SCENARIOS } from "./lib/headless-qa-lanes.mjs";

export const PASSES_TO_CLOSE = 3;
export const PASS_MARKER = "<!-- nightly-qa:pass -->";
const WORKFLOW_AUTHORS = new Set(["github-actions", "github-actions[bot]", "app/github-actions"]);

function optionValue(name) {
  const prefix = `${name}=`;
  return process.argv.slice(2).find((arg) => arg.startsWith(prefix))?.slice(prefix.length);
}

// Consecutive passing runs recorded at the end of an issue's comment history.
// Comments by people are ignored: only the workflow's own records count.
export function passStreak(comments) {
  let streak = 0;
  for (const comment of [...comments].reverse()) {
    if (!WORKFLOW_AUTHORS.has(comment.author)) continue;
    if (!comment.body.includes(PASS_MARKER)) break;
    streak += 1;
  }
  return streak;
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

function recordPass(server, scenario, runUrl) {
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
    const streak = passStreak(history) + 1;
    const body =
      `${PASS_MARKER}\nThe nightly \`${scenario}\` scenario passed on \`${server}\` ` +
      `(${streak} of ${PASSES_TO_CLOSE} consecutive passing runs): ${runUrl}`;
    gh(["issue", "comment", String(number), "--body", body]);
    if (streak >= PASSES_TO_CLOSE) {
      gh(["issue", "close", String(number), "--reason", "completed", "--comment",
        `Closing: \`${scenario}\` passed on \`${server}\` in ${PASSES_TO_CLOSE} consecutive ` +
        "nightly runs. A recurrence reopens this issue."]);
    }
    console.log(`nightly-qa-record-passes: #${number} ${signature} streak=${streak}`);
  }
}

function main() {
  const server = optionValue("--server");
  const runUrl = optionValue("--run-url");
  const scenario = optionValue("--scenario");
  const failuresPath = optionValue("--failures");
  if (!["tuwunel", "synapse"].includes(server) || runUrl === undefined
    || (scenario === undefined) === (failuresPath === undefined)) {
    console.error(
      "usage: nightly-qa-record-passes.mjs --server=tuwunel|synapse --run-url=<url> " +
        "(--scenario=aggregate | --failures=<failures.json>)"
    );
    process.exit(2);
  }
  const scenarios = scenario !== undefined
    ? [scenario]
    : passedScenarios(existsSync(failuresPath)
      ? JSON.parse(readFileSync(failuresPath, "utf8"))
      : undefined);
  for (const passed of scenarios) recordPass(server, passed, runUrl);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
