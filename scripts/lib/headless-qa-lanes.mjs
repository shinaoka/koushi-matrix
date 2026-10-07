// Single source for which automated lane runs each headless Core QA scenario.
// `scripts/check-headless-qa-lanes.mjs` fails when a scenario registered in
// `registry.rs` is in none of these lists, so a new scenario cannot be
// orphaned outside every lane again.

// Scenarios whose stages `--scenario=all` already runs (nightly aggregate).
export const COVERED_BY_ALL = Object.freeze([
  "safety",
  "login_sync",
  "session_status",
  "credential_health",
  "native_attention",
  "e2ee_trust",
  "invites_dm",
  "room_space",
  "directory",
  "room_management",
  "room_people_projection",
  "timeline",
  "activity",
  "composer",
  "reply",
  "media",
  "live_signals",
  "thread",
  "edit_redact_search",
  "redact_edit_convergence",
  "search_crawler",
  "room_history_export",
  "scheduled_send",
  "send_queue",
  "restore_cleanup",
  "link_preview"
]);

// Short focused scenarios run on every PR (core-invites job, both servers).
export const PR_CI_SCENARIOS = Object.freeze(["invites_dm", "gate_no_proof"]);

// Focused scenarios outside `all` that run nightly, one run per scenario, so
// one failure does not hide the others. `all` runs in its own nightly job.
export const NIGHTLY_SCENARIOS = Object.freeze([
  "gate_negative",
  "gate_restore",
  "device_cleanup",
  "e2ee_login_store",
  "cache_restore",
  "timeline_reconnect",
  "timeline_stress",
  "avatar_demand",
  "read_state_convergence",
  "hidden_state_acl",
  "search_crawler_catchup",
  "thread_late_joiner",
  "account_notifications",
  "user_verification"
]);

// Registered scenarios deliberately in no automated lane, with the reason.
export const EXCLUDED_SCENARIOS = Object.freeze({});

export const SCENARIO_TIMEOUT_MS = 600000;

const scenarioArm = /^\s*"([a-z0-9_]+)"\s*=>\s*Ok\(Self::[A-Za-z0-9]+\)/;

// Scenario names accepted by `QaScenario::from_env_value` in registry.rs.
export function registeredScenarios(registrySource) {
  const start = registrySource.indexOf("fn from_env_value");
  if (start < 0) throw new Error("registry.rs: QaScenario::from_env_value not found");
  const end = registrySource.indexOf("fn should_run_stage", start);
  const body = registrySource.slice(start, end < 0 ? undefined : end);
  return body
    .split("\n")
    .map((line) => scenarioArm.exec(line)?.[1])
    .filter((name) => name !== undefined && name !== "all");
}

export function laneCoverageViolations(
  registered,
  {
    coveredByAll = COVERED_BY_ALL,
    prCi = PR_CI_SCENARIOS,
    nightly = NIGHTLY_SCENARIOS,
    excluded = EXCLUDED_SCENARIOS
  } = {}
) {
  const violations = [];
  const known = new Set(registered);
  const lanes = { all: coveredByAll, "PR CI": prCi, nightly, excluded: Object.keys(excluded) };
  for (const [lane, names] of Object.entries(lanes)) {
    for (const name of names) {
      if (!known.has(name)) violations.push(`${lane} lists unregistered scenario: ${name}`);
    }
    if (new Set(names).size !== names.length) violations.push(`${lane} lists a scenario twice`);
  }
  for (const [name, reason] of Object.entries(excluded)) {
    if (typeof reason !== "string" || reason.trim() === "") {
      violations.push(`excluded scenario has no reason: ${name}`);
    }
  }
  for (const name of registered) {
    const automated = [coveredByAll, prCi, nightly].some((names) => names.includes(name));
    if (automated && Object.hasOwn(excluded, name)) {
      violations.push(`scenario is both automated and excluded: ${name}`);
    }
    if (!automated && !Object.hasOwn(excluded, name)) {
      violations.push(`scenario is in no QA lane and has no exclusion reason: ${name}`);
    }
  }
  return violations;
}
