#!/usr/bin/env node
// Fail when a headless Core QA scenario registered in registry.rs is covered
// by no automated lane (`all`, PR CI, nightly) and has no written exclusion.

import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { laneCoverageViolations, registeredScenarios } from "./lib/headless-qa-lanes.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const registryPath = join(
  repoRoot,
  "crates/koushi-qa/src/bin/headless_core_qa/registry.rs"
);

const registered = registeredScenarios(readFileSync(registryPath, "utf8"));
const violations = laneCoverageViolations(registered);
if (violations.length > 0) {
  for (const violation of violations) console.error(`headless QA lanes: ${violation}`);
  console.error("Update scripts/lib/headless-qa-lanes.mjs (see docs/agents/qa-lanes.md).");
  process.exit(1);
}
console.log(`headless QA lanes ok (${registered.length} scenarios)`);
