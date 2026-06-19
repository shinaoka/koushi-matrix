#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { open } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const appProcessNames = ["koushi-desktop", "matrix-desktop", "matrix-desktop-app"];
let activeProcessName = appProcessNames[0];
const checks = [
  "launch Tauri dev shell",
  "verify main window",
  "optional real login from stdin",
  "optional reusable QA profile for restored sync state",
  "optional synthetic send smoke message",
  "verify QA title panel token after shortcuts",
  "open Keyboard settings shortcut",
  "open User settings shortcut",
  "capture private-data-free screenshots",
  "stop app process group"
];

const args = new Set(process.argv.slice(2));
const artifactDir = optionValue("--artifact-dir") ?? join(repoRoot, "artifacts", "mac-gui-smoke");
const timeoutMs = Number(optionValue("--timeout-ms") ?? "30000");
const sendTimeoutMs = Number(optionValue("--send-timeout-ms") ?? "30000");
const realLoginFromStdin = args.has("--real-login-from-stdin");
const keepSession = args.has("--keep-session");
const allowEmptyTimeline = args.has("--allow-empty-timeline");
const allowPrivateScreenshots = args.has("--allow-private-screenshots");
const verbose = args.has("--verbose");
const qaProfile = optionValue("--qa-profile");
const sendSmokeMessageOption = optionValue("--send-smoke-message");
const sendSmokeUserId = sendSmokeUserIdFromOption(optionValue("--send-smoke-user-id"));
const sendSmokeMessage =
  args.has("--send-smoke-message") || sendSmokeMessageOption !== undefined
    ? sendSmokeMessageFromOption(sendSmokeMessageOption)
    : null;

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

if (args.has("--child-env")) {
  for (const [key, value] of Object.entries(
    childEnvironment(qaDataDirForRun("/tmp/matrix-desktop-mac-gui-smoke"))
  ).sort(([left], [right]) => left.localeCompare(right))) {
    console.log(`${key}=${value}`);
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

const qaTitleSendReadySample = optionValue("--qa-title-send-ready");
if (qaTitleSendReadySample !== undefined) {
  console.log(qaStatusHasSendSuccess(parseQaTitle(qaTitleSendReadySample)) ? "ready" : "not-ready");
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
  const dataDir = qaDataDirForRun(runDir);
  const logPath = join(runDir, "tauri-dev.log");
  const diagnosticsPath = join(runDir, "qa-diagnostics.log");
  const qaLoginPipePath = realLogin ? join(runDir, "qa-login.pipe") : null;
  // Second FIFO: a debug/test-only control channel the harness uses to drive a
  // clean logout after a real login so no stale device survives the run.
  const qaControlPipePath = realLogin ? join(runDir, "qa-control.pipe") : null;
  mkdirSync(screenshotDir, { recursive: true });
  mkdirSync(dataDir, { recursive: true });
  const diagnostics = createQaDiagnostics(diagnosticsPath);
  diagnostics.record(
    "config",
    `real_login=${Boolean(realLogin)} qa_profile=${qaProfile !== undefined} send_smoke=${sendSmokeMessage !== null} allow_empty_timeline=${allowEmptyTimeline} timeout_ms=${timeoutMs} send_timeout_ms=${sendTimeoutMs}`
  );
  if (qaLoginPipePath) {
    createNamedPipe(qaLoginPipePath);
  }
  if (qaControlPipePath) {
    createNamedPipe(qaControlPipePath);
  }

  const child = spawn("npm", ["run", "tauri", "dev"], {
    cwd: desktopDir,
    env: childEnvironment(dataDir, qaLoginPipePath, qaControlPipePath),
    detached: true,
    stdio: ["ignore", "pipe", "pipe"]
  });

  // Tracks whether credentials were handed to the app. After that point a
  // partial login may have created a device, so teardown must attempt logout
  // even if the ready gate later fails.
  let realLoginCleanupRequired = false;

  const output = [];
  child.stdout.on("data", (chunk) => recordOutput(output, logPath, chunk));
  child.stderr.on("data", (chunk) => recordOutput(output, logPath, chunk));

  try {
    const windowInfo = await waitForWindow(timeoutMs, diagnostics);
    console.log(`ok verify main window: ${formatWindowInfo(windowInfo)}`);

    const initialWindowScreenshotIsAllowed = qaProfile === undefined || allowPrivateScreenshots;
    const postLoginScreenshotsAreAllowed =
      (!realLogin && qaProfile === undefined) || allowPrivateScreenshots;
    if (initialWindowScreenshotIsAllowed) {
      const firstRunScreenshot = join(screenshotDir, "01-first-run.png");
      await captureAppWindowScreenshot(firstRunScreenshot);
      requireNonEmptyFile(firstRunScreenshot, "first-run screenshot");
    } else {
      console.log("skip profile screenshot: restored windows can contain private room data");
    }

    if (realLogin) {
      await writeRealLoginPipe(qaLoginPipePath, realLogin);
      realLoginCleanupRequired = true;
      const qaTitle = await waitForQaTitle(
        timeoutMs,
        Boolean(realLogin.recoverySecret),
        allowEmptyTimeline,
        diagnostics
      );
      console.log(`ok real login QA: ${qaTitle}`);
      console.log("skip real login screenshot: post-login windows can contain private room data");
    } else if (qaProfile !== undefined) {
      const qaTitle = await waitForQaTitle(timeoutMs, false, allowEmptyTimeline, diagnostics);
      console.log(`ok restored session QA: ${qaTitle}`);
    }
    if (sendSmokeMessage !== null) {
      const qaSendTitle = await waitForQaSend(sendTimeoutMs, diagnostics);
      console.log(`ok send smoke QA: ${qaSendTitle}`);
    }

    await keyChord("/");
    const keyboardTitle = await waitForQaPanel(timeoutMs, "keyboardSettings", diagnostics);
    console.log(`ok keyboard settings QA: ${keyboardTitle}`);
    if (postLoginScreenshotsAreAllowed) {
      const keyboardScreenshot = join(screenshotDir, "02-keyboard-settings.png");
      await captureAppWindowScreenshot(keyboardScreenshot);
      requireNonEmptyFile(keyboardScreenshot, "keyboard settings screenshot");
    }

    await keyChord(",");
    const userSettingsTitle = await waitForQaPanel(timeoutMs, "userSettings", diagnostics);
    console.log(`ok user settings QA: ${userSettingsTitle}`);
    if (postLoginScreenshotsAreAllowed) {
      const userSettingsScreenshot = join(screenshotDir, "03-user-settings.png");
      await captureAppWindowScreenshot(userSettingsScreenshot);
      requireNonEmptyFile(userSettingsScreenshot, "user settings screenshot");
    }

    console.log(`mac GUI smoke passed: ${runDir}`);
    if (verbose) {
      console.log(`diagnostics path: ${diagnosticsPath}`);
    }
  } catch (error) {
    console.error(`mac GUI smoke failed. Artifacts: ${runDir}`);
    console.error(`diagnostics path: ${diagnosticsPath}`);
    console.error(tail(output.join(""), 40));
    throw error;
  } finally {
    // Real-login cleanup guard: if a real login reached ready and the caller
    // did not ask to keep the session, drive a logout through the QA control
    // pipe and wait for `session=signedOut` so no stale device survives the
    // run. Best-effort: a cleanup failure is logged, never thrown.
    if (qaControlPipePath && realLoginCleanupRequired && !keepSession) {
      try {
        await requestQaLogout(qaControlPipePath);
        const signedOutTitle = await waitForQaSignedOut(timeoutMs, diagnostics);
        console.log(`ok real login logout cleanup: ${signedOutTitle}`);
      } catch (cleanupError) {
        console.error(`real login logout cleanup failed: ${cleanupError.message}`);
      }
    }
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

function childEnvironment(dataDir, qaLoginPipePath = null, qaControlPipePath = null) {
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
  env.MATRIX_DESKTOP_RESTORE_SESSION = qaProfile !== undefined ? "1" : "0";
  env.MATRIX_DESKTOP_SKIP_SAVED_SESSIONS = qaProfile !== undefined ? "0" : "1";
  env.MATRIX_DESKTOP_DATA_DIR = dataDir;
  env.MATRIX_DESKTOP_QA_TITLE = "1";
  env.VITE_MATRIX_DESKTOP_QA_TITLE = "1";
  if (process.env.MATRIX_DESKTOP_DEBUG_SDK_ERROR) {
    env.MATRIX_DESKTOP_DEBUG_SDK_ERROR = "1";
  }
  if (sendSmokeMessage !== null) {
    env.VITE_MATRIX_DESKTOP_QA_SEND_SMOKE_MESSAGE = sendSmokeMessage;
  }
  if (sendSmokeUserId !== null) {
    env.VITE_MATRIX_DESKTOP_QA_SEND_SMOKE_USER_ID = sendSmokeUserId;
  }
  if (qaProfile !== undefined || realLoginFromStdin) {
    env.MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR = join(dataDir, "qa-credential-store");
  }
  if (realLoginFromStdin && qaProfile === undefined) {
    env.MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE = "1";
  }
  if (qaLoginPipePath) {
    env.MATRIX_DESKTOP_QA_LOGIN_PIPE = qaLoginPipePath;
  }
  if (qaControlPipePath) {
    env.MATRIX_DESKTOP_QA_CONTROL_PIPE = qaControlPipePath;
  }
  env.NO_COLOR = "1";
  return env;
}

function qaDataDirForRun(runDir) {
  if (qaProfile === undefined) {
    return join(runDir, "data");
  }
  return join(repoRoot, ".local-secrets", "qa-profiles", validatedQaProfileName(), "data");
}

function validatedQaProfileName() {
  if (!qaProfile || !/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(qaProfile)) {
    throw new Error("qa profile must be 1-64 characters of letters, numbers, underscore, or dash");
  }
  return qaProfile;
}

function sendSmokeMessageFromOption(value) {
  const message = value?.trim() || `Matrix Desktop synthetic QA send ${timestamp()}`;
  if (/[\r\n]/.test(message)) {
    throw new Error("send smoke message must be a single line");
  }
  return message;
}

function sendSmokeUserIdFromOption(value) {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  return trimmed.startsWith("@") ? trimmed : `@${trimmed}`;
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

async function waitForWindow(timeout, diagnostics = null) {
  const startedAt = Date.now();
  let lastError = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const value = await appleScript(windowQueryScript());
      recordQaPoll(diagnostics, "window", value);
      if (value !== "missing" && !value.endsWith("|no-window")) {
        const windowInfo = parseWindowInfo(value);
        activeProcessName = windowInfo.processName;
        return windowInfo;
      }
      lastError = value;
    } catch (error) {
      recordQaPoll(diagnostics, "window", error.message);
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

async function requestQaLogout(path) {
  // Reuse the FIFO writer (no parent environment is inherited) to push a single
  // control command. The logout command carries no secret values.
  const payload = JSON.stringify({ command: "logout" }) + "\n";
  await writeSensitivePayloadToPath(path, payload, 10000);
}

// Write a sensitive payload directly to the FIFO via node:fs/promises. No
// helper child process is spawned, so no parent environment is inherited by a
// `tee`-style writer (security: credential payloads must not leak the parent
// env to a child). `open(path, "w")` blocks until the reader opens the pipe,
// so the write is bounded by `timeout`.
async function writeSensitivePayloadToPath(path, payload, timeout) {
  let handle;
  const write = async () => {
    handle = await open(path, "w");
    await handle.writeFile(payload, "utf8");
  };
  try {
    await Promise.race([
      write(),
      new Promise((_, reject) =>
        setTimeout(() => reject(new Error("real login FIFO write timed out")), timeout)
      )
    ]);
  } finally {
    await handle?.close();
  }
}

async function waitForQaTitle(timeout, requireRecovered, allowEmptyTimeline, diagnostics = null) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const windowInfo = await currentWindowInfo();
      lastTitle = windowInfo.windowName;
      recordQaPoll(diagnostics, "ready", lastTitle);
      const status = parseQaTitle(lastTitle);
      if (qaStatusHasBlockingError(status)) {
        throw new Error(`QA reported an error before ready. Last title: ${lastTitle}`);
      }
      if (qaStatusIsReady(status, requireRecovered, allowEmptyTimeline)) {
        return summarizeQaStatus(status);
      }
    } catch (error) {
      lastTitle = error.message;
      recordQaPoll(diagnostics, "ready", lastTitle);
    }
    await sleep(1000);
  }
  throw new Error(`real login QA did not reach ready room/timeline state. Last title: ${lastTitle}`);
}

async function waitForQaPanel(timeout, requiredPanel, diagnostics = null) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const windowInfo = await currentWindowInfo();
      lastTitle = windowInfo.windowName;
      recordQaPoll(diagnostics, `panel:${requiredPanel}`, lastTitle);
      const status = parseQaTitle(lastTitle);
      if (qaStatusHasRequiredPanel(status, requiredPanel)) {
        return summarizeQaStatus(status);
      }
    } catch (error) {
      lastTitle = error.message;
      recordQaPoll(diagnostics, `panel:${requiredPanel}`, lastTitle);
    }
    await sleep(1000);
  }
  throw new Error(`real login QA did not report panel=${requiredPanel}. Last title: ${lastTitle}`);
}

async function waitForQaSend(timeout, diagnostics = null) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const windowInfo = await currentWindowInfo();
      lastTitle = windowInfo.windowName;
      recordQaPoll(diagnostics, "send", lastTitle);
      const status = parseQaTitle(lastTitle);
      if (status.send === "failed") {
        throw new Error(`send smoke failed. Last title: ${lastTitle}`);
      }
      if (qaStatusHasSendSuccess(status)) {
        return summarizeQaStatus(status);
      }
    } catch (error) {
      lastTitle = error.message;
      recordQaPoll(diagnostics, "send", lastTitle);
      if (lastTitle.includes("send smoke failed")) {
        throw error;
      }
    }
    await sleep(1000);
  }
  throw new Error(`send smoke QA did not reach send=sent. Last title: ${lastTitle}`);
}

async function waitForQaSignedOut(timeout, diagnostics = null) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    try {
      const windowInfo = await currentWindowInfo();
      lastTitle = windowInfo.windowName;
      recordQaPoll(diagnostics, "logout", lastTitle);
      const status = parseQaTitle(lastTitle);
      if (qaStatusIsSignedOut(status)) {
        return summarizeQaStatus(status);
      }
    } catch (error) {
      lastTitle = error.message;
      recordQaPoll(diagnostics, "logout", lastTitle);
    }
    await sleep(1000);
  }
  throw new Error(`logout cleanup did not reach session=signedOut. Last title: ${lastTitle}`);
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
  if (status.errors !== 0) {
    return false;
  }
  if (status.panel === requiredPanel) {
    return true;
  }
  return (
    status.panel === "recovery" &&
    (status.session === "needsRecovery" || status.session === "recovering")
  );
}

function qaStatusHasSendSuccess(status) {
  return status.errors === 0 && status.send === "sent";
}

function qaStatusHasBlockingError(status) {
  return status.errors > 0 || status.send === "failed";
}

function qaStatusIsSignedOut(status) {
  return status.session === "signedOut";
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
    status.errors === 0 &&
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
    `errors=${status.errors}`,
    `error_code=${status.error_code ?? "none"}`
  ];
  if (status.panel !== undefined) {
    values.push(`panel=${status.panel}`);
  }
  if (status.send !== undefined) {
    values.push(`send=${status.send}`);
  }
  for (const key of ["target_dm", "target_selected", "target_members"]) {
    if (status[key] !== undefined) {
      values.push(`${key}=${status[key]}`);
    }
  }
  return values.join(" ");
}

function createQaDiagnostics(path) {
  const startedAt = Date.now();
  return {
    record(phase, message) {
      const elapsed = Date.now() - startedAt;
      const line = `+${elapsed}ms ${phase} ${message}\n`;
      appendFileSync(path, line);
      if (verbose) {
        console.log(`[qa] ${line.trimEnd()}`);
      }
    }
  };
}

function recordQaPoll(diagnostics, phase, value) {
  diagnostics?.record(phase, summarizeQaDiagnosticValue(value));
}

function summarizeQaDiagnosticValue(value) {
  const text = String(value ?? "").replace(/\s+/g, " ").trim();
  const status = parseQaTitle(text);
  if (status.session !== undefined || status.sync !== undefined || status.errors !== undefined) {
    return summarizeQaStatus(status);
  }
  if (text === "missing" || text.endsWith("|no-window")) {
    return text;
  }
  return text.slice(0, 240);
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
    "Usage: node scripts/desktop-mac-gui-smoke.mjs --list|--check-tools|--child-env|--child-env-keys|--print-window-query-script|--print-screenshot-args|--print-real-login-transport|--qa-title-panel=TITLE|--qa-title-panel-ready=TITLE [--required-panel=PANEL]|--qa-title-ready=TITLE|--qa-title-send-ready=TITLE|--qa-title-ready-require-recovered=TITLE|--run [--real-login-from-stdin] [--keep-session] [--qa-profile=NAME] [--send-smoke-message[=BODY]] [--send-smoke-user-id=USER_ID] [--allow-empty-timeline] [--allow-private-screenshots] [--verbose] [--artifact-dir=PATH] [--timeout-ms=MS] [--send-timeout-ms=MS]"
  );
}
