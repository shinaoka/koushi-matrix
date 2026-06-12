#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { createWriteStream, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import net from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const localSecretsRoot = join(repoRoot, ".local-secrets", "headless-local-qa");
const checks = [
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
  } finally {
    await stopProcess(serverProcess);
  }
}

function startHomeserver(serverKind, configPath, logPath) {
  const log = createWriteStream(logPath, { flags: "a" });
  const child =
    serverKind === "conduit"
      ? spawn("conduit", [], {
          cwd: repoRoot,
          env: { ...minimalEnvironment(), CONDUIT_CONFIG: configPath },
          stdio: ["ignore", "pipe", "pipe"]
        })
      : spawn("tuwunel", ["--config", configPath], {
          cwd: repoRoot,
          env: minimalEnvironment(),
          stdio: ["ignore", "pipe", "pipe"]
        });
  child.stdout.on("data", (chunk) => log.write(chunk));
  child.stderr.on("data", (chunk) => log.write(chunk));
  child.once("exit", () => log.end());
  return child;
}

async function waitForHomeserver(homeserver, child, maxWaitMs, logPath) {
  const deadline = Date.now() + maxWaitMs;
  let lastError = "not attempted";
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`homeserver exited before readiness; see ${logPath}`);
    }
    try {
      const response = await fetch(`${homeserver}/_matrix/client/versions`);
      if (response.ok) {
        return;
      }
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = error.message;
    }
    await sleep(500);
  }
  throw new Error(`homeserver did not become ready: ${lastError}; see ${logPath}`);
}

async function registerUser(homeserver, username, password) {
  const body = {
    username,
    password,
    inhibit_login: false,
    auth: { type: "m.login.dummy" }
  };
  let response = await postJson(`${homeserver}/_matrix/client/v3/register`, body);
  if (response.status === 401 && response.json?.session) {
    response = await postJson(`${homeserver}/_matrix/client/v3/register`, {
      ...body,
      auth: { type: "m.login.dummy", session: response.json.session }
    });
  }
  if (response.status < 200 || response.status >= 300) {
    throw new Error(`register synthetic user ${username} failed with HTTP ${response.status}`);
  }
}

async function postJson(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body)
  });
  const text = await response.text();
  let json = null;
  if (text.trim()) {
    try {
      json = JSON.parse(text);
    } catch {
      json = null;
    }
  }
  return { status: response.status, json };
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
    ["run", "-p", "matrix-desktop-auth", "--features", "smoke", "--bin", "headless-local-qa"],
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

async function stopProcess(child) {
  if (child.exitCode !== null) {
    return;
  }
  const exited = new Promise((resolve) => child.once("exit", resolve));
  child.kill("SIGTERM");
  const stopped = await Promise.race([exited.then(() => true), sleep(5000).then(() => false)]);
  if (!stopped && child.exitCode === null) {
    child.kill("SIGKILL");
    await exited;
  }
}

function conduitConfig({ serverName, port, dataDir }) {
  return `[global]
server_name = ${tomlString(serverName)}
database_backend = "sqlite"
database_path = ${tomlString(dataDir)}
address = "127.0.0.1"
port = ${port}
max_request_size = 20000000
allow_registration = true
allow_federation = false
allow_room_creation = true
allow_check_for_updates = false
enable_lightning_bolt = false
trusted_servers = []
`;
}

function tuwunelConfig({ serverName, port, dataDir }) {
  return `[global]
server_name = ${tomlString(serverName)}
database_path = ${tomlString(dataDir)}
address = ["127.0.0.1"]
port = ${port}
new_user_displayname_suffix = ""
allow_registration = true
yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse = true
allow_federation = false
allow_room_creation = true
trusted_servers = []
`;
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

function checkInstalledHomeserver(name) {
  const result = spawnSync(name, ["--version"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: minimalEnvironment()
  });
  if (result.status !== 0) {
    throw new Error(`${name} is not installed or not runnable with --version`);
  }
}

async function freePort() {
  return new Promise((resolvePromise, rejectPromise) => {
    const server = net.createServer();
    server.once("error", rejectPromise);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close(() => resolvePromise(address.port));
    });
  });
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

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, "-");
}

function safeTimestamp() {
  return `${Date.now()}_${process.pid}`.replaceAll("-", "_");
}

function tomlString(value) {
  return JSON.stringify(value);
}

function sleep(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

function printUsage() {
  console.log("Usage: desktop-headless-local-qa.mjs --run [--server=conduit|tuwunel|both]");
  console.log("Starts a disposable local homeserver and runs non-GUI Matrix SDK QA.");
}
