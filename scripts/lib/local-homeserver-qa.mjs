#!/usr/bin/env node
import { chmodSync, createWriteStream, mkdirSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

/** @typedef {"tuwunel" | "synapse"} LocalHomeserverKind */

/**
 * @typedef {object} SynapseFixtureOptions
 * @property {boolean} [slidingSyncEnabled]
 */

/**
 * @typedef {LocalHomeserverKind | "both"}
 *   HomeserverSelection
 */

export function minimalEnvironment() {
  const env = {};
  for (const key of [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
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

export function checkInstalledHomeserver(name) {
  if (name === "synapse") {
    checkDockerAvailable();
    return;
  }

  const result = spawnSync(name, ["--version"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: minimalEnvironment(),
    stdio: "ignore"
  });
  if (result.status !== 0) {
    throw new Error(`${name} is not installed or not runnable with --version`);
  }
}

function checkDockerAvailable() {
  const version = spawnSync("docker", ["--version"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: minimalEnvironment(),
    stdio: "ignore"
  });
  if (version.status !== 0) {
    throw new Error("docker is not installed or not runnable with --version");
  }

  const info = spawnSync("docker", ["info"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: minimalEnvironment(),
    stdio: "ignore"
  });
  if (info.status !== 0) {
    throw new Error("docker daemon is not available for local Synapse QA");
  }
}

export function tuwunelConfig({ serverName, port, dataDir }) {
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

/**
 * @typedef {"intrinsic" | "synapse_experimental_feature" | "unsupported"}
 *   SimplifiedSlidingSyncConfiguration
 */

/**
 * @typedef {object} SimplifiedSlidingSyncCapability
 * @property {"org.matrix.simplified_msc3575"} unstableFeature
 * @property {boolean} enabled
 * @property {SimplifiedSlidingSyncConfiguration} configuration
 */

/**
 * @typedef {object} HomeserverFixtureCapabilities
 * @property {SimplifiedSlidingSyncCapability} simplifiedSlidingSync
 */

/**
 * @param {LocalHomeserverKind} serverKind
 * @param {SynapseFixtureOptions} [options]
 * @returns {HomeserverFixtureCapabilities}
 */
export function homeserverFixtureCapabilities(
  serverKind,
  { slidingSyncEnabled = true } = {}
) {
  /** @type {SimplifiedSlidingSyncConfiguration} */
  let configuration;
  let enabled;
  if (serverKind === "tuwunel") {
    configuration = "intrinsic";
    enabled = true;
  } else if (serverKind === "synapse") {
    configuration = "synapse_experimental_feature";
    enabled = slidingSyncEnabled;
  } else {
    throw new Error(`unknown local homeserver kind: ${serverKind}`);
  }

  return {
    simplifiedSlidingSync: {
      unstableFeature: "org.matrix.simplified_msc3575",
      enabled,
      configuration
    }
  };
}

/**
 * @param {HomeserverSelection} value Requested CLI homeserver selection.
 * @returns {LocalHomeserverKind[]} Concrete homeserver fixtures to run.
 */
export function selectedServers(value) {
  if (value === "both") {
    return ["tuwunel", "synapse"];
  }
  if (value === "tuwunel" || value === "synapse") {
    return [value];
  }
  throw new Error("--server must be tuwunel, synapse, or both");
}

export function startHomeserver(serverKind, configPath, logPath, options = {}) {
  const log = createWriteStream(logPath, { flags: "a" });
  const child = startHomeserverProcess(serverKind, configPath, options);
  child.stdout.on("data", (chunk) => log.write(chunk));
  child.stderr.on("data", (chunk) => log.write(chunk));
  child.once("exit", () => log.end());
  return child;
}

function startHomeserverProcess(serverKind, configPath, options) {
  if (serverKind === "tuwunel") {
    return spawn("tuwunel", ["--config", configPath], {
      cwd: repoRoot,
      env: minimalEnvironment(),
      stdio: ["ignore", "pipe", "pipe"]
    });
  }
  if (serverKind === "synapse") {
    return startSynapseHomeserver(configPath, options);
  }
  throw new Error(`unknown local homeserver kind: ${serverKind}`);
}

function startSynapseHomeserver(
  configPath,
  { serverName, port, dataDir, slidingSyncEnabled = true }
) {
  if (!serverName || !port || !dataDir) {
    throw new Error("Synapse local QA requires serverName, port, and dataDir");
  }
  checkDockerAvailable();

  const runDir = dirname(configPath);
  mkdirSync(dataDir, { recursive: true });
  const entrypointPath = resolve(runDir, "synapse-local-qa-start.sh");
  const dockerfilePath = resolve(runDir, "Dockerfile.synapse-local-qa");
  writeFileSync(entrypointPath, synapseEntrypoint({ slidingSyncEnabled }), { mode: 0o770 });
  chmodSync(entrypointPath, 0o770);
  writeFileSync(dockerfilePath, synapseDockerfile());

  const imageTag = `koushi-synapse-local-qa:${process.pid}-${Date.now()}`;
  const build = spawnSync(
    "docker",
    ["build", "-q", "-t", imageTag, "-f", dockerfilePath, runDir],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: minimalEnvironment(),
      maxBuffer: 10 * 1024 * 1024
    }
  );
  if (build.status !== 0) {
    throw new Error("local Synapse Docker image build failed");
  }

  const containerName = `koushi-synapse-local-qa-${process.pid}-${Date.now()}`;
  const child = spawn(
    "docker",
    [
      "run",
      "--rm",
      "--name",
      containerName,
      "-p",
      `127.0.0.1:${port}:8008`,
      "-v",
      `${dataDir}:/data`,
      "-e",
      `SYNAPSE_SERVER_NAME=${serverName}`,
      "-e",
      "SYNAPSE_REPORT_STATS=no",
      "-e",
      "SYNAPSE_HTTP_PORT=8008",
      "-e",
      "SYNAPSE_NO_TLS=1",
      imageTag
    ],
    {
      cwd: repoRoot,
      env: minimalEnvironment(),
      stdio: ["ignore", "pipe", "pipe"]
    }
  );
  child.koushiDockerContainerName = containerName;
  child.koushiDockerImageTag = imageTag;
  return child;
}

/** @returns {string} Dockerfile for the pinned local Synapse QA image. */
export function synapseDockerfile() {
  return `FROM docker.io/matrixdotorg/synapse:v1.157.0
COPY synapse-local-qa-start.sh /synapse-local-qa-start.sh
RUN chmod 770 /synapse-local-qa-start.sh
ENTRYPOINT ["/synapse-local-qa-start.sh"]
`;
}

/**
 * @param {SynapseFixtureOptions} [options] Requested Synapse fixture capabilities.
 * @returns {string} Entrypoint that creates and validates the requested fixture config.
 */
export function synapseEntrypoint({ slidingSyncEnabled = true } = {}) {
  return `#!/bin/bash
set -euo pipefail
export SYNAPSE_SERVER_NAME="\${SYNAPSE_SERVER_NAME:-localhost}"
export SYNAPSE_REPORT_STATS="\${SYNAPSE_REPORT_STATS:-no}"
if [ ! -f /data/homeserver.yaml ]; then
  /start.py migrate_config
  printf '\\n' >> /data/homeserver.yaml
  cat >> /data/homeserver.yaml <<'YAML'
enable_registration: true
enable_registration_without_verification: true
allow_public_rooms_without_auth: true
allow_public_rooms_over_federation: false
room_list_publication_rules:
  - action: allow
trusted_key_servers: []
rc_message:
  per_second: 1000
  burst_count: 1000
rc_room_creation:
  per_second: 1000
  burst_count: 1000
rc_registration:
  per_second: 1000
  burst_count: 1000
rc_login:
  address:
    per_second: 1000
    burst_count: 1000
  account:
    per_second: 1000
    burst_count: 1000
  failed_attempts:
    per_second: 1000
    burst_count: 1000
rc_admin_redaction:
  per_second: 1000
  burst_count: 1000
rc_joins:
  local:
    per_second: 1000
    burst_count: 1000
  remote:
    per_second: 1000
    burst_count: 1000
rc_invites:
  per_room:
    per_second: 1000
    burst_count: 1000
  per_user:
    per_second: 1000
    burst_count: 1000
  per_issuer:
    per_second: 1000
    burst_count: 1000
experimental_features:
  msc3266_enabled: true
YAML
fi
export KOUSHI_SYNAPSE_MSC3575_ENABLED=${slidingSyncEnabled ? "true" : "false"}
python3 - /data/homeserver.yaml <<'PYTHON'
import os
from pathlib import Path
import sys

import yaml

config_path = Path(sys.argv[1])
requested_value = os.environ["KOUSHI_SYNAPSE_MSC3575_ENABLED"]
if requested_value not in {"true", "false"}:
    raise RuntimeError("invalid simplified Sliding Sync fixture value")
expected = requested_value == "true"

with config_path.open("r", encoding="utf-8") as config_file:
    config = yaml.safe_load(config_file) or {}
if not isinstance(config, dict):
    raise RuntimeError("Synapse config root must be a mapping")
experimental_features = config.get("experimental_features")
if experimental_features is None:
    experimental_features = {}
elif not isinstance(experimental_features, dict):
    raise RuntimeError("Synapse experimental_features must be a mapping")
experimental_features["msc3575_enabled"] = expected
config["experimental_features"] = experimental_features

with config_path.open("w", encoding="utf-8") as config_file:
    yaml.safe_dump(config, config_file, sort_keys=False)
with config_path.open("r", encoding="utf-8") as config_file:
    loaded = yaml.safe_load(config_file) or {}
actual = loaded.get("experimental_features", {}).get("msc3575_enabled")
if type(actual) is not bool or actual != expected:
    raise RuntimeError("simplified Sliding Sync fixture value did not persist")
PYTHON
/start.py run
`;
}

export async function waitForHomeserver(homeserver, child, maxWaitMs, logPath) {
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

export async function registerUser(homeserver, username, password) {
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
  return response.json ?? {};
}

export async function createRoom(homeserver, accessToken, body = {}) {
  const response = await fetch(`${homeserver}/_matrix/client/v3/createRoom`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${accessToken}`,
      "content-type": "application/json"
    },
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
  if (!response.ok) {
    throw new Error(`createRoom failed with HTTP ${response.status}`);
  }
  return json ?? {};
}

export async function inviteUser(homeserver, accessToken, roomId, userId) {
  const response = await fetch(
    `${homeserver}/_matrix/client/v3/rooms/${encodeURIComponent(roomId)}/invite`,
    {
      method: "POST",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": "application/json"
      },
      body: JSON.stringify({ user_id: userId })
    }
  );
  if (!response.ok) {
    throw new Error(`inviteUser failed with HTTP ${response.status}`);
  }
}

export async function joinRoom(homeserver, accessToken, roomIdOrAlias) {
  const response = await fetch(
    `${homeserver}/_matrix/client/v3/join/${encodeURIComponent(roomIdOrAlias)}`,
    {
      method: "POST",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": "application/json"
      },
      body: JSON.stringify({})
    }
  );
  if (!response.ok) {
    throw new Error(`joinRoom failed with HTTP ${response.status}`);
  }
}

export async function sendReadMarkers(homeserver, accessToken, roomId, eventId) {
  const response = await fetch(
    `${homeserver}/_matrix/client/v3/rooms/${encodeURIComponent(roomId)}/read_markers`,
    {
      method: "POST",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": "application/json"
      },
      body: JSON.stringify({ fully_read: eventId, "m.read": eventId })
    }
  );
  if (!response.ok) {
    throw new Error(`sendReadMarkers failed with HTTP ${response.status}`);
  }
}

export async function sendRoomMessage(homeserver, accessToken, roomId, body, transactionId) {
  return sendRoomMessageContent(
    homeserver,
    accessToken,
    roomId,
    { msgtype: "m.text", body },
    transactionId
  );
}

export async function sendRoomFormattedMessage(
  homeserver,
  accessToken,
  roomId,
  body,
  formattedBody,
  transactionId
) {
  return sendRoomMessageContent(
    homeserver,
    accessToken,
    roomId,
    {
      msgtype: "m.text",
      body,
      format: "org.matrix.custom.html",
      formatted_body: formattedBody
    },
    transactionId
  );
}

export async function sendRoomNoticeMessage(homeserver, accessToken, roomId, body, transactionId) {
  return sendRoomMessageContent(
    homeserver,
    accessToken,
    roomId,
    { msgtype: "m.notice", body },
    transactionId
  );
}

export async function sendRoomEmoteMessage(homeserver, accessToken, roomId, body, transactionId) {
  return sendRoomMessageContent(
    homeserver,
    accessToken,
    roomId,
    { msgtype: "m.emote", body },
    transactionId
  );
}

async function sendRoomMessageContent(homeserver, accessToken, roomId, content, transactionId) {
  const path =
    `${homeserver}/_matrix/client/v3/rooms/${encodeURIComponent(roomId)}` +
    `/send/m.room.message/${encodeURIComponent(transactionId)}`;
  const response = await fetch(
    path,
    {
      method: "PUT",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": "application/json"
      },
      body: JSON.stringify(content)
    }
  );
  if (!response.ok) {
    throw new Error(`sendRoomMessage failed with HTTP ${response.status}`);
  }
  return response.json();
}

export async function setDisplayName(homeserver, accessToken, userId, displayName) {
  const response = await fetch(
    `${homeserver}/_matrix/client/v3/profile/${encodeURIComponent(userId)}/displayname`,
    {
      method: "PUT",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": "application/json"
      },
      body: JSON.stringify({ displayname: displayName })
    }
  );
  if (!response.ok) {
    throw new Error(`setDisplayName failed with HTTP ${response.status}`);
  }
}

export function freePort() {
  return new Promise((resolvePromise, rejectPromise) => {
    const server = net.createServer();
    server.once("error", rejectPromise);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close(() => rejectPromise(new Error("failed to acquire a free port")));
        return;
      }
      const { port } = address;
      server.close(() => resolvePromise(port));
    });
  });
}

export async function stopProcess(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    cleanupDockerHomeserver(child);
    return;
  }
  const exited = new Promise((resolve) => child.once("exit", resolve));
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {
    try {
      child.kill("SIGTERM");
    } catch {
      // ignore cleanup failures
    }
  }
  const settled = await Promise.race([exited.then(() => true), sleep(5000).then(() => false)]);
  if (!settled && child.exitCode === null && child.signalCode === null) {
    if (child.koushiDockerContainerName) {
      spawnSync("docker", ["stop", "--time", "5", child.koushiDockerContainerName], {
        cwd: repoRoot,
        encoding: "utf8",
        env: minimalEnvironment(),
        stdio: "ignore"
      });
    }
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {
      try {
        child.kill("SIGKILL");
      } catch {
        // ignore cleanup failures
      }
    }
    await exited;
  }
  cleanupDockerHomeserver(child);
}

function cleanupDockerHomeserver(child) {
  if (child?.koushiDockerImageTag) {
    spawnSync("docker", ["image", "rm", "-f", child.koushiDockerImageTag], {
      cwd: repoRoot,
      encoding: "utf8",
      env: minimalEnvironment(),
      stdio: "ignore"
    });
  }
}

function postJson(url, body) {
  return fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body)
  }).then(async (response) => {
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
  });
}

function tomlString(value) {
  return JSON.stringify(value);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
