#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { failureStage } from "./headless-qa-lane.mjs";
import { laneCoverageViolations, registeredScenarios } from "./lib/headless-qa-lanes.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const registryFixture = `
impl QaScenario {
    pub(super) fn from_env_value(value: &str) -> Result<Self, String> {
        match value {
            "all" => Ok(Self::All),
            "safety" => Ok(Self::Safety),
            "gate_no_proof" => Ok(Self::GateNoProof),
            "fresh_scenario" => Ok(Self::FreshScenario),
            other => Err(format!("got {other}")),
        }
    }

    pub(super) fn should_run_stage(self, stage: QaStage) -> bool {
        "not_a_scenario" => Ok(Self::Nope),
    }
}
`;

const lanes = {
  coveredByAll: ["safety"],
  prCi: ["gate_no_proof"],
  nightly: [],
  excluded: {}
};

test("parses only from_env_value scenario arms and skips all", () => {
  assert.deepEqual(registeredScenarios(registryFixture), [
    "safety",
    "gate_no_proof",
    "fresh_scenario"
  ]);
});

test("a registered scenario outside every lane is reported", () => {
  assert.deepEqual(laneCoverageViolations(registeredScenarios(registryFixture), lanes), [
    "scenario is in no QA lane and has no exclusion reason: fresh_scenario"
  ]);
});

test("a nightly entry or a reasoned exclusion covers the scenario", () => {
  const registered = registeredScenarios(registryFixture);
  assert.deepEqual(
    laneCoverageViolations(registered, { ...lanes, nightly: ["fresh_scenario"] }),
    []
  );
  assert.deepEqual(
    laneCoverageViolations(registered, {
      ...lanes,
      excluded: { fresh_scenario: "needs a 30 minute soak" }
    }),
    []
  );
});

test("empty exclusion reasons, stale names, and duplicates are rejected", () => {
  const registered = registeredScenarios(registryFixture);
  assert.deepEqual(
    laneCoverageViolations(registered, {
      ...lanes,
      nightly: ["retired_scenario", "safety", "safety"],
      excluded: { fresh_scenario: " " }
    }),
    [
      "nightly lists unregistered scenario: retired_scenario",
      "nightly lists a scenario twice",
      "excluded scenario has no reason: fresh_scenario"
    ]
  );
});

test("an automated scenario cannot also be excluded", () => {
  const registered = registeredScenarios(registryFixture);
  assert.deepEqual(
    laneCoverageViolations(registered, {
      ...lanes,
      nightly: ["fresh_scenario"],
      excluded: { safety: "duplicate" }
    }),
    ["scenario is both automated and excluded: safety"]
  );
});

test("the repository registry is fully covered", () => {
  const source = readFileSync(
    join(repoRoot, "crates/koushi-qa/src/bin/headless_core_qa/registry.rs"),
    "utf8"
  );
  const registered = registeredScenarios(source);
  assert.ok(registered.includes("gate_no_proof"));
  assert.deepEqual(laneCoverageViolations(registered), []);
});

test("failure stage is the first clause of the last core QA failure line", () => {
  assert.equal(
    failureStage(
      "[koushi] noise\nHeadless core QA failed: gate negative primary trust loss: timed out; phase=x\n"
    ),
    "gate negative primary trust loss"
  );
  assert.equal(failureStage("Headless core QA failed: no-proof rejection timed out\n"), "no-proof rejection timed out");
  assert.equal(failureStage("no failure line\n"), "unknown");
});
