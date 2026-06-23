#!/usr/bin/env node
/**
 * Real homeserver QA runner (Phase 8 — Milestone G).
 *
 * Runs the `real-homeserver-qa` Rust binary against a real homeserver
 * (matrix.org) using pre-approved credentials stored in
 * `.local-secrets/real-account-qa/credentials.json` (git-ignored, mode 600).
 *
 * ## Secrets protocol
 *
 * - The credentials file is READ BY THE BINARY, not by this script.
 * - This script passes the FILE PATH via env, never the credentials themselves.
 * - All output is captured to a per-run log file under
 *   `.local-secrets/real-account-qa/runs/<ts>/qa.log` (git-ignored).
 *   For `startup_latency`, the persistent profile dir is used instead:
 *   `.local-secrets/real-account-qa/profile/startup_latency/`.
 * - This script repeats the password/recovery_key leak self-check on the
 *   captured output (defence in depth; the binary already self-checks too).
 * - Never log, echo, or print the credentials file content.
 * - ABSOLUTE PROHIBITION: no GUI launch.
 *
 * ## Rate limits (matrix.org)
 *
 * - Single login per run. Logout cleanup runs even on failure.
 * - Never loop login/logout cycles.
 * - The `startup_latency` scenario performs run 1 (login + cache populate)
 *   then run 2 (cold restore + timing measurement) against the same
 *   persistent profile dir. This consumes ONE initial login device; run 2+
 *   restores from the SQLite store without a new login.
 *
 * ## Usage
 *
 *   node scripts/desktop-real-homeserver-qa.mjs --run [--scenario=compat|space_compat|all|startup_latency]
 *   npm --prefix apps/desktop run qa:real-homeserver
 */

import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  assertNoLocalPaths,
  assertNoMatrixIdentifiers,
  assertNoRawSdkErrors,
  assertRequiredTokens
} from "./lib/qa-token-contract.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const realAccountDir = join(repoRoot, ".local-secrets", "real-account-qa");
const credentialsPath = join(realAccountDir, "credentials.json");
const scenarioOption = optionValue("--scenario") ?? "space_compat";

const args = process.argv.slice(2);
if (args.includes("--run")) {
  try {
    await run();
    process.exit(0);
  } catch (error) {
    console.error(`real-homeserver-qa failed: ${error.message}`);
    process.exit(1);
  }
} else {
  printUsage();
}

async function run() {
  if (scenarioOption === "startup_latency") {
    await runStartupLatency();
    return;
  }
  await runSinglePass();
}

/**
 * Standard single-pass runner for all scenarios except `startup_latency`.
 * Uses a fresh per-run dir so no state bleeds across runs.
 */
async function runSinglePass() {
  const ts = new Date().toISOString().replace(/[:.]/g, "-");
  const runDir = join(realAccountDir, "runs", ts);
  const dataDir = join(runDir, "data");
  const credStoreDir = join(runDir, "cred-store");
  const logPath = join(runDir, "qa.log");
  mkdirSync(dataDir, { recursive: true });
  mkdirSync(credStoreDir, { recursive: true });

  console.log(`real-homeserver-qa: run=${ts}`);
  console.log("real-homeserver-qa: running binary (output captured to log)...");

  const env = {
    ...minimalEnvironment(),
    // Path to the credentials file — not a secret itself.
    KOUSHI_REAL_QA_CREDENTIALS_PATH: credentialsPath,
    KOUSHI_REAL_QA_SCENARIO: scenarioOption,
    // File-dir credential store backend: prevents OS keychain prompts.
    KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR: credStoreDir,
    // Fresh per-run data dir (SQLite store, media cache, etc.).
    KOUSHI_QA_DATA_DIR: dataDir
  };

  const result = spawnSync(
    "cargo",
    [
      "run",
      "--quiet",
      "-p",
      "koushi-core",
      "--features",
      "qa-bin",
      "--bin",
      "real-homeserver-qa"
    ],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env,
      // 15-minute wall-clock timeout (matrix.org can be slow; multiple
      // round-trips for login, sync, sends, edits, restore, logout).
      timeout: 15 * 60 * 1000,
      maxBuffer: 10 * 1024 * 1024
    }
  );

  const logParts = [];
  if (result.stdout) {
    logParts.push("[real-homeserver-qa stdout]\n", result.stdout, "\n");
  }
  if (result.stderr) {
    logParts.push("[real-homeserver-qa stderr]\n", result.stderr, "\n");
  }

  // Script-side redaction check (defence in depth): load the credentials
  // file and verify neither password nor recovery_key appears in the output.
  // We do this before writing artifacts or checking the exit code so a failed
  // run cannot persist private output.
  checkForLeaks(result.stdout, result.stderr, credentialsPath);

  // Matrix identifiers must never appear in real QA output — enforce this even
  // on failure, since a failed run can still leak an id into captured output.
  const combinedOutput = `${result.stdout || ""}\n${result.stderr || ""}`;
  assertNoMatrixIdentifiers(combinedOutput, "real-homeserver-qa");
  assertNoLocalPaths(combinedOutput, "real-homeserver-qa");
  assertNoRawSdkErrors(combinedOutput, "real-homeserver-qa");

  writeFileSync(logPath, logParts.join(""), "utf8");

  // Now check exit code.
  if (result.status !== 0) {
    throw new Error(
      `real-homeserver-qa binary exited with status ${result.status ?? "unknown"}; child output omitted after private-data validation`
    );
  }

  // Enforce scenario-specific success tokens (not just exit code): a clean exit
  // must still prove every documented checkpoint was reached.
  assertRequiredTokens(
    combinedOutput,
    requiredTokensForScenario(scenarioOption),
    "real-homeserver-qa"
  );

  // Print the binary's summary line (last non-empty stdout line).
  const summaryLine = (result.stdout || "")
    .trim()
    .split("\n")
    .filter((l) => l.trim())
    .at(-1);
  if (summaryLine) {
    console.log(`real-homeserver-qa: ${summaryLine}`);
  }
  console.log("real-homeserver-qa: PASSED");
}

/**
 * Two-run orchestrator for `--scenario=startup_latency`.
 *
 * Uses a PERSISTENT profile dir so the event cache and SQLite store
 * accumulate across runs:
 *   .local-secrets/real-account-qa/profile/startup_latency/data/
 *   .local-secrets/real-account-qa/profile/startup_latency/cred-store/
 *
 * Run 1 (populate): first invocation logs in and seeds the local event cache.
 * Run 2 (measure):  cold restore from the populated SQLite store; this is the
 *                   evidence run whose timing tokens are asserted.
 *
 * Both runs are subject to the full redaction / matrix-id / local-path /
 * SDK-error checks before any artifact is written. Run 2 must contain
 * `startup_lat restore=session` — if it shows `restore=not_found` the
 * persistent profile did not carry the session and the run is a failure.
 *
 * Rate-limit note: run 1 performs ONE real login; run 2 restores and does
 * NOT log in again. This consumes one device slot on the homeserver for the
 * lifetime of the profile (until `KOUSHI_STARTUP_LAT_TEARDOWN=1` is used).
 */
async function runStartupLatency() {
  const profileDir = join(realAccountDir, "profile", "startup_latency");
  const dataDir = join(profileDir, "data");
  const credStoreDir = join(profileDir, "cred-store");
  const logDir = join(profileDir, "logs");
  mkdirSync(dataDir, { recursive: true });
  mkdirSync(credStoreDir, { recursive: true });
  mkdirSync(logDir, { recursive: true });

  const baseEnv = {
    ...minimalEnvironment(),
    KOUSHI_REAL_QA_CREDENTIALS_PATH: credentialsPath,
    KOUSHI_REAL_QA_SCENARIO: "startup_latency",
    KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR: credStoreDir,
    KOUSHI_QA_DATA_DIR: dataDir,
    // Enable the env-gated origin observer and sub-phase timing.
    KOUSHI_STARTUP_TRACE: "1"
  };

  const cargoArgs = [
    "run",
    "--quiet",
    "-p",
    "koushi-core",
    "--features",
    "qa-bin",
    "--bin",
    "real-homeserver-qa"
  ];

  // -------------------------------------------------------------------------
  // Run 1: populate (first login + event-cache seed)
  // -------------------------------------------------------------------------
  const ts1 = new Date().toISOString().replace(/[:.]/g, "-");
  console.log(`real-homeserver-qa startup_latency: run1=${ts1} (populate)`);
  console.log("real-homeserver-qa startup_latency: run 1 may perform a real login (one device)");

  const result1 = spawnSync("cargo", cargoArgs, {
    cwd: repoRoot,
    encoding: "utf8",
    env: baseEnv,
    timeout: 15 * 60 * 1000,
    maxBuffer: 10 * 1024 * 1024
  });

  // Redaction checks on run 1 before any artifact write.
  checkForLeaks(result1.stdout, result1.stderr, credentialsPath);
  const run1Output = `${result1.stdout || ""}\n${result1.stderr || ""}`;
  assertNoMatrixIdentifiers(run1Output, "startup_latency run 1");
  assertNoLocalPaths(run1Output, "startup_latency run 1");
  assertNoRawSdkErrors(run1Output, "startup_latency run 1");

  const log1Parts = [];
  if (result1.stdout) log1Parts.push("[stdout]\n", result1.stdout, "\n");
  if (result1.stderr) log1Parts.push("[stderr]\n", result1.stderr, "\n");
  writeFileSync(join(logDir, `run1-${ts1}.log`), log1Parts.join(""), "utf8");

  if (result1.status !== 0) {
    throw new Error(
      `startup_latency run 1 (populate) exited with status ${result1.status ?? "unknown"}; ` +
        "child output omitted after private-data validation"
    );
  }
  console.log("real-homeserver-qa startup_latency: run 1 (populate) PASSED");

  // -------------------------------------------------------------------------
  // Run 2: cold restore + measure
  // -------------------------------------------------------------------------
  const ts2 = new Date().toISOString().replace(/[:.]/g, "-");
  console.log(`real-homeserver-qa startup_latency: run2=${ts2} (measure)`);

  const result2 = spawnSync("cargo", cargoArgs, {
    cwd: repoRoot,
    encoding: "utf8",
    env: baseEnv,
    timeout: 15 * 60 * 1000,
    maxBuffer: 10 * 1024 * 1024
  });

  // Redaction checks on run 2 before any artifact write.
  checkForLeaks(result2.stdout, result2.stderr, credentialsPath);
  const run2Output = `${result2.stdout || ""}\n${result2.stderr || ""}`;
  assertNoMatrixIdentifiers(run2Output, "startup_latency run 2");
  assertNoLocalPaths(run2Output, "startup_latency run 2");
  assertNoRawSdkErrors(run2Output, "startup_latency run 2");

  const log2Parts = [];
  if (result2.stdout) log2Parts.push("[stdout]\n", result2.stdout, "\n");
  if (result2.stderr) log2Parts.push("[stderr]\n", result2.stderr, "\n");
  writeFileSync(join(logDir, `run2-${ts2}.log`), log2Parts.join(""), "utf8");

  if (result2.status !== 0) {
    throw new Error(
      `startup_latency run 2 (measure) exited with status ${result2.status ?? "unknown"}; ` +
        "child output omitted after private-data validation"
    );
  }

  // -------------------------------------------------------------------------
  // Assert run 2 shows a cold RESTORE (not a fresh login).
  // If it shows restore=not_found the persistent profile didn't carry the
  // session — either the first run failed silently or the profile dir was
  // wiped between runs.
  // -------------------------------------------------------------------------
  if (run2Output.includes("startup_lat restore=not_found")) {
    throw new Error(
      "startup_latency run 2 shows restore=not_found instead of restore=session. " +
        "The persistent profile did not carry the session from run 1 " +
        "(see docs/qa/startup-latency-observability.md)."
    );
  }

  // -------------------------------------------------------------------------
  // Assert required timing tokens in run 2.
  // -------------------------------------------------------------------------
  const alwaysRequired = [
    "startup_lat phase=restore",
    "startup_lat phase=sync_to_ready",
    "startup_lat phase=room_list",
    "startup_latency=ok"
  ];
  for (const token of alwaysRequired) {
    if (!run2Output.includes(token)) {
      throw new Error(`startup_latency run 2 missing required token: ${token}`);
    }
  }

  // Subscribe/paginate/origin tokens are required unless the account has no
  // joined non-DM room (sparse account path emits subscribe=skipped).
  if (run2Output.includes("startup_lat subscribe=skipped")) {
    console.log(
      "real-homeserver-qa startup_latency: subscribe=skipped (sparse account — no joined non-DM room). " +
        "Skipping subscribe/paginate/origin token assertions."
    );
  } else {
    const subscribeRequired = [
      "startup_lat phase=subscribe",
      "startup_lat phase=paginate",
      "koushi.startup phase=origin"
    ];
    for (const token of subscribeRequired) {
      if (!run2Output.includes(token)) {
        throw new Error(
          `startup_latency run 2 missing required token: ${token} ` +
            "(expected when subscribe is not skipped)"
        );
      }
    }
  }

  // Print the binary's summary line (last non-empty stdout line from run 2).
  const summaryLine = (result2.stdout || "")
    .trim()
    .split("\n")
    .filter((l) => l.trim())
    .at(-1);
  if (summaryLine) {
    console.log(`real-homeserver-qa startup_latency: ${summaryLine}`);
  }
  console.log("real-homeserver-qa startup_latency: PASSED");
}

/**
 * Load the credentials JSON and check that neither the password nor the
 * recovery_key appears verbatim in stdout or stderr. Throws on leak.
 *
 * We read the credentials file here for the sole purpose of performing the
 * negative check. We do NOT print, log, or expose the values.
 */
function checkForLeaks(stdout, stderr, credPath) {
  let creds;
  try {
    creds = JSON.parse(readFileSync(credPath, "utf8"));
  } catch (e) {
    // If we cannot read the creds file, we cannot perform the check.
    // That is itself a problem, but not a leak — skip rather than error.
    void e;
    console.warn("real-homeserver-qa: WARNING: could not read credentials for leak check");
    return;
  }

  const combined = `${stdout || ""}\n${stderr || ""}`;
  for (const field of ["password", "recovery_key"]) {
    const value = creds[field];
    if (typeof value === "string" && value.length > 0 && combined.includes(value)) {
      throw new Error(
        `REDACTION FAILURE: '${field}' appears in QA output. ` +
          "This is a secrets leak. Do NOT share the QA output."
      );
    }
  }
}

function minimalEnvironment() {
  const env = {};
  for (const key of [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTC_WRAPPER",
    "RUSTFLAGS",
    "TERM"
  ]) {
    if (process.env[key] !== undefined) {
      env[key] = process.env[key];
    }
  }
  return env;
}

function optionValue(prefix) {
  const arg = process.argv.slice(2).find((argument) => argument.startsWith(`${prefix}=`));
  return arg?.slice(prefix.length + 1);
}

/**
 * Required success tokens per scenario. The base set covers login through
 * logout; space scenarios additionally require the real-space create/link/
 * cleanup tokens. These mirror docs/qa/headless-basic-operations.md.
 */
function requiredTokensForScenario(scenario) {
  const base = [
    "login=ok",
    "sync=running",
    "qa_room=created",
    "send_msg1=ok",
    "send_search=ok",
    "send_msg2=ok",
    "real_reply=ok",
    "edit_msg1=ok",
    "redact_msg2=ok",
    "search=ok",
    "store_restore=ok",
    "leave_room=ok",
    "forget_room=ok",
    "logout=ok",
    "post_logout_restore=not_found"
  ];
  if (scenario === "space_compat" || scenario === "all") {
    return [...base, "real_space_create=ok", "real_space_child=ok", "real_space_cleanup=ok"];
  }
  return base;
}

function printUsage() {
  console.log(
    "Usage: node scripts/desktop-real-homeserver-qa.mjs --run [--scenario=compat|space_compat|all|startup_latency]"
  );
  console.log(
    "Runs the real-homeserver-qa binary against a real Matrix homeserver."
  );
  console.log(
    "Scenario defaults to space_compat (full cleanup-proving lane); compat is a reduced debug subset."
  );
  console.log(
    "startup_latency: read-only two-run persistent-profile timing lane (see docs/qa/startup-latency-observability.md)."
  );
  console.log(
    "Requires: .local-secrets/real-account-qa/credentials.json (git-ignored, mode 600)"
  );
  console.log(
    "  JSON keys: homeserver, user_id, password, recovery_key, device_display_name"
  );
  console.log(
    "Output: .local-secrets/real-account-qa/runs/<ts>/qa.log (standard scenarios),"
  );
  console.log(
    "        .local-secrets/real-account-qa/profile/startup_latency/logs/ (startup_latency)"
  );
}
