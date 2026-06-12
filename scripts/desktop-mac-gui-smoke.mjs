#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const appProcessNames = ["matrix-desktop", "matrix-desktop-app"];
let activeProcessName = appProcessNames[0];
const checks = [
  "launch Tauri dev shell",
  "verify main window",
  "optional real login from stdin",
  "verify QA title panel token after shortcuts",
  "open Keyboard settings shortcut",
  "open User settings shortcut",
  "capture private-data-free screenshots",
  "stop app process group"
];

const args = new Set(process.argv.slice(2));
const artifactDir = optionValue("--artifact-dir") ?? join(repoRoot, "artifacts", "mac-gui-smoke");
const timeoutMs = Number(optionValue("--timeout-ms") ?? "120000");
const realLoginFromStdin = args.has("--real-login-from-stdin");
const allowEmptyTimeline = args.has("--allow-empty-timeline");

if (args.has("--list")) {
  for (const check of checks) {
    console.log(check);
  }
  process.exit(0);
}

if (args.has("--check-tools")) {
  checkMacTools();
  console.log("mac GUI smoke tools available");
  process.exit(0);
}

if (args.has("--child-env-keys")) {
  for (const key of Object.keys(childEnvironment("/tmp/matrix-desktop-mac-gui-smoke")).sort()) {
    console.log(key);
  }
  process.exit(0);
}

if (args.has("--print-window-query-script")) {
  console.log(windowQueryScript());
  process.exit(0);
}

if (args.has("--print-screenshot-args")) {
  console.log(
    screenshotArgs({ x: 10, y: 20, width: 300, height: 400 }, "/tmp/window.png").join("\n")
  );
  process.exit(0);
}

if (args.has("--print-real-login-transport")) {
  console.log("fifo");
  process.exit(0);
}

const qaTitlePanelSample = optionValue("--qa-title-panel");
if (qaTitlePanelSample !== undefined) {
  const status = parseQaTitle(qaTitlePanelSample);
  console.log(status.panel ?? "missing");
  process.exit(0);
}

const qaTitlePanelReadySample = optionValue("--qa-title-panel-ready");
if (qaTitlePanelReadySample !== undefined) {
  const requiredPanel = optionValue("--required-panel") ?? "keyboardSettings";
  const status = parseQaTitle(qaTitlePanelReadySample);
  console.log(qaStatusHasRequiredPanel(status, requiredPanel) ? "ready" : "not-ready");
  process.exit(0);
}

const qaTitleReadySample = optionValue("--qa-title-ready");
if (qaTitleReadySample !== undefined) {
  console.log(
    qaStatusIsReady(parseQaTitle(qaTitleReadySample), false, allowEmptyTimeline)
      ? "ready"
      : "not-ready"
  );
  process.exit(0);
}

const qaRecoveredTitleReadySample = optionValue("--qa-title-ready-require-recovered");
if (qaRecoveredTitleReadySample !== undefined) {
  console.log(
    qaStatusIsReady(parseQaTitle(qaRecoveredTitleReadySample), true, allowEmptyTimeline)
      ? "ready"
      : "not-ready"
  );
  process.exit(0);
}

if (args.has("--run")) {
  await run();
  process.exit(0);
}

printUsage();

async function run() {
  checkMacTools();
  const realLogin = realLoginFromStdin ? await readRealLoginCredentials() : null;

  const runDir = join(artifactDir, timestamp());
  const screenshotDir = join(runDir, "screenshots");
  const dataDir = join(runDir, "data");
  const logPath = join(runDir, "tauri-dev.log");
  const qaLoginPipePath = realLogin ? join(runDir, "qa-login.pipe") : null;
  mkdirSync(screenshotDir, { recursive: true });
  mkdirSync(dataDir, { recursive: true });
  if (qaLoginPipePath) {
    createNamedPipe(qaLoginPipePath);
  }

  const child = spawn("npm", ["run", "tauri", "dev"], {
    cwd: desktopDir,
    env: childEnvironment(dataDir, qaLoginPipePath),
    detached: true,
    stdio: ["ignore", "pipe", "pipe"]
  });

  const output = [];
  child.stdout.on("data", (chunk) => recordOutput(output, logPath, chunk));
  child.stderr.on("data", (chunk) => recordOutput(output, logPath, chunk));

  try {
    const windowInfo = await waitForWindow(timeoutMs);
    console.log(`ok verify main window: ${formatWindowInfo(windowInfo)}`);

    const firstRunScreenshot = join(screenshotDir, "01-first-run.png");
    await captureAppWindowScreenshot(firstRunScreenshot);
    requireNonEmptyFile(firstRunScreenshot, "first-run screenshot");

    if (realLogin) {
      await writeRealLoginPipe(qaLoginPipePath, realLogin);
      const qaTitle = await waitForQaTitle(
        timeoutMs,
        Boolean(realLogin.recoverySecret),
        allowEmptyTimeline
      );
      console.log(`ok real login QA: ${qaTitle}`);
      console.log("skip real login screenshot: post-login windows can contain private room data");
    }

    await keyChord("/");
    await sleep(1000);
    const keyboardTitle = await waitForQaPanel(timeoutMs, "keyboardSettings");
    console.log(`ok keyboard settings QA: ${keyboardTitle}`);
    if (!realLogin) {
      const keyboardScreenshot = join(screenshotDir, "02-keyboard-settings.png");
      await captureAppWindowScreenshot(keyboardScreenshot);
      requireNonEmptyFile(keyboardScreenshot, "keyboard settings screenshot");
    }

    await keyChord(",");
    await sleep(1000);
    const userSettingsTitle = await waitForQaPanel(timeoutMs, "userSettings");
    console.log(`ok user settings QA: ${userSettingsTitle}`);
    if (!realLogin) {
      const userSettingsScreenshot = join(screenshotDir, "03-user-settings.png");
      await captureAppWindowScreenshot(userSettingsScreenshot);
      requireNonEmptyFile(userSettingsScreenshot, "user settings screenshot");
    }

    console.log(`mac GUI smoke passed: ${runDir}`);
  } catch (error) {
    console.error(`mac GUI smoke failed. Artifacts: ${runDir}`);
    console.error(tail(output.join(""), 40));
    throw error;
  } finally {
    terminateProcessGroup(child, "SIGTERM");
    await settleChild(child);
    if (output.join("").includes("error:")) {
      console.log("tauri output contained an error marker; inspect artifacts if the smoke failed");
    }
  }
}

function checkMacTools() {
  if (process.platform !== "darwin") {
    throw new Error("mac GUI smoke must run on macOS");
  }
  for (const tool of ["osascript", "screencapture", "npm"]) {
    execFileSync("/usr/bin/which", [tool], { encoding: "utf8", stdio: "ignore" });
  }
}

function childEnvironment(dataDir, qaLoginPipePath = null) {
  const allowedKeys = [
    "AR",
    "CARGO_HOME",
    "CC",
    "CFLAGS",
    "CPATH",
    "CPPFLAGS",
    "CXX",
    "CXXFLAGS",
    "HOME",
    "LANG",
    "LC_ALL",
    "LDFLAGS",
    "LIBRARY_PATH",
    "LOGNAME",
    "MACOSX_DEPLOYMENT_TARGET",
    "NPM_CONFIG_USERCONFIG",
    "PATH",
    "PKG_CONFIG_PATH",
    "RUSTFLAGS",
    "RUSTUP_HOME",
    "SDKROOT",
    "SHELL",
    "TMPDIR",
    "USER",
    "npm_config_userconfig"
  ];
  const env = {};
  for (const key of allowedKeys) {
    if (process.env[key]) {
      env[key] = process.env[key];
    }
  }
  env.MATRIX_DESKTOP_RESTORE_SESSION = "0";
  env.MATRIX_DESKTOP_SKIP_SAVED_SESSIONS = "1";
  env.MATRIX_DESKTOP_DATA_DIR = dataDir;
  env.MATRIX_DESKTOP_QA_TITLE = "1";
  env.VITE_MATRIX_DESKTOP_QA_TITLE = "1";
  if (realLoginFromStdin) {
    env.MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE = "1";
  }
  if (qaLoginPipePath) {
    env.MATRIX_DESKTOP_QA_LOGIN_PIPE = qaLoginPipePath;
  }
  env.NO_COLOR = "1";
  return env;
}

function readRealLoginCredentials() {
  return new Promise((resolve, reject) => {
    let input = "";
    let settled = false;
    const settle = (callback) => {
      if (settled) {
        return;
      }
      settled = true;
      callback();
    };
    const parseInput = () => {
      try {
        const credentials = realLoginCredentialsFromInput(input);
        settle(() => resolve(credentials));
      } catch (error) {
        settle(() => reject(error));
      }
    };
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => {
      input += chunk;
      if (completeRealLoginInputWasReceived(input)) {
        parseInput();
      }
    });
    process.stdin.on("error", reject);
    process.stdin.on("end", () => {
      parseInput();
    });
    process.stdin.resume();
  });
}

function completeRealLoginInputWasReceived(input) {
  return (input.replace(/\r/g, "").match(/\n/g) ?? []).length >= 5;
}

function realLoginCredentialsFromInput(input) {
  const [homeserverInput, username, password, deviceNameInput, recoverySecretInput] = input
    .replace(/\r/g, "")
    .split("\n");
  const homeserver = homeserverInput.trim() || "https://matrix.org";
  const deviceName = deviceNameInput?.trim() || "Matrix Desktop Smoke Test";
  const recoverySecret = recoverySecretInput?.trim() || null;
  if (!username?.trim() || !password?.trim()) {
    throw new Error("real login stdin must contain homeserver, username, and password lines");
  }
  return {
    homeserver,
    username: username.trim(),
    password: password.trim(),
    deviceName,
    recoverySecret
  };
}

async function waitForWindow(timeout) {
  const startedAt = Date.now();
  let lastError = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const value = await appleScript(windowQueryScript());
      if (value !== "missing" && !value.endsWith("|no-window")) {
        const windowInfo = parseWindowInfo(value);
        activeProcessName = windowInfo.processName;
        return windowInfo;
      }
      lastError = value;
    } catch (error) {
      if (error.message.includes("AppleScript timed out")) {
        throw new Error(
          `${error.message}. Grant Accessibility permission to the terminal running this script, then rerun qa:mac-gui.`
        );
      }
      lastError = error.message;
    }
    await sleep(1000);
  }
  throw new Error(
    `matrix-desktop window did not appear within ${timeout}ms. Last state: ${lastError}`
  );
}

async function currentWindowInfo() {
  const value = await appleScript(windowQueryScript());
  if (value === "missing" || value.endsWith("|no-window")) {
    throw new Error(`matrix-desktop window is unavailable: ${value}`);
  }
  const windowInfo = parseWindowInfo(value);
  activeProcessName = windowInfo.processName;
  return windowInfo;
}

function windowQueryScript() {
  return `
tell application "System Events"
  set candidateNames to {${appProcessNames.map((name) => `"${name}"`).join(", ")}}
  repeat with candidateNameRef in candidateNames
    set candidateName to candidateNameRef as text
    if exists (first process whose name is candidateName) then
      tell (first process whose name is candidateName)
        set frontmost to true
        if (count of windows) is 0 then return candidateName & "|no-window"
        set windowName to name of window 1
        set windowPosition to position of window 1
        set windowSize to size of window 1
        return candidateName & "|" & windowName & "|" & (item 1 of windowPosition as string) & "," & (item 2 of windowPosition as string) & "|" & (item 1 of windowSize as string) & "x" & (item 2 of windowSize as string)
      end tell
    end if
  end repeat
  return "missing"
end tell
`;
}

function parseWindowInfo(value) {
  const [processName, windowName, position, size] = value.split("|");
  const [x, y] = parseNumberPair(position, ",");
  const [width, height] = parseNumberPair(size, "x");
  if (!processName || !windowName || width <= 0 || height <= 0) {
    throw new Error(`invalid window info: ${value}`);
  }
  return { processName, windowName, x, y, width, height };
}

function parseNumberPair(value, separator) {
  const parts = value?.split(separator).map((part) => Number(part.trim())) ?? [];
  if (parts.length !== 2 || parts.some((part) => !Number.isFinite(part))) {
    throw new Error(`invalid window geometry: ${value}`);
  }
  return parts;
}

function formatWindowInfo({ windowName, x, y, width, height }) {
  return `${windowName}|${x},${y}|${width}x${height}`;
}

async function keyChord(key) {
  await appleScript(`
tell application "System Events"
  tell process "${activeProcessName}"
    set frontmost to true
    keystroke "${key}" using command down
  end tell
end tell
`);
}

function createNamedPipe(path) {
  execFileSync("mkfifo", [path], { stdio: "ignore" });
}

async function writeRealLoginPipe(path, credentials) {
  const payloadObject = {
    homeserver: credentials.homeserver,
    username: credentials.username,
    password: credentials.password,
    device_display_name: credentials.deviceName
  };
  if (credentials.recoverySecret) {
    payloadObject.recovery_secret = credentials.recoverySecret;
  }
  const payload = JSON.stringify(payloadObject) + "\n";
  await writeSensitivePayloadToPath(path, payload, 10000);
}

function writeSensitivePayloadToPath(path, payload, timeout) {
  return new Promise((resolve, reject) => {
    const child = spawn("tee", [path], { stdio: ["pipe", "ignore", "pipe"] });
    let stderr = "";
    let settled = false;
    const settle = (callback) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      callback();
    };
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      settle(() => reject(new Error("real login FIFO write timed out")));
    }, timeout);

    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", (error) => settle(() => reject(error)));
    child.on("exit", (code, signal) => {
      settle(() => {
        if (code === 0) {
          resolve();
        } else {
          reject(new Error(stderr.trim() || `real login FIFO writer exited with ${code ?? signal}`));
        }
      });
    });
    child.stdin.end(payload);
  });
}

async function waitForQaTitle(timeout, requireRecovered, allowEmptyTimeline) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const windowInfo = await currentWindowInfo();
      lastTitle = windowInfo.windowName;
      const status = parseQaTitle(lastTitle);
      if (qaStatusIsReady(status, requireRecovered, allowEmptyTimeline)) {
        return summarizeQaStatus(status);
      }
    } catch (error) {
      lastTitle = error.message;
    }
    await sleep(1000);
  }
  throw new Error(`real login QA did not reach ready room/timeline state. Last title: ${lastTitle}`);
}

async function waitForQaPanel(timeout, requiredPanel) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const windowInfo = await currentWindowInfo();
      lastTitle = windowInfo.windowName;
      const status = parseQaTitle(lastTitle);
      if (qaStatusHasRequiredPanel(status, requiredPanel)) {
        return summarizeQaStatus(status);
      }
    } catch (error) {
      lastTitle = error.message;
    }
    await sleep(1000);
  }
  throw new Error(`real login QA did not report panel=${requiredPanel}. Last title: ${lastTitle}`);
}

function parseQaTitle(title) {
  const status = {};
  for (const token of title.split(/\s+/)) {
    const [key, value] = token.split("=");
    if (!value) {
      continue;
    }
    if (["rooms", "spaces", "timeline_items", "errors"].includes(key)) {
      status[key] = Number(value);
    } else if (["active_room", "timeline_subscribed"].includes(key)) {
      status[key] = value === "true";
    } else {
      status[key] = value;
    }
  }
  return status;
}

function qaStatusHasRequiredPanel(status, requiredPanel) {
  if (status.panel === requiredPanel) {
    return true;
  }
  return (
    status.panel === "recovery" &&
    (status.session === "needsRecovery" || status.session === "recovering")
  );
}

function qaStatusIsReady(status, requireRecovered, allowEmptyTimeline = false) {
  const sessionReady = requireRecovered
    ? status.session === "ready"
    : status.session === "ready" || status.session === "needsRecovery";
  const timelineReady = allowEmptyTimeline
    ? Number.isFinite(status.timeline_items) && status.timeline_items >= 0
    : status.timeline_items > 0;
  return (
    sessionReady &&
    status.sync === "running" &&
    status.rooms > 0 &&
    status.active_room === true &&
    status.timeline_subscribed === true &&
    timelineReady
  );
}

function summarizeQaStatus(status) {
  const values = [
    `session=${status.session}`,
    `sync=${status.sync}`,
    `rooms=${status.rooms}`,
    `spaces=${status.spaces}`,
    `active_room=${status.active_room}`,
    `timeline_subscribed=${status.timeline_subscribed}`,
    `timeline_items=${status.timeline_items}`,
    `errors=${status.errors}`
  ];
  if (status.panel !== undefined) {
    values.push(`panel=${status.panel}`);
  }
  return values.join(" ");
}

async function captureAppWindowScreenshot(path) {
  const windowInfo = await currentWindowInfo();
  captureScreenshot(path, windowInfo);
}

function captureScreenshot(path, windowInfo) {
  execFileSync("screencapture", screenshotArgs(windowInfo, path), { stdio: "ignore" });
}

function screenshotArgs({ x, y, width, height }, path) {
  return ["-x", "-R", `${round(x)},${round(y)},${round(width)},${round(height)}`, path];
}

function round(value) {
  return Math.round(value);
}

function requireNonEmptyFile(path, label) {
  if (!existsSync(path) || statSync(path).size === 0) {
    throw new Error(`${label} was not captured`);
  }
}

function appleScript(source, timeoutMs = 5000) {
  return new Promise((resolve, reject) => {
    const child = spawn("osascript", source.split("\n").flatMap((line) => ["-e", line]), {
      stdio: ["ignore", "pipe", "pipe"]
    });
    let stdout = "";
    let stderr = "";
    let settled = false;

    const settle = (callback) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      callback();
    };
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      settle(() => reject(new Error("AppleScript timed out while controlling System Events")));
    }, timeoutMs);

    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", (error) => {
      settle(() => reject(error));
    });
    child.on("exit", (code, signal) => {
      settle(() => {
        if (code === 0) {
          resolve(stdout.trim());
        } else {
          reject(new Error(stderr.trim() || `AppleScript exited with ${code ?? signal}`));
        }
      });
    });
  });
}

function optionValue(name) {
  const prefix = `${name}=`;
  const value = process.argv.find((argument) => argument.startsWith(prefix));
  return value?.slice(prefix.length);
}

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, "-");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function recordOutput(output, logPath, chunk) {
  const text = chunk.toString();
  output.push(text);
  appendFileSync(logPath, text);
}

function terminateProcessGroup(child, signal) {
  if (!child.pid) {
    return;
  }
  try {
    process.kill(-child.pid, signal);
  } catch {
    child.kill(signal);
  }
}

function settleChild(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      terminateProcessGroup(child, "SIGKILL");
      resolve();
    }, 5000);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

function tail(value, lines) {
  const allLines = value.trimEnd().split("\n");
  return allLines.slice(Math.max(0, allLines.length - lines)).join("\n");
}

function printUsage() {
  console.log(
    "Usage: node scripts/desktop-mac-gui-smoke.mjs --list|--check-tools|--child-env-keys|--print-window-query-script|--print-screenshot-args|--print-real-login-transport|--qa-title-panel=TITLE|--qa-title-panel-ready=TITLE [--required-panel=PANEL]|--qa-title-ready=TITLE|--qa-title-ready-require-recovered=TITLE|--run [--real-login-from-stdin] [--allow-empty-timeline] [--artifact-dir=PATH] [--timeout-ms=MS]"
  );
}
