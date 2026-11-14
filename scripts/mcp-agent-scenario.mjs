#!/usr/bin/env node
// Agent scenario: a coding agent drives a real Koushi through MCP.
//
// This is the end-to-end lane for #1526. It starts a disposable homeserver,
// registers a fresh account, seeds one room with a searchable message and a PNG
// attachment, then runs `headless-core-qa` in MCP host mode (KOUSHI_QA_MCP_HOST)
// so a real Koushi runtime owns the account. The agent talks to that runtime
// through the same `koushi-mcp` launcher a desktop installation uses, and the
// scenario passes only on server-side evidence:
//
//   1. a new message from the account that replies to the seeded image event,
//   2. the seeded PNG present in Koushi's media cache.
//
// The agent's prose is never the pass criterion. Every identifier is synthetic:
// the homeserver, the account, and the room are created for this run and
// deleted after it.
import { spawn, spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import {
  freePort,
  minimalEnvironment,
  registerUser,
  startHomeserver,
  stopProcess,
  tuwunelConfig,
  waitForHomeserver
} from "./lib/local-homeserver-qa.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const READY_TIMEOUT_MS = 240_000;
const MCP_LIST_TIMEOUT_MS = 90_000;
const REPLY_TIMEOUT_MS = 90_000;

/// 1x1 PNG. The scenario compares the cached bytes, so the fixture must be a
/// real image a homeserver will serve back unchanged.
const PNG_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64"
);

const ACCOUNT_PASSWORD = `agent-scenario-${randomBytes(12).toString("hex")}`;

function parseArgs(argv) {
  const options = {
    server: "tuwunel",
    hostBin: join(repoRoot, "target", "debug", "headless-core-qa"),
    mcpBin: join(repoRoot, "target", "debug", "koushi-mcp"),
    piBin: "pi",
    timeoutMs: 900_000,
    keep: false,
    debug: false
  };
  for (const arg of argv) {
    if (arg.startsWith("--server=")) {
      options.server = arg.slice("--server=".length);
    } else if (arg.startsWith("--host-bin=")) {
      options.hostBin = resolve(arg.slice("--host-bin=".length));
    } else if (arg.startsWith("--mcp-bin=")) {
      options.mcpBin = resolve(arg.slice("--mcp-bin=".length));
    } else if (arg.startsWith("--pi=")) {
      options.piBin = arg.slice("--pi=".length);
    } else if (arg.startsWith("--timeout-ms=")) {
      options.timeoutMs = Number(arg.slice("--timeout-ms=".length));
    } else if (arg === "--keep") {
      options.keep = true;
    } else if (arg === "--debug") {
      options.debug = true;
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  if (options.server !== "tuwunel") {
    throw new Error(`agent scenarios run against --server=tuwunel only; got ${options.server}`);
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error("--timeout-ms must be a positive number");
  }
  return options;
}

function sleep(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

let logStream = null;

function log(message) {
  const line = `[agent-scenario] ${message}`;
  console.log(line);
  logStream?.write(`${line}\n`);
}

async function getJson(url, accessToken) {
  const response = await fetch(url, {
    headers: accessToken ? { authorization: `Bearer ${accessToken}` } : {}
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
    throw new Error(`GET ${new URL(url).pathname} failed with HTTP ${response.status}`);
  }
  return json ?? {};
}

async function sendJson(method, url, accessToken, body) {
  const response = await fetch(url, {
    method,
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
    throw new Error(`${method} ${new URL(url).pathname} failed with HTTP ${response.status}`);
  }
  return json ?? {};
}

async function uploadPng(homeserver, accessToken) {
  const response = await fetch(`${homeserver}/_matrix/media/v3/upload?filename=agent-scenario.png`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${accessToken}`,
      "content-type": "image/png"
    },
    body: PNG_BYTES
  });
  if (!response.ok) {
    throw new Error(`media upload failed with HTTP ${response.status}`);
  }
  const body = await response.json();
  if (typeof body.content_uri !== "string") {
    throw new Error("media upload returned no content_uri");
  }
  return body.content_uri;
}

async function sendMessageEvent(homeserver, accessToken, roomId, content) {
  const transactionId = `agent-scenario-${randomBytes(8).toString("hex")}`;
  const url = `${homeserver}/_matrix/client/v3/rooms/${encodeURIComponent(roomId)}/send/m.room.message/${transactionId}`;
  const body = await sendJson("PUT", url, accessToken, content);
  if (typeof body.event_id !== "string") {
    throw new Error("send returned no event_id");
  }
  return body.event_id;
}

/// Waits for the MCP host's ready line and returns its payload.
function waitForReadyLine(child, deadline) {
  return new Promise((resolvePromise, rejectPromise) => {
    const timer = setTimeout(() => {
      rejectPromise(new Error("MCP host did not report readiness in time"));
    }, Math.max(0, deadline - Date.now()));
    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => {
      logStream?.write(`${line}\n`);
      const match = /^MCP_HOST_READY (\{.*\})$/u.exec(line);
      if (!match) {
        return;
      }
      clearTimeout(timer);
      lines.close();
      try {
        resolvePromise(JSON.parse(match[1]));
      } catch {
        rejectPromise(new Error("MCP host printed an unparsable ready line"));
      }
    });
    child.once("exit", (code, signal) => {
      clearTimeout(timer);
      rejectPromise(
        new Error(`MCP host exited before readiness (code=${code} signal=${signal}); see host log`)
      );
    });
  });
}

/// `pi mcp list --json` proves the bridge answers and the grant is live before
/// the model is asked to do anything, so a transport failure never looks like a
/// model failure.
async function waitForMcpTools(piBin, piEnv, cwd, expectedTools, deadline) {
  let lastOutput = "not attempted";
  const started = Date.now();
  while (Date.now() < deadline) {
    const result = spawnSync(piBin, ["mcp", "list", "--json"], {
      cwd,
      env: piEnv,
      encoding: "utf8",
      maxBuffer: 10 * 1024 * 1024
    });
    lastOutput = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
    if (result.status === 0) {
      const tools = collectMcpToolNames(lastOutput);
      if (expectedTools.every((tool) => tools.includes(tool))) {
        return tools;
      }
      lastOutput = `tools found: ${tools.join(", ") || "(none)"}`;
    }
    if (Date.now() - started > MCP_LIST_TIMEOUT_MS) {
      break;
    }
    await sleep(2000);
  }
  const safeOutput = lastOutput.split("\n").slice(-8).join("\n");
  throw new Error(`MCP server never exposed the Koushi tools; last output:\n${safeOutput}`);
}

/// `pi mcp list --json` reports one entry per configured server with the tool
/// names the server published.
function collectMcpToolNames(output) {
  let payload = null;
  try {
    payload = JSON.parse(output.slice(output.indexOf("{")));
  } catch {
    return [];
  }
  const servers = Array.isArray(payload?.servers) ? payload.servers : [];
  return servers.flatMap((server) =>
    Array.isArray(server?.tools) ? server.tools.filter((tool) => typeof tool === "string") : []
  );
}

async function waitForReply(homeserver, accessToken, roomId, targetEventId, deadline) {
  const url = `${homeserver}/_matrix/client/v3/rooms/${encodeURIComponent(roomId)}/messages?dir=b&limit=50`;
  while (Date.now() < deadline) {
    const body = await getJson(url, accessToken);
    const reply = (body.chunk ?? []).find(
      (event) =>
        event.type === "m.room.message" &&
        event.content?.["m.relates_to"]?.["m.in_reply_to"]?.event_id === targetEventId
    );
    if (reply) {
      return reply;
    }
    await sleep(2000);
  }
  return null;
}

function findCachedAttachment(accountRoot, expectedSha256, expectedSize) {
  const pending = [accountRoot];
  while (pending.length > 0) {
    const dir = pending.pop();
    let entries = [];
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        pending.push(path);
        continue;
      }
      if (!entry.isFile() || statSync(path).size !== expectedSize) {
        continue;
      }
      const hash = createHash("sha256").update(readFileSync(path)).digest("hex");
      if (hash === expectedSha256) {
        return path;
      }
    }
  }
  return null;
}

function runAgent(options, piEnv, cwd, prompt) {
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(
      options.piBin,
      [
        "-p",
        "--provider",
        "deepseek",
        "--model",
        "deepseek-flash",
        "--no-session",
        "--no-context-files",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-approve",
        prompt
      ],
      {
        cwd,
        env: piEnv,
        stdio: ["ignore", "pipe", "pipe"]
      }
    );
    const timer = setTimeout(() => {
      child.kill("SIGTERM");
      rejectPromise(new Error("the agent did not finish within the scenario timeout"));
    }, options.timeoutMs);
    child.stdout.on("data", (chunk) => logStream?.write(chunk));
    child.stderr.on("data", (chunk) => logStream?.write(chunk));
    child.once("exit", (code, signal) => {
      clearTimeout(timer);
      if (signal !== null) {
        rejectPromise(new Error(`the agent was stopped by ${signal}`));
        return;
      }
      resolvePromise(code);
    });
  });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  for (const [label, path] of [
    ["host binary", options.hostBin],
    ["koushi-mcp binary", options.mcpBin]
  ]) {
    if (!existsSync(path)) {
      throw new Error(
        `${label} is missing at ${path}; build it first with ` +
          "`cargo build -p koushi-qa --features qa-bin --bin headless-core-qa` and " +
          "`cargo build -p koushi-mcp --bin koushi-mcp`"
      );
    }
  }
  if (!process.env.DEEPSEEK_API_KEY) {
    throw new Error("DEEPSEEK_API_KEY is required; the agent model is deepseek-flash");
  }

  const stamp = new Date().toISOString().replace(/[:.]/gu, "-");
  const runRoot = join(repoRoot, ".local-secrets", "agent-scenarios", stamp);
  mkdirSync(runRoot, { recursive: true });
  logStream = createWriteStream(join(runRoot, "scenario.log"), { flags: "a" });
  const hostLog = createWriteStream(join(runRoot, "host.log"), { flags: "a" });

  const port = await freePort();
  const serverName = "agent-scenario.localhost";
  const homeserver = `http://127.0.0.1:${port}`;
  const dataDir = join(runRoot, "homeserver");
  // A Unix socket path is limited to ~104 bytes, so the bridge cannot live under
  // the (deep) run directory. Koushi publishes the socket inside this directory.
  const bridgeDir = mkdtempSync(join(tmpdir(), "koushi-mcp-"));
  const credentialDir = join(runRoot, "credentials");
  const qaDataDir = join(runRoot, "koushi-data");
  const piAgentDir = join(runRoot, "pi-agent");
  const piProjectDir = join(runRoot, "pi-project");
  for (const dir of [dataDir, credentialDir, qaDataDir, piAgentDir, piProjectDir]) {
    mkdirSync(dir, { recursive: true });
  }

  const configPath = join(runRoot, "tuwunel.toml");
  writeFileSync(configPath, tuwunelConfig({ serverName, port, dataDir }));

  let homeserverChild = null;
  let hostChild = null;
  let passed = false;
  try {
    log("starting a disposable homeserver");
    homeserverChild = startHomeserver("tuwunel", configPath, join(runRoot, "homeserver.log"));
    await waitForHomeserver(homeserver, homeserverChild, 60_000, join(runRoot, "homeserver.log"));

    const username = `agent${randomBytes(4).toString("hex")}`;
    const registration = await registerUser(homeserver, username, ACCOUNT_PASSWORD);
    const accessToken = registration.access_token;
    const userId = registration.user_id;
    if (!accessToken || !userId) {
      throw new Error("the disposable account did not register");
    }
    log("registering a fresh disposable account");

    const room = await sendJson(
      "POST",
      `${homeserver}/_matrix/client/v3/createRoom`,
      accessToken,
      { name: "Agent scenario", preset: "private_chat" }
    );
    const roomId = room.room_id;
    if (!roomId) {
      throw new Error("the disposable room was not created");
    }

    const token = `KOUSHI_AGENT_${randomBytes(6).toString("hex").toUpperCase()}`;
    const mediaUri = await uploadPng(homeserver, accessToken);
    const textEventId = await sendMessageEvent(homeserver, accessToken, roomId, {
      msgtype: "m.text",
      body: `${token}: an attachment follows in the next message.`
    });
    const imageEventId = await sendMessageEvent(homeserver, accessToken, roomId, {
      msgtype: "m.image",
      body: "agent-scenario.png",
      url: mediaUri,
      info: { mimetype: "image/png", size: PNG_BYTES.length, w: 1, h: 1 }
    });
    log(`seeding one room with a searchable message and one PNG (search event ${options.debug ? textEventId : "hidden"})`);

    const hostEnv = {
      ...minimalEnvironment(),
      KOUSHI_LOCAL_QA_HOMESERVER: homeserver,
      KOUSHI_LOCAL_QA_SERVER_NAME: serverName,
      KOUSHI_LOCAL_QA_USER_A: username,
      KOUSHI_LOCAL_QA_PASSWORD_A: ACCOUNT_PASSWORD,
      KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR: credentialDir,
      KOUSHI_QA_DATA_DIR: qaDataDir,
      KOUSHI_QA_MCP_HOST: "1",
      KOUSHI_QA_MCP_BRIDGE_DIR: bridgeDir,
      KOUSHI_QA_MCP_CLIENT: "pi"
    };
    log("starting the headless Koushi MCP host");
    hostChild = spawn(options.hostBin, [], {
      cwd: repoRoot,
      env: hostEnv,
      stdio: ["ignore", "pipe", "pipe"]
    });
    hostChild.stderr.pipe(hostLog);
    const ready = await waitForReadyLine(hostChild, Date.now() + READY_TIMEOUT_MS);
    if (typeof ready.endpoint !== "string" || typeof ready.account_id !== "string") {
      throw new Error("the MCP host ready line is incomplete");
    }

    const piEnv = {
      ...minimalEnvironment(),
      DEEPSEEK_API_KEY: process.env.DEEPSEEK_API_KEY,
      PI_CODING_AGENT_DIR: piAgentDir
    };
    const added = spawnSync(
      options.piBin,
      [
        "mcp",
        "add",
        "koushi",
        "--exposure",
        "direct",
        "--description",
        "Koushi desktop: search messages, send replies, fetch attachments",
        "--",
        options.mcpBin,
        "--endpoint",
        ready.endpoint,
        "--client-name",
        "pi"
      ],
      { cwd: piProjectDir, env: piEnv, encoding: "utf8" }
    );
    if (added.status !== 0) {
      throw new Error(`registering the MCP server with the agent failed: ${added.stderr?.trim()}`);
    }

    const expectedTools = [
      "list_accounts",
      "list_rooms",
      "list_spaces",
      "list_space_rooms",
      "read_room_messages",
      "read_recent_activity",
      "search_messages",
      "send_message",
      "wait_for_events",
      "get_attachment"
    ];
    const tools = await waitForMcpTools(
      options.piBin,
      piEnv,
      piProjectDir,
      expectedTools,
      Date.now() + MCP_LIST_TIMEOUT_MS
    );
    log(`the agent sees ${tools.length} Koushi tools over MCP`);

    const prompt = [
      "You are testing Koushi, a Matrix client, through its MCP tools only.",
      `Use account_id "${ready.account_id}".`,
      `1. Call search_messages with query "${token}" and find the message whose body is "agent-scenario.png".`,
      "2. Call get_attachment for that message so Koushi downloads the image.",
      "3. Call send_message with reply_to_event_id set to that image message's event_id and body \"添付を確認しました\".",
      "Do not invent ids: use the values the tools return. When you are done, print one line summarising which tools you called."
    ].join("\n");
    log("running the agent");
    const agentExitCode = await runAgent(options, piEnv, piProjectDir, prompt);
    if (agentExitCode !== 0) {
      log(`the agent exited with code ${agentExitCode}; checking server-side evidence anyway`);
    }

    const reply = await waitForReply(
      homeserver,
      accessToken,
      roomId,
      imageEventId,
      Date.now() + REPLY_TIMEOUT_MS
    );
    if (!reply) {
      throw new Error("no message from the account replies to the seeded image event");
    }

    const expectedSha256 = createHash("sha256").update(PNG_BYTES).digest("hex");
    const accountRoot = join(qaDataDir, "mcp-host", "accounts");
    let cached = null;
    const cacheDeadline = Date.now() + REPLY_TIMEOUT_MS;
    while (Date.now() < cacheDeadline) {
      cached = findCachedAttachment(accountRoot, expectedSha256, PNG_BYTES.length);
      if (cached !== null) {
        break;
      }
      await sleep(2000);
    }
    if (cached === null) {
      throw new Error("the seeded PNG never reached Koushi's media cache");
    }

    passed = true;
    log(
      `agent scenario search_attachment_reply=ok mcp_tools=${tools.length} ` +
        `reply_events=1 attachment_cached=1`
    );
  } finally {
    await stopProcess(hostChild);
    await stopProcess(homeserverChild);
    logStream?.end();
    hostLog.end();
    // A passing run leaves nothing behind; a failing one keeps its logs so CI
    // can upload them and a local run can be inspected with --keep.
    if (!options.keep && passed) {
      rmSync(runRoot, { recursive: true, force: true });
    } else {
      console.log(`[agent-scenario] kept the run directory for inspection: ${runRoot}`);
    }
    rmSync(bridgeDir, { recursive: true, force: true });
  }
}

main().then(
  () => process.exit(0),
  (error) => {
    console.error(`[agent-scenario] failed: ${error.message}`);
    process.exit(1);
  }
);
