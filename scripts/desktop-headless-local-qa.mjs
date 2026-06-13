#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createWriteStream, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  checkInstalledHomeserver,
  conduitConfig,
  freePort,
  minimalEnvironment,
  registerUser,
  startHomeserver,
  stopProcess,
  tuwunelConfig,
  waitForHomeserver
} from "./lib/local-homeserver-qa.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const localSecretsRoot = join(repoRoot, ".local-secrets", "headless-local-qa");
const checks = [
  "scenario safety",
  "scenario login_sync",
  "scenario room_space",
  "scenario timeline",
  "scenario reply",
  "scenario thread",
  "scenario edit_redact_search",
  "scenario restore_cleanup",
  "verify installed Conduit binary",
  "verify installed Tuwunel binary",
  "start disposable local homeserver",
  "register two synthetic local users",
  "run headless Matrix SDK operations",
  "stop disposable local homeserver"
];

const args = new Set(process.argv.slice(2));
const serverOption = optionValue("--server") ?? "both";
const timeoutMs = Number(optionValue("--timeout-ms") ?? "90000");
const scenarioOption = optionValue("--scenario") ?? "all";
// --core: run the headless-core-qa binary in addition to (or instead of) the
// headless-local-qa binary. When this flag is present, both QA paths run for
// each server so both layers are exercised.
const runCoreQa = args.has("--core");

if (args.has("--list")) {
  for (const check of checks) {
    console.log(check);
  }
  process.exit(0);
}

if (args.has("--check-tools")) {
  checkInstalledHomeserver("conduit");
  checkInstalledHomeserver("tuwunel");
  console.log("headless local QA tools available");
  process.exit(0);
}

if (args.has("--print-conduit-config")) {
  console.log(
    conduitConfig({ serverName: "localhost:6167", port: 6167, dataDir: "/tmp/conduit-data" })
  );
  process.exit(0);
}

if (args.has("--print-tuwunel-config")) {
  console.log(
    tuwunelConfig({ serverName: "localhost:8008", port: 8008, dataDir: "/tmp/tuwunel-data" })
  );
  process.exit(0);
}

if (args.has("--run")) {
  try {
    await run();
    process.exit(0);
  } catch (error) {
    console.error(`headless local QA failed: ${error.message}`);
    process.exit(1);
  }
}

printUsage();

async function run() {
  const servers = selectedServers(serverOption);
  mkdirSync(localSecretsRoot, { recursive: true });

  for (const serverKind of servers) {
    await runForServer(serverKind);
  }
}

async function runForServer(serverKind) {
  checkInstalledHomeserver(serverKind);

  const port = await freePort();
  const serverName = `localhost:${port}`;
  const homeserver = `http://127.0.0.1:${port}`;
  const runDir = mkdtempSync(join(localSecretsRoot, `${timestamp()}-${serverKind}-`));
  const dataDir = join(runDir, "data");
  const logPath = join(runDir, "homeserver.log");
  mkdirSync(dataDir, { recursive: true });

  const configPath = join(runDir, `${serverKind}.toml`);
  writeFileSync(
    configPath,
    serverKind === "conduit"
      ? conduitConfig({ serverName, port, dataDir })
      : tuwunelConfig({ serverName, port, dataDir })
  );

  const serverProcess = startHomeserver(serverKind, configPath, logPath);
  try {
    await waitForHomeserver(homeserver, serverProcess, timeoutMs, logPath);

    const userSuffix = safeTimestamp();
    const userA = `qa_a_${userSuffix}`;
    const userB = `qa_b_${userSuffix}`;
    const passwordA = `matrix-desktop-local-a-${userSuffix}`;
    const passwordB = `matrix-desktop-local-b-${userSuffix}`;
    await registerUser(homeserver, userA, passwordA);
    await registerUser(homeserver, userB, passwordB);

    const qaResult = runHeadlessQa({
      serverKind,
      homeserver,
      serverName,
      userA,
      passwordA,
      userB,
      passwordB,
      logPath
    });
    console.log(qaResult.trim());

    if (runCoreQa) {
      // Leg 1: probed backend. Both local servers advertise MSC4186, so the
      // probe must select SyncService; the expectation makes drift fail QA.
      const coreQaResult = runCoreHeadlessQa({
        serverKind,
        homeserver,
        serverName,
        userA,
        passwordA,
        userB,
        passwordB,
        logPath,
        legLabel: "probed",
        expectSyncBackend: "SyncService"
      });
      console.log(`core QA (probed SyncService): ${coreQaResult.trim()}`);

      // Leg 2: forced LegacySync (debug/test-only env override). Fresh data
      // dir + cred store dir so no store state leaks across legs. Legacy
      // /sync works against MSC4186-capable servers too, so this leg
      // exercises the LegacySync product path end-to-end.
      const coreLegacyResult = runCoreHeadlessQa({
        serverKind,
        homeserver,
        serverName,
        userA,
        passwordA,
        userB,
        passwordB,
        logPath,
        legLabel: "legacy",
        forceLegacyBackend: true,
        expectSyncBackend: "LegacySync"
      });
      console.log(`core QA (forced LegacySync): ${coreLegacyResult.trim()}`);
      if (!coreLegacyResult.includes("sync_backend_a=LegacySync")) {
        throw new Error(
          "forced-legacy core QA leg did not report sync_backend_a=LegacySync"
        );
      }
    }
  } finally {
    await stopProcess(serverProcess);
  }
}

function runHeadlessQa({
  serverKind,
  homeserver,
  serverName,
  userA,
  passwordA,
  userB,
  passwordB,
  logPath
}) {
  const result = spawnSync(
    "cargo",
    ["run", "-p", "matrix-desktop-sdk", "--features", "smoke", "--bin", "headless-local-qa"],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...minimalEnvironment(),
        MATRIX_DESKTOP_LOCAL_QA_SERVER_KIND: serverKind,
        MATRIX_DESKTOP_LOCAL_QA_HOMESERVER: homeserver,
        MATRIX_DESKTOP_LOCAL_QA_SERVER_NAME: serverName,
        MATRIX_DESKTOP_LOCAL_QA_USER_A: userA,
        MATRIX_DESKTOP_LOCAL_QA_PASSWORD_A: passwordA,
        MATRIX_DESKTOP_LOCAL_QA_USER_B: userB,
        MATRIX_DESKTOP_LOCAL_QA_PASSWORD_B: passwordB
      },
      maxBuffer: 10 * 1024 * 1024
    }
  );
  appendQaOutput(logPath, result.stdout, result.stderr);
  if (result.status !== 0) {
    const stderr = result.stderr.trim();
    const stdout = result.stdout.trim();
    throw new Error(
      `headless SDK QA failed for ${serverKind}; stdout=${stdout || "<empty>"} stderr=${stderr || "<empty>"}; see ${logPath}`
    );
  }
  return result.stdout;
}

function runCoreHeadlessQa({
  serverKind,
  homeserver,
  serverName,
  userA,
  passwordA,
  userB,
  passwordB,
  logPath,
  legLabel = "default",
  forceLegacyBackend = false,
  expectSyncBackend
}) {
  // Per-leg dirs so backend legs never share SDK store or credential state.
  const runDataDir = join(logPath, "..", `core-qa-data-${legLabel}`);
  mkdirSync(runDataDir, { recursive: true });

  // The file credential store dir keeps QA unattended (no OS keychain prompts).
  const credStoreDir = join(logPath, "..", `core-qa-cred-${legLabel}`);
  mkdirSync(credStoreDir, { recursive: true });

  const env = {
    ...minimalEnvironment(),
    MATRIX_DESKTOP_LOCAL_QA_SERVER_KIND: serverKind,
    MATRIX_DESKTOP_LOCAL_QA_HOMESERVER: homeserver,
    MATRIX_DESKTOP_LOCAL_QA_SERVER_NAME: serverName,
    MATRIX_DESKTOP_LOCAL_QA_USER_A: userA,
    MATRIX_DESKTOP_LOCAL_QA_PASSWORD_A: passwordA,
    MATRIX_DESKTOP_LOCAL_QA_USER_B: userB,
    MATRIX_DESKTOP_LOCAL_QA_PASSWORD_B: passwordB,
    MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR: credStoreDir,
    MATRIX_DESKTOP_QA_DATA_DIR: runDataDir,
    MATRIX_DESKTOP_QA_SCENARIO: scenarioOption
  };
  if (forceLegacyBackend) {
    // Debug/test-only override; release builds ignore it entirely.
    env.MATRIX_DESKTOP_QA_FORCE_SYNC_BACKEND = "legacy";
  }
  if (expectSyncBackend) {
    env.MATRIX_DESKTOP_LOCAL_QA_EXPECT_SYNC_BACKEND = expectSyncBackend;
  }

  const result = spawnSync(
    "cargo",
    ["run", "-p", "matrix-desktop-core", "--features", "qa-bin", "--bin", "headless-core-qa"],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env,
      maxBuffer: 10 * 1024 * 1024
    }
  );
  appendQaOutput(logPath, result.stdout, result.stderr);
  // Secret redaction check: ensure passwords do not appear in QA output.
  for (const [label, secret] of [
    ["passwordA", passwordA],
    ["passwordB", passwordB]
  ]) {
    if (result.stdout.includes(secret) || result.stderr.includes(secret)) {
      throw new Error(
        `headless core QA output contains ${label} — secret redaction failure`
      );
    }
  }
  if (result.status !== 0) {
    const stderr = result.stderr.trim();
    const stdout = result.stdout.trim();
    throw new Error(
      `headless core QA (leg=${legLabel}) failed for ${serverKind}; stdout=${stdout || "<empty>"} stderr=${stderr || "<empty>"}; see ${logPath}`
    );
  }
  return result.stdout;
}

function appendQaOutput(logPath, stdout, stderr) {
  const log = createWriteStream(logPath, { flags: "a" });
  if (stdout) {
    log.write("\n[headless-local-qa stdout]\n");
    log.write(stdout);
  }
  if (stderr) {
    log.write("\n[headless-local-qa stderr]\n");
    log.write(stderr);
  }
  log.end();
}

function selectedServers(value) {
  if (value === "both") {
    return ["conduit", "tuwunel"];
  }
  if (value === "conduit" || value === "tuwunel") {
    return [value];
  }
  throw new Error("--server must be conduit, tuwunel, or both");
}

function optionValue(name) {
  const prefix = `${name}=`;
  for (const arg of process.argv.slice(2)) {
    if (arg.startsWith(prefix)) {
      return arg.slice(prefix.length);
    }
  }
  return undefined;
}

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, "-");
}

function safeTimestamp() {
  return `${Date.now()}_${process.pid}`.replaceAll("-", "_");
}

function printUsage() {
  console.log(
    "Usage: desktop-headless-local-qa.mjs --run [--server=conduit|tuwunel|both] [--scenario=all] [--core]"
  );
  console.log("Starts a disposable local homeserver and runs non-GUI Matrix SDK QA.");
  console.log("  --core  Also run the headless-core-qa binary (Phase 2+ core runtime QA).");
}
