#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import {
  cpSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync
} from "node:fs";
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
import {
  assertNoLocalPaths,
  assertNoMatrixIdentifiers,
  assertNoRawSdkErrors
} from "./lib/qa-token-contract.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const localSecretsRoot = join(repoRoot, ".local-secrets", "headless-local-qa");
const checks = [
  "scenario safety",
  "scenario login_sync",
  "scenario e2ee_trust",
  "scenario invites_dm",
  "scenario room_space",
  "scenario directory",
  "scenario room_management",
  "scenario timeline",
  "scenario timeline_stress",
  "scenario activity",
  "scenario composer",
  "scenario credential_health",
  "scenario native_attention",
  "scenario reply",
  "scenario link_preview",
  "scenario media",
  "scenario live_signals",
  "scenario thread",
  "scenario edit_redact_search",
  "scenario search_crawler",
  "scenario scheduled_send",
  "scenario send_queue",
  "scenario restore_cleanup",
  "verify installed Conduit binary",
  "verify installed Tuwunel binary",
  "verify local Synapse Docker runtime when --server=synapse",
  "start disposable local homeserver",
  "register synthetic local users",
  "run headless Matrix SDK operations",
  "stop disposable local homeserver"
];

const args = new Set(process.argv.slice(2));
const serverOption = optionValue("--server") ?? "both";
const timeoutMs = Number(optionValue("--timeout-ms") ?? "90000");
const scenarioOption = optionValue("--scenario") ?? "all";
const coreBackendOption =
  optionValue("--core-backend") ?? defaultCoreBackendForScenario(scenarioOption);
const fixtureRunOption = optionValue("--fixture-run");
const fixtureReplay = args.has("--fixture-replay") || fixtureRunOption !== undefined;
const e2eeRecipientSecondDeviceOption = args.has("--e2ee-recipient-second-device");
const e2eePauseSyncBeforeMultiDeviceSendOption = args.has(
  "--e2ee-pause-sync-before-multi-device-send"
);
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
  for (const serverKind of selectedServers(serverOption)) {
    checkInstalledHomeserver(serverKind);
  }
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
  if (
    (e2eeRecipientSecondDeviceOption || e2eePauseSyncBeforeMultiDeviceSendOption) &&
    !runCoreQa
  ) {
    throw new Error("E2EE multi-device options require --core");
  }
  if (scenarioOption === "timeline_stress" && !runCoreQa) {
    throw new Error("--scenario=timeline_stress requires --core because it validates Core state");
  }
  if (fixtureRunOption !== undefined) {
    if (scenarioOption !== "timeline_stress") {
      throw new Error("--fixture-run is currently supported only with --scenario=timeline_stress");
    }
    if (!runCoreQa) {
      throw new Error("--fixture-run requires --core");
    }
    if (coreBackendOption !== "probed") {
      throw new Error("--fixture-run requires --core-backend=probed");
    }
  }

  const servers = selectedServers(serverOption);
  if (fixtureRunOption !== undefined && (servers.length !== 1 || servers[0] !== "synapse")) {
    throw new Error("--fixture-run requires --server=synapse or --server=matrixorg");
  }
  mkdirSync(localSecretsRoot, { recursive: true });

  for (const serverKind of servers) {
    await runForServer(serverKind);
  }
}

async function runForServer(serverKind) {
  checkInstalledHomeserver(serverKind);

  const fixture = fixtureRunOption ? loadQaFixture(fixtureRunOption, serverKind) : null;
  const port = await freePort();
  const serverName = fixture?.serverName ?? `localhost:${port}`;
  const homeserver = `http://127.0.0.1:${port}`;
  const runDir = mkdtempSync(join(localSecretsRoot, `${timestamp()}-${serverKind}-`));
  const dataDir = join(runDir, "data");
  const logPath = join(runDir, "homeserver.log");
  if (fixture) {
    copyFixtureDataDir(fixture, dataDir);
  } else {
    mkdirSync(dataDir, { recursive: true });
  }

  const configPath = join(runDir, `${serverKind}.toml`);
  if (serverKind === "conduit" || serverKind === "tuwunel") {
    writeFileSync(
      configPath,
      serverKind === "conduit"
        ? conduitConfig({ serverName, port, dataDir })
        : tuwunelConfig({ serverName, port, dataDir })
    );
  } else if (serverKind === "synapse") {
    writeFileSync(configPath, "Synapse local QA is configured through Docker env.\n");
  }

  const serverProcess = startHomeserver(serverKind, configPath, logPath, {
    serverName,
    port,
    dataDir
  });
  try {
    await waitForHomeserver(homeserver, serverProcess, timeoutMs, logPath);

    if (scenarioOption !== "timeline_stress") {
      const sdkUsers = await registerQaUsers(homeserver, "sdk");

      const qaResult = runHeadlessQa({
        serverKind,
        homeserver,
        serverName,
        ...sdkUsers,
        logPath
      });
      console.log(qaResult.trim());
    } else {
      console.log("headless SDK QA skipped for core-only scenario timeline_stress");
    }

    if (runCoreQa) {
      // Leg 1: probed backend. Local homeservers that advertise MSC4186 should
      // run SyncService; the expectation makes drift fail QA.
      if (shouldRunCoreBackend("probed")) {
        const coreUsers = fixture ?? (await registerQaUsers(homeserver, "core_probed"));
        if (!fixture && serverKind === "synapse") {
          writeQaFixture(runDir, {
            serverKind,
            serverName,
            ...coreUsers
          });
        }
        const coreQaResult = runCoreHeadlessQa({
          serverKind,
          homeserver,
          serverName,
          ...coreUsers,
          logPath,
          legLabel: "probed",
          expectSyncBackend: "SyncService",
          replayExistingStress: fixtureReplay
        });
        console.log(`core QA (probed backend): ${coreQaResult.trim()}`);
      }

      // Leg 2: forced LegacySync (debug/test-only env override). Fresh data
      // dir + cred store dir so no store state leaks across legs. Legacy
      // /sync works against MSC4186-capable servers too, so this leg
      // exercises the LegacySync product path end-to-end.
      if (shouldRunCoreBackend("legacy")) {
        const coreUsers = await registerQaUsers(homeserver, "core_legacy");
        const coreLegacyResult = runCoreHeadlessQa({
          serverKind,
          homeserver,
          serverName,
          ...coreUsers,
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
    }
  } finally {
    await stopProcess(serverProcess);
  }
}

async function registerQaUsers(homeserver, label) {
  const userSuffix = `${label}_${safeTimestamp()}`;
  const userA = `qa_a_${userSuffix}`;
  const userB = `qa_b_${userSuffix}`;
  const userC = `qa_c_${userSuffix}`;
  const passwordA = `koushi-desktop-local-a-${userSuffix}`;
  const passwordB = `koushi-desktop-local-b-${userSuffix}`;
  const passwordC = `koushi-desktop-local-c-${userSuffix}`;
  await registerUser(homeserver, userA, passwordA);
  await registerUser(homeserver, userB, passwordB);
  await registerUser(homeserver, userC, passwordC);
  return { userA, passwordA, userB, passwordB, userC };
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
    [
      "run",
      "--quiet",
      "-p",
      "koushi-sdk",
      "--features",
      "smoke",
      "--bin",
      "headless-local-qa"
    ],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...minimalEnvironment(),
        KOUSHI_LOCAL_QA_SERVER_KIND: serverKind,
        KOUSHI_LOCAL_QA_HOMESERVER: homeserver,
        KOUSHI_LOCAL_QA_SERVER_NAME: serverName,
        KOUSHI_LOCAL_QA_USER_A: userA,
        KOUSHI_LOCAL_QA_PASSWORD_A: passwordA,
        KOUSHI_LOCAL_QA_USER_B: userB,
        KOUSHI_LOCAL_QA_PASSWORD_B: passwordB
      },
      maxBuffer: 10 * 1024 * 1024,
      timeout: timeoutMs
    }
  );
  writeQaOutputFiles(logPath, "sdk", result.stdout, result.stderr);
  assertQaOutputIsPrivate("headless SDK QA", result, [
    ["passwordA", passwordA],
    ["passwordB", passwordB]
  ]);
  appendQaOutput(logPath, result.stdout, result.stderr);
  if (result.error?.code === "ETIMEDOUT") {
    throw new Error(
      `headless SDK QA timed out for ${serverKind}; child output omitted after private-data validation`
    );
  }
  if (result.status !== 0) {
    throw new Error(
      `headless SDK QA failed for ${serverKind} with status ${result.status ?? "unknown"}; child output omitted after private-data validation`
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
  userC,
  logPath,
  legLabel = "default",
  forceLegacyBackend = false,
  expectSyncBackend,
  replayExistingStress = false
}) {
  // Per-leg dirs so backend legs never share SDK store or credential state.
  const runDataDir = join(logPath, "..", `core-qa-data-${legLabel}`);
  mkdirSync(runDataDir, { recursive: true });

  // The file credential store dir keeps QA unattended (no OS keychain prompts).
  const credStoreDir = join(logPath, "..", `core-qa-cred-${legLabel}`);
  mkdirSync(credStoreDir, { recursive: true });

  const env = {
    ...minimalEnvironment(),
    KOUSHI_LOCAL_QA_SERVER_KIND: serverKind,
    KOUSHI_LOCAL_QA_HOMESERVER: homeserver,
    KOUSHI_LOCAL_QA_SERVER_NAME: serverName,
    KOUSHI_LOCAL_QA_USER_A: userA,
    KOUSHI_LOCAL_QA_PASSWORD_A: passwordA,
    KOUSHI_LOCAL_QA_USER_B: userB,
    KOUSHI_LOCAL_QA_PASSWORD_B: passwordB,
    KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR: credStoreDir,
    KOUSHI_QA_DATA_DIR: runDataDir,
    KOUSHI_QA_SCENARIO: scenarioOption
  };
  if (userC) {
    env.KOUSHI_LOCAL_QA_USER_C = userC;
  }
  if (forceLegacyBackend) {
    // Debug/test-only override; release builds ignore it entirely.
    env.KOUSHI_QA_FORCE_SYNC_BACKEND = "legacy";
  }
  if (expectSyncBackend) {
    env.KOUSHI_LOCAL_QA_EXPECT_SYNC_BACKEND = expectSyncBackend;
  }
  for (const name of [
    "KOUSHI_QA_STRESS_SPACES",
    "KOUSHI_QA_STRESS_ROOMS_PER_SPACE",
    "KOUSHI_QA_STRESS_MESSAGES_PER_ROOM",
    "KOUSHI_QA_E2EE_RECIPIENT_SECOND_DEVICE",
    "KOUSHI_QA_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND",
    "KOUSHI_CORE_ACTOR_TRACE"
  ]) {
    if (process.env[name] !== undefined) {
      env[name] = process.env[name];
    }
  }
  if (e2eeRecipientSecondDeviceOption) {
    env.KOUSHI_QA_E2EE_RECIPIENT_SECOND_DEVICE = "true";
  }
  if (e2eePauseSyncBeforeMultiDeviceSendOption) {
    env.KOUSHI_QA_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND = "true";
  }
  if (process.env.KOUSHI_QA_RUST_LOG !== undefined) {
    env.RUST_LOG = process.env.KOUSHI_QA_RUST_LOG;
  }
  if (process.env.KOUSHI_QA_RUST_BACKTRACE !== undefined) {
    env.RUST_BACKTRACE = process.env.KOUSHI_QA_RUST_BACKTRACE;
  }
  if (replayExistingStress) {
    env.KOUSHI_QA_STRESS_REPLAY_EXISTING = "1";
  }

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
      "headless-core-qa"
    ],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env,
      maxBuffer: 10 * 1024 * 1024,
      timeout: timeoutMs
    }
  );
  writeQaOutputFiles(logPath, `core-${legLabel}`, result.stdout, result.stderr);
  assertQaOutputIsPrivate("headless core QA", result, [
    ["passwordA", passwordA],
    ["passwordB", passwordB]
  ]);
  appendQaOutput(logPath, result.stdout, result.stderr);
  if (result.status !== 0) {
    if (result.error?.code === "ETIMEDOUT") {
      throw new Error(
        `headless core QA (leg=${legLabel}) timed out for ${serverKind}; child output omitted after private-data validation`
      );
    }
    throw new Error(
      `headless core QA (leg=${legLabel}) failed for ${serverKind} with status ${result.status ?? "unknown"}; child output omitted after private-data validation`
    );
  }
  return result.stdout;
}

function assertQaOutputIsPrivate(label, result, secrets) {
  const stdout = result.stdout || "";
  const stderr = result.stderr || "";
  const output = `${stdout}\n${stderr}`;
  for (const [secretLabel, secret] of secrets) {
    if (secret && output.includes(secret)) {
      throw new Error(`${label}: ${secretLabel} leaked into QA output`);
    }
  }
  assertNoMatrixIdentifiers(output, label);
  assertNoLocalPaths(output, label);
  assertNoRawSdkErrors(output, label);
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

function writeQaOutputFiles(logPath, label, stdout, stderr) {
  const dir = dirname(logPath);
  writeFileSync(join(dir, `${label}-stdout.log`), stdout || "");
  writeFileSync(join(dir, `${label}-stderr.log`), stderr || "");
}

function writeQaFixture(runDir, fixture) {
  writeFileSync(
    join(runDir, "fixture.json"),
    `${JSON.stringify(
      {
        version: 1,
        serverKind: fixture.serverKind,
        serverName: fixture.serverName,
        userA: fixture.userA,
        passwordA: fixture.passwordA,
        userB: fixture.userB,
        passwordB: fixture.passwordB
      },
      null,
      2
    )}\n`
  );
}

function loadQaFixture(runName, serverKind) {
  if (!/^[A-Za-z0-9_.-]+$/.test(runName)) {
    throw new Error("--fixture-run must be a local headless QA run directory name");
  }
  const fixtureRunDir = join(localSecretsRoot, runName);
  const fixturePath = join(fixtureRunDir, "fixture.json");
  const dataDir = join(fixtureRunDir, "data");
  if (!existsSync(fixturePath)) {
    throw new Error("--fixture-run did not contain a fixture manifest");
  }
  if (!existsSync(dataDir) || !statSync(dataDir).isDirectory()) {
    throw new Error("--fixture-run did not contain a homeserver data directory");
  }
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8"));
  if (fixture.version !== 1) {
    throw new Error("--fixture-run has an unsupported fixture manifest version");
  }
  if (fixture.serverKind !== serverKind) {
    throw new Error("--fixture-run server kind does not match the selected homeserver");
  }
  for (const key of ["serverName", "userA", "passwordA", "userB", "passwordB"]) {
    if (typeof fixture[key] !== "string" || fixture[key].length === 0) {
      throw new Error("--fixture-run manifest is missing required synthetic account data");
    }
  }
  return {
    serverKind: fixture.serverKind,
    serverName: fixture.serverName,
    userA: fixture.userA,
    passwordA: fixture.passwordA,
    userB: fixture.userB,
    passwordB: fixture.passwordB,
    dataDir
  };
}

function copyFixtureDataDir(fixture, dataDir) {
  rmSync(dataDir, { recursive: true, force: true });
  cpSync(fixture.dataDir, dataDir, {
    recursive: true,
    force: true,
    errorOnExist: false
  });
}

function selectedServers(value) {
  if (value === "both") {
    return ["conduit", "tuwunel"];
  }
  if (value === "all") {
    return ["conduit", "tuwunel", "synapse"];
  }
  if (value === "conduit" || value === "tuwunel" || value === "synapse") {
    return [value];
  }
  if (value === "matrixorg") {
    return ["synapse"];
  }
  throw new Error("--server must be conduit, tuwunel, synapse, matrixorg, both, or all");
}

function defaultCoreBackendForScenario(value) {
  if (value === "all" || value === "e2ee_trust" || value === "timeline_stress") {
    return "probed";
  }
  return "both";
}

function shouldRunCoreBackend(backend) {
  if (coreBackendOption === "both") {
    return true;
  }
  if (coreBackendOption === "probed" || coreBackendOption === "legacy") {
    return coreBackendOption === backend;
  }
  throw new Error("--core-backend must be probed, legacy, or both");
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
    "Usage: desktop-headless-local-qa.mjs --run [--server=conduit|tuwunel|synapse|matrixorg|both|all] [--scenario=all|timeline_stress|directory|room_management|activity|composer|credential_health|native_attention|send_queue|live_signals|link_preview] [--core] [--core-backend=probed|legacy|both] [--fixture-run=<local-run-dir>] [--e2ee-recipient-second-device] [--e2ee-pause-sync-before-multi-device-send]"
  );
  console.log("Starts a disposable local homeserver and runs non-GUI Matrix SDK QA.");
  console.log("  --server=synapse/matrixorg  Runs local Synapse in Docker.");
  console.log("  --core  Also run the headless-core-qa binary (Phase 2+ core runtime QA).");
  console.log("  --core-backend  Select core backend leg. E2EE scenarios default to probed.");
  console.log("  --fixture-run  Replay a saved local Synapse fixture by copying its data dir.");
  console.log(
    "  --e2ee-recipient-second-device  Require encrypted sends to decrypt on the recipient's second verified device."
  );
  console.log(
    "  --e2ee-pause-sync-before-multi-device-send  Pause sync before the strict multi-device E2EE send for diagnostics."
  );
}
