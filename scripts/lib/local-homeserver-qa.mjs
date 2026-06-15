#!/usr/bin/env node
import { createWriteStream } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

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

export function conduitConfig({ serverName, port, dataDir }) {
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

export function startHomeserver(serverKind, configPath, logPath) {
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

export async function sendRoomMessage(homeserver, accessToken, roomId, body, transactionId) {
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
      body: JSON.stringify({ msgtype: "m.text", body })
    }
  );
  if (!response.ok) {
    throw new Error(`sendRoomMessage failed with HTTP ${response.status}`);
  }
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
