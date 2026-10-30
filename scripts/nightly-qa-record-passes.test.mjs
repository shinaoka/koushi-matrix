#!/usr/bin/env node

import assert from "node:assert/strict";
import { test } from "node:test";

import { NIGHTLY_SCENARIOS } from "./lib/headless-qa-lanes.mjs";
import {
  passRecord,
  passStreak,
  passedScenarios,
  recordedRunIds
} from "./nightly-qa-record-passes.mjs";

const pass = (runId, fixLanded = true) => ({
  author: "github-actions",
  body: `${passRecord(runId, fixLanded)}\npassed`
});
const failure = { author: "github-actions", body: "The nightly scenario failed again" };
const person = { author: "maintainer", body: "Looking into it." };

test("an issue with no recorded passes has no streak", () => {
  assert.equal(passStreak([]).size, 0);
  assert.equal(passStreak([failure]).size, 0);
});

test("trailing workflow passes count, and a failure resets the streak", () => {
  assert.equal(passStreak([pass("1"), pass("2")]).size, 2);
  assert.equal(passStreak([pass("1"), pass("2"), failure, pass("3")]).size, 1);
  assert.equal(passStreak([pass("1"), failure]).size, 0);
});

test("a re-run of the same workflow run counts once", () => {
  assert.equal(passStreak([pass("1"), pass("1"), pass("2")]).size, 2);
  assert.deepEqual([...recordedRunIds([pass("1"), failure, pass("2")])].sort(), ["1", "2"]);
});

test("passes recorded before a fix landed do not count toward closure", () => {
  assert.equal(passStreak([pass("1", false), pass("2", false)]).size, 0);
  assert.equal(passStreak([pass("1", false), pass("2"), pass("3")]).size, 2);
});

test("comments by people neither count nor break the streak", () => {
  assert.equal(passStreak([pass("1"), person, pass("2")]).size, 2);
  assert.equal(passStreak([{ author: "maintainer", body: passRecord("9", true) }]).size, 0);
});

test("the Actions bot login is accepted in both forms", () => {
  assert.equal(passStreak([{ ...pass("1"), author: "github-actions[bot]" }]).size, 1);
});

test("the focused job passes every nightly scenario it did not record as failed", () => {
  const [failed, ...rest] = NIGHTLY_SCENARIOS;
  assert.deepEqual(passedScenarios([{ scenario: failed, stage: "unknown" }]), rest);
  assert.deepEqual(passedScenarios([]), [...NIGHTLY_SCENARIOS]);
});

test("a lane that wrote no failure record passed nothing", () => {
  assert.deepEqual(passedScenarios(undefined), []);
});
