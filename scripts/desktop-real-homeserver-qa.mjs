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
 * - This script repeats the password/recovery_key leak self-check on the
 *   captured output (defence in depth; the binary already self-checks too).
 * - Never log, echo, or print the credentials file content.
 * - ABSOLUTE PROHIBITION: no GUI launch.
 *
 * ## Rate limits (matrix.org)
 *
 * - Single login per run. Logout cleanup runs even on failure.
 * - Never loop login/logout cycles.
 *
 * ## Usage
 *
 *   node scripts/desktop-real-homeserver-qa.mjs --run
 *   npm --prefix apps/desktop run qa:real-homeserver
 */

import { spawnSync } from "node:child_process";
import { createWriteStream, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const realAccountDir = join(repoRoot, ".local-secrets", "real-account-qa");
const credentialsPath = join(realAccountDir, "credentials.json");

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
  // Per-run data dir: fresh every run so no state bleeds across runs.
  const ts = new Date().toISOString().replace(/[:.]/g, "-");
  const runDir = join(realAccountDir, "runs", ts);
  const dataDir = join(runDir, "data");
  const credStoreDir = join(runDir, "cred-store");
  const logPath = join(runDir, "qa.log");
  mkdirSync(dataDir, { recursive: true });
  mkdirSync(credStoreDir, { recursive: true });

  console.log(`real-homeserver-qa: run dir = ${runDir}`);
  console.log(`real-homeserver-qa: credentials file = ${credentialsPath}`);
  console.log("real-homeserver-qa: running binary (output captured to log)...");

  const env = {
    ...minimalEnvironment(),
    // Path to the credentials file — not a secret itself.
    MATRIX_DESKTOP_REAL_QA_CREDENTIALS_PATH: credentialsPath,
    // File-dir credential store backend: prevents OS keychain prompts.
    MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR: credStoreDir,
    // Fresh per-run data dir (SQLite store, media cache, etc.).
    MATRIX_DESKTOP_QA_DATA_DIR: dataDir
  };

  const result = spawnSync(
    "cargo",
    [
      "run",
      "-p",
      "matrix-desktop-core",
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

  // Write all output to the per-run log FIRST, before any checks.
  const log = createWriteStream(logPath, { flags: "a" });
  if (result.stdout) {
    log.write("[real-homeserver-qa stdout]\n");
    log.write(result.stdout);
    log.write("\n");
  }
  if (result.stderr) {
    log.write("[real-homeserver-qa stderr]\n");
    log.write(result.stderr);
    log.write("\n");
  }
  log.end();

  // Script-side redaction check (defence in depth): load the credentials
  // file and verify neither password nor recovery_key appears in the output.
  // We do this BEFORE checking the exit code so the leak is reported even
  // if the binary also failed for a different reason.
  checkForLeaks(result.stdout, result.stderr, credentialsPath, logPath);

  // Now check exit code.
  if (result.status !== 0) {
    const stdout = (result.stdout || "").trim();
    const stderr = (result.stderr || "").trim();
    throw new Error(
      `real-homeserver-qa binary exited with status ${result.status}\n` +
        `stdout: ${stdout || "<empty>"}\n` +
        `stderr: ${stderr || "<empty>"}\n` +
        `log: ${logPath}`
    );
  }

  // Print the binary's summary line (last non-empty stdout line).
  const summaryLine = (result.stdout || "")
    .trim()
    .split("\n")
    .filter((l) => l.trim())
    .at(-1);
  if (summaryLine) {
    console.log(`real-homeserver-qa: ${summaryLine}`);
  }
  console.log(`real-homeserver-qa: PASSED. Log: ${logPath}`);
}

/**
 * Load the credentials JSON and check that neither the password nor the
 * recovery_key appears verbatim in stdout or stderr. Throws on leak.
 *
 * We read the credentials file here for the sole purpose of performing the
 * negative check. We do NOT print, log, or expose the values.
 */
function checkForLeaks(stdout, stderr, credPath, logPath) {
  let creds;
  try {
    creds = JSON.parse(readFileSync(credPath, "utf8"));
  } catch (e) {
    // If we cannot read the creds file, we cannot perform the check.
    // That is itself a problem, but not a leak — skip rather than error.
    console.warn(
      `real-homeserver-qa: WARNING: could not read credentials for leak check: ${e.message}`
    );
    return;
  }

  const combined = `${stdout || ""}\n${stderr || ""}`;
  for (const field of ["password", "recovery_key"]) {
    const value = creds[field];
    if (typeof value === "string" && value.length > 0 && combined.includes(value)) {
      throw new Error(
        `REDACTION FAILURE: '${field}' appears in QA output. ` +
          `This is a secrets leak. Log: ${logPath}. ` +
          `Do NOT share the log file.`
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

function printUsage() {
  console.log(
    "Usage: node scripts/desktop-real-homeserver-qa.mjs --run"
  );
  console.log(
    "Runs the real-homeserver-qa binary against a real Matrix homeserver."
  );
  console.log(
    "Requires: .local-secrets/real-account-qa/credentials.json (git-ignored, mode 600)"
  );
  console.log(
    "  JSON keys: homeserver, user_id, password, recovery_key, device_display_name"
  );
  console.log(
    "Output is captured to .local-secrets/real-account-qa/runs/<ts>/qa.log"
  );
}
