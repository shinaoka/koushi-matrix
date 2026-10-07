#!/usr/bin/env node
// Run every focused headless Core QA scenario of one lane against one server.
// Each scenario runs in its own disposable homeserver; a failure is recorded
// and the remaining scenarios still run. Exit status is 1 when any failed.
//
//   node scripts/headless-qa-lane.mjs --lane=nightly --server=tuwunel \
//     --cargo-profile=ci --failures-out=failures.json

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  NIGHTLY_SCENARIOS,
  PR_CI_SCENARIOS,
  SCENARIO_TIMEOUT_MS
} from "./lib/headless-qa-lanes.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const qaOutputRoot = join(repoRoot, ".local-secrets", "headless-local-qa");

const lanes = { pr: PR_CI_SCENARIOS, nightly: NIGHTLY_SCENARIOS };

function optionValue(name) {
  const prefix = `${name}=`;
  return process.argv.slice(2).find((arg) => arg.startsWith(prefix))?.slice(prefix.length);
}

// The first clause of the core QA failure line, reduced to a stable
// signature: "Headless core QA failed: gate negative primary trust loss: ..."
// gives "gate negative primary trust loss".
export function failureStage(stderr) {
  const line = stderr
    .split("\n")
    .filter((entry) => entry.startsWith("Headless core QA failed: "))
    .at(-1);
  if (line === undefined) return "unknown";
  const words = line
    .slice("Headless core QA failed: ".length)
    .split(":")[0]
    .toLowerCase()
    .replace(/[^a-z0-9 _-]+/g, " ")
    .split(/\s+/)
    .filter((word) => word !== "");
  // Measured values ("held 1 closed 0", "observed_total=1") vary run to run;
  // drop each number and the counter name before it so a recurring failure
  // keeps one issue title instead of filing a new one every night.
  const kept = [];
  for (const word of words) {
    if (/\d/.test(word)) {
      if (kept.length > 0 && !/\d/.test(kept.at(-1))) kept.pop();
      continue;
    }
    kept.push(word);
  }
  const stage = kept.join(" ").slice(0, 80).trim();
  return stage === "" ? "unknown" : stage;
}

function latestRunDirectory(server, scenario) {
  if (!existsSync(qaOutputRoot)) return undefined;
  const suffix = `-${server}-${scenario}-`;
  return readdirSync(qaOutputRoot)
    .filter((name) => name.includes(suffix))
    .sort()
    .at(-1);
}

function stageForRun(server, scenario) {
  const directory = latestRunDirectory(server, scenario);
  if (directory === undefined) return "unknown";
  const stderrPath = join(qaOutputRoot, directory, "core-core-stderr.log");
  return existsSync(stderrPath) ? failureStage(readFileSync(stderrPath, "utf8")) : "unknown";
}

function main() {
  const lane = optionValue("--lane");
  const server = optionValue("--server");
  const cargoProfile = optionValue("--cargo-profile") ?? "dev";
  const failuresOut = optionValue("--failures-out");
  const scenarios = lanes[lane];
  if (scenarios === undefined || !["tuwunel", "synapse"].includes(server)) {
    console.error("usage: headless-qa-lane.mjs --lane=pr|nightly --server=tuwunel|synapse");
    process.exit(2);
  }

  const failures = [];
  for (const scenario of scenarios) {
    const started = Date.now();
    const result = spawnSync(
      process.execPath,
      [
        join(repoRoot, "scripts", "desktop-headless-local-qa.mjs"),
        "--run",
        `--server=${server}`,
        "--core",
        `--scenario=${scenario}`,
        `--cargo-profile=${cargoProfile}`,
        `--timeout-ms=${SCENARIO_TIMEOUT_MS}`
      ],
      { cwd: repoRoot, stdio: "inherit" }
    );
    const seconds = Math.round((Date.now() - started) / 1000);
    const status = result.status ?? 1;
    console.log(`headless-qa-lane: ${scenario} on ${server} EXIT=${status} (${seconds}s)`);
    if (status !== 0) failures.push({ scenario, stage: stageForRun(server, scenario) });
  }

  if (failuresOut !== undefined) writeFileSync(failuresOut, `${JSON.stringify(failures)}\n`);
  if (failures.length > 0) {
    console.error(
      `headless-qa-lane: ${failures.length} of ${scenarios.length} scenario(s) failed: ` +
        failures.map(({ scenario }) => scenario).join(", ")
    );
    process.exit(1);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
