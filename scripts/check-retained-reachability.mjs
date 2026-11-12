#!/usr/bin/env node
// Reachability guard for retained customizations (#1395).
//
// The retained-customization inventory in
// docs/upstream/matrix-rust-sdk-feedback.md is supposed to name each retained
// SDK customization together with its live product consumer. The cache-only
// anchor-restore surface showed that a recorded call site can silently become
// unreachable: the entry stayed, the consumer did not exist, and only a manual
// audit found it.
//
// This check makes the same rot fail CI:
//
//   1. Every entry in the audit manifest below must still have its recorded
//      consumer: the listed file must exist and still contain the surface
//      token. Removing the call site (the anchor-restore failure mode) or
//      moving the file fails the check.
//   2. Every command registered in the Tauri `generate_handler!` list must be
//      invoked from the renderer, or be recorded in the audit as a renderer-less
//      surface with a justification. A registered-but-unreachable command is the
//      exact shape that hid the dead anchor-restore command.
//   3. A removed surface must not reappear without a consumer and a fresh
//      verdict.
//
// Update this manifest in the same change that removes or replaces a consumer,
// and keep the verdicts in the inventory document in step.

import { readFileSync, existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const ROOT = process.cwd();

/** Retained-customization surfaces and the consumer that must still use them. */
const INVENTORY = [
  {
    entry: "ngram / CJK search",
    token: "encrypted_directory_ngram",
    paths: ["crates/koushi-sdk/src/search.rs"],
  },
  {
    entry: "Event cache: gap inspection",
    token: "inspect_room_timeline_gaps",
    paths: ["crates/koushi-sdk/src/timeline.rs"],
  },
  {
    entry: "Event cache: gap repair",
    token: "repair_room_timeline_gap",
    paths: ["crates/koushi-sdk/src/timeline.rs"],
  },
  {
    entry: "Event cache: live-tail refresh",
    token: "refresh_room_live_tail",
    paths: ["crates/koushi-sdk/src/timeline.rs"],
  },
  {
    entry: "Event cache: sync observation",
    token: "MatrixCommittedRoomTimelineCheckpoint",
    paths: ["crates/koushi-sdk/src/timeline.rs", "crates/koushi-core/src/timeline/manager.rs"],
  },
  {
    entry: "Redaction replay",
    token: "pending_redactions",
    paths: ["vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/mod.rs"],
  },
  {
    entry: "Room subscriptions: generation reconcile",
    token: "reconcile_room_subscriptions_with_generation",
    paths: ["crates/koushi-core/src/timeline/residency.rs"],
  },
  {
    entry: "Encryption sync readiness",
    token: "EncryptionSyncReadiness",
    paths: ["crates/koushi-core/src/sync.rs", "crates/koushi-sdk/src/sync.rs"],
  },
  {
    entry: "Timeline lazy reveal",
    token: "live_lazy_paginate_backwards_with_reveal",
    paths: ["vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/timeline/controller/mod.rs"],
  },
];

/**
 * Registered Tauri commands with no renderer caller, recorded as a verdict in
 * the inventory's reachability audit. Each is tracked for removal there.
 * `refresh_pinned_events` and `toggle_reaction` were the first two; #1409
 * removed them. Add an entry only together with an audit
 * verdict that names the removal.
 */
const RENDERER_LESS_COMMANDS = new Set([
  // #1871 Phase A ships the engine and IPC that move decryption keys out of
  // another local Matrix app. The settings surface that calls them is Phase B;
  // the verdict in docs/upstream/matrix-rust-sdk-feedback.md records that this
  // is deliberate and names the Phase B caller as the removal condition.
  "migration_detect_sources",
  "migration_import_element_desktop",
]);

/** Surfaces removed by #1395; they must not reappear in product crates. */
const REMOVED_SURFACES = [
  "run_backwards_cache_only",
  "live_restore_from_cache",
  "CacheOnlyBackOutcome",
  "RestoreFromCacheOutcome",
  "RestoreTimelineAnchor",
  "AnchorRestoreFinished",
];

function read(path) {
  return readFileSync(join(ROOT, path), "utf8");
}

function rendererSources() {
  const listed = execFileSync("git", ["ls-files", "apps/desktop/src"], {
    cwd: ROOT,
    encoding: "utf8",
  })
    .split("\n")
    .filter((file) => file.endsWith(".ts") || file.endsWith(".tsx"));
  return listed.map((file) => read(file)).join("\n");
}

function registeredCommands() {
  const source = read("apps/desktop/src-tauri/src/lib.rs");
  const match = source.match(/generate_handler!\[([\s\S]*?)\]/);
  if (!match) throw new Error("could not find the Tauri generate_handler! list");
  return match[1]
    .split(",")
    .map((entry) => entry.trim().split("::").pop())
    .filter(Boolean);
}

const violations = [];

for (const { entry, token, paths } of INVENTORY) {
  const existing = paths.filter((path) => existsSync(join(ROOT, path)));
  if (existing.length === 0) {
    violations.push(
      `inventory "${entry}": none of its recorded call sites exist (${paths.join(", ")})`,
    );
    continue;
  }
  if (!existing.some((path) => read(path).includes(token))) {
    violations.push(
      `inventory "${entry}": no recorded call site still contains "${token}" (${existing.join(", ")})`,
    );
  }
}

const renderer = rendererSources();
const commands = registeredCommands();
for (const command of commands) {
  if (renderer.includes(command) || RENDERER_LESS_COMMANDS.has(command)) continue;
  violations.push(
    `Tauri command "${command}" has no renderer caller and no recorded renderer-less verdict`,
  );
}

for (const surface of REMOVED_SURFACES) {
  let found = "";
  try {
    found = execFileSync("git", ["grep", "-l", surface, "--", "crates", "apps"], {
      cwd: ROOT,
      encoding: "utf8",
    });
  } catch (error) {
    // git grep exits 1 when nothing matches, which is the expected state.
    if (error.status !== 1) throw error;
  }
  const files = found.split("\n").filter(Boolean);
  if (files.length > 0) {
    violations.push(`removed surface "${surface}" is reachable again in ${files.join(", ")}`);
  }
}

if (violations.length > 0) {
  console.error("retained-customization reachability violations:");
  for (const violation of violations) console.error(`- ${violation}`);
  process.exit(1);
}

console.log(
  `check-retained-reachability: ok — ${INVENTORY.length} inventory surfaces, ${commands.length} Tauri commands, ${REMOVED_SURFACES.length} removed surfaces`,
);
