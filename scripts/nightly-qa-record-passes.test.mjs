#!/usr/bin/env node

import assert from "node:assert/strict";
import { test } from "node:test";

import { NIGHTLY_SCENARIOS } from "./lib/headless-qa-lanes.mjs";
import { PASS_MARKER, passStreak, passedScenarios } from "./nightly-qa-record-passes.mjs";

const pass = { author: "github-actions", body: `${PASS_MARKER}\npassed` };
const failure = { author: "github-actions", body: "The nightly scenario failed again" };
const person = { author: "maintainer", body: "Looking into it." };

test("an issue with no recorded passes has no streak", () => {
  assert.equal(passStreak([]), 0);
  assert.equal(passStreak([failure]), 0);
});

test("trailing workflow passes count, and a failure resets the streak", () => {
  assert.equal(passStreak([pass, pass]), 2);
  assert.equal(passStreak([pass, pass, failure, pass]), 1);
  assert.equal(passStreak([pass, failure]), 0);
});

test("comments by people neither count nor break the streak", () => {
  assert.equal(passStreak([pass, person, pass]), 2);
  assert.equal(passStreak([{ author: "maintainer", body: PASS_MARKER }]), 0);
});

test("the focused job passes every nightly scenario it did not record as failed", () => {
  const [failed, ...rest] = NIGHTLY_SCENARIOS;
  assert.deepEqual(passedScenarios([{ scenario: failed, stage: "unknown" }]), rest);
  assert.deepEqual(passedScenarios([]), [...NIGHTLY_SCENARIOS]);
});

test("a lane that wrote no failure record passed nothing", () => {
  assert.deepEqual(passedScenarios(undefined), []);
});
