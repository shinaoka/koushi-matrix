#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import * as net from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const desktopPackageRequire = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const checks = [
  "verify Xvfb virtual display",
  "verify tauri-driver and WebKitWebDriver",
  "verify debug Tauri build",
  "drive WebdriverIO session",
  "exercise real IPC and DOM smoke",
  "optional local homeserver login via FIFO",
  "clean process teardown"
];

const args = new Set(process.argv.slice(2));
const qaProfile = optionValue("--qa-profile");
const realLoginFromStdin = args.has("--real-login-from-stdin");
const allowEmptyTimeline = args.has("--allow-empty-timeline");
const artifactRoot = optionValue("--artifact-dir") ?? join(repoRoot, "artifacts", "linux-gui-qa");
const timeoutMs = Number(optionValue("--timeout-ms") ?? "120000");

if (args.has("--list")) {
  for (const check of checks) {
    console.log(check);
  }
  process.exit(0);
}

if (args.has("--check-tools")) {
  checkLinuxTools();
  console.log("linux GUI smoke tools available");
  process.exit(0);
}

if (args.has("--child-env-keys")) {
  for (const key of Object.keys(childEnvironment("/tmp/matrix-desktop-linux-gui-qa")).sort()) {
    console.log(key);
  }
  process.exit(0);
}

if (args.has("--child-env")) {
  for (const [key, value] of Object.entries(
    childEnvironment(qaDataDirForRun("/tmp/matrix-desktop-linux-gui-qa"))
  ).sort(([left], [right]) => left.localeCompare(right))) {
    console.log(`${key}=${value}`);
  }
  process.exit(0);
}

if (args.has("--print-real-login-transport")) {
  console.log("fifo");
  process.exit(0);
}

if (args.has("--print-webdriver-capabilities")) {
  const appBinary = optionValue("--app-binary");
  if (!appBinary) {
    throw new Error("--app-binary=PATH is required when printing WebDriver capabilities");
  }
  console.log(JSON.stringify(webdriverCapabilities(appBinary), null, 2));
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

const qaTitleAttentionReadySample = optionValue("--qa-title-attention-ready");
if (qaTitleAttentionReadySample !== undefined) {
  console.log(
    qaStatusHasAttentionBaseline(parseQaTitle(qaTitleAttentionReadySample)) ? "ready" : "not-ready"
  );
  process.exit(0);
}

const qaWindowStateReadySample = optionValue("--qa-window-state-ready");
if (qaWindowStateReadySample !== undefined) {
  console.log(qaWindowStatePathHasContract(qaWindowStateReadySample) ? "ready" : "not-ready");
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
  checkLinuxTools();

  const runDir = join(artifactRoot, timestamp());
  const screenshotDir = join(runDir, "screenshots");
  const dataDir = qaDataDirForRun(runDir);
  const logPath = join(runDir, "run.log");
  mkdirSync(screenshotDir, { recursive: true });
  mkdirSync(dataDir, { recursive: true });

  const baseEnv = childEnvironment(dataDir);
  const dbusSession = ensureDbusSession(logPath, baseEnv);
  const buildEnv = {
    ...baseEnv,
    ...dbusSession.env
  };
  const xvfb = await startXvfb(logPath, buildEnv);
  const driverPort = await findFreePort();
  const nativePort = await findFreePort();
  const tauriDriver = spawnLogged(
    "tauri-driver",
    ["--port", String(driverPort), "--native-port", String(nativePort)],
    {
      cwd: desktopDir,
      env: { ...buildEnv, DISPLAY: `:${xvfb.display}` },
      detached: true,
      logPath,
      label: "tauri-driver"
    }
  );

  let browser;
  let appLaunched = false;
  let dbusMonitor = null;
  try {
    await runLoggedCommand("npm", ["run", "tauri", "build", "--", "--debug", "--no-bundle"], {
      cwd: desktopDir,
      env: buildEnv,
      logPath,
      label: "tauri build"
    });

    const appBinary = resolveDebugAppBinary();
    await waitForPort("127.0.0.1", driverPort, timeoutMs);

    const { remote } = await importDesktopWebdriverio();
    browser = await remote({
      hostname: "127.0.0.1",
      port: driverPort,
      logLevel: "error",
      capabilities: webdriverCapabilities(appBinary)
    });
    appLaunched = true;

    const authScreen = await browser.$('[data-testid="auth-screen"]');
    await authScreen.waitForDisplayed({ timeout: timeoutMs });
    console.log("auth_screen=ok");

    await waitForSignedOutTitle(browser, timeoutMs);
    console.log("title_signed_out=ok");

    const screenshotPath = join(runDir, "screenshots/01-signed-out.png");
    await browser.saveScreenshot(screenshotPath);
    requireNonEmptyFile(screenshotPath, "signed-out screenshot");
    console.log("screenshot=ok");

    dbusMonitor = startDbusMonitor(logPath, buildEnv);
    await waitForDbusMonitorReady(dbusMonitor, timeoutMs);
    await triggerNotificationSmoke(browser, timeoutMs);
    await waitForDbusMonitorToken(dbusMonitor, timeoutMs);
    console.log("notification_dbus=ok");

    console.log(`run_dir=${runDir}`);
  } finally {
    try {
      if (dbusMonitor) {
        terminateProcessGroup(dbusMonitor.child, "SIGTERM");
        await settleChild(dbusMonitor.child);
      }
      if (browser) {
        await safeDeleteSession(browser);
      }
      if (appLaunched) {
        const windowStatePath = join(dataDir, "app-shell", "window-state.json");
        console.log(`window_state_path=${windowStatePath}`);
        console.log("window_state_path_contract=ok");
      }
    } finally {
      if (dbusSession.pid) {
        try {
          process.kill(dbusSession.pid, "SIGTERM");
        } catch {
          // ignore cleanup failures
        }
      }
      terminateProcessGroup(tauriDriver, "SIGTERM");
      await settleChild(tauriDriver);
      terminateProcessGroup(xvfb.child, "SIGTERM");
      await settleChild(xvfb.child);
    }
  }
}

function checkLinuxTools() {
  if (process.platform !== "linux") {
    throw new Error("linux GUI smoke must run on Linux");
  }
  const requiredTools = [
    "npm",
    "cargo",
    "Xvfb",
    "tauri-driver",
    "WebKitWebDriver",
    "mkfifo",
    "dbus-daemon",
    "dbus-monitor"
  ];
  const missing = [];
  for (const tool of requiredTools) {
    try {
      execFileSync("which", [tool], { encoding: "utf8", stdio: "ignore" });
    } catch {
      missing.push(tool);
    }
  }
  if (missing.length) {
    throw new Error(`missing required Linux GUI tools: ${missing.join(", ")}`);
  }
}

function childEnvironment(dataDir) {
  const allowedKeys = [
    "AR",
    "CARGO_HOME",
    "CC",
    "CFLAGS",
    "CPATH",
    "CPPFLAGS",
    "CXX",
    "CXXFLAGS",
    "DBUS_SESSION_BUS_ADDRESS",
    "DISPLAY",
    "GDK_BACKEND",
    "HOME",
    "LANG",
    "LC_ALL",
    "LDFLAGS",
    "LIBRARY_PATH",
    "LOGNAME",
    "NPM_CONFIG_USERCONFIG",
    "PATH",
    "PKG_CONFIG_PATH",
    "RUSTFLAGS",
    "RUSTUP_HOME",
    "SHELL",
    "TMPDIR",
    "USER",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "npm_config_userconfig"
  ];
  const env = {};
  for (const key of allowedKeys) {
    if (process.env[key]) {
      env[key] = process.env[key];
    }
  }
  env.GDK_BACKEND = "x11";
  env.MATRIX_DESKTOP_RESTORE_SESSION = qaProfile !== undefined ? "1" : "0";
  env.MATRIX_DESKTOP_SKIP_SAVED_SESSIONS = "1";
  env.MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE = "1";
  env.MATRIX_DESKTOP_DATA_DIR = dataDir;
  env.MATRIX_DESKTOP_QA_TITLE = "1";
  env.VITE_MATRIX_DESKTOP_QA_TITLE = "1";
  env.MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR = join(dataDir, "qa-credential-store");
  env.NO_COLOR = "1";
  if (qaProfile !== undefined) {
    env.MATRIX_DESKTOP_RESTORE_SESSION = "1";
  }
  if (realLoginFromStdin) {
    env.MATRIX_DESKTOP_QA_LOGIN_PIPE = join(dataDir, "qa-login.pipe");
  }
  Object.assign(env, nssWrapperEnvironment(dataDir));
  return env;
}

function nssWrapperEnvironment(dataDir) {
  const libraryPath = "/usr/lib/x86_64-linux-gnu/libnss_wrapper.so";
  if (!existsSync(libraryPath)) {
    return {};
  }

  const uid = typeof process.getuid === "function" ? process.getuid() : null;
  const gid = typeof process.getgid === "function" ? process.getgid() : null;
  if (!Number.isInteger(uid) || !Number.isInteger(gid)) {
    return {};
  }

  const nssDir = join(dataDir, "qa-nss-wrapper");
  mkdirSync(nssDir, { recursive: true });

  const passwdPath = join(nssDir, "passwd");
  const groupPath = join(nssDir, "group");
  writeFileSync(passwdPath, `matrix-desktop:x:${uid}:${gid}:Matrix Desktop:/tmp:/bin/sh\n`);
  writeFileSync(groupPath, `matrix-desktop:x:${gid}:\n`);

  return {
    LD_PRELOAD: buildLdPreload(libraryPath),
    NSS_WRAPPER_PASSWD: passwdPath,
    NSS_WRAPPER_GROUP: groupPath
  };
}

function buildLdPreload(libraryPath) {
  const existing = process.env.LD_PRELOAD?.trim();
  return existing ? `${libraryPath} ${existing}` : libraryPath;
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

async function startXvfb(logPath, buildEnv) {
  const display = await findFreeDisplayNumber();
  const child = spawn("Xvfb", [`:${display}`, "-screen", "0", "1280x900x24", "-nolisten", "tcp", "-ac"], {
    cwd: repoRoot,
    env: buildEnv,
    detached: true,
    stdio: ["ignore", "pipe", "pipe"]
  });
  recordProcessOutput(child, logPath, "Xvfb");
  child.unref();
  await waitForDisplaySocket(display, timeoutMs);
  return { child, display };
}

async function waitForSignedOutTitle(browser, timeout) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    if (
      lastTitle.includes("session=signedOut") &&
      lastTitle.includes("errors=0") &&
      qaStatusHasAttentionBaseline(parseQaTitle(lastTitle))
    ) {
      return lastTitle;
    }
    await sleep(250);
  }
  throw new Error(`signed-out QA title did not appear. Last title: ${lastTitle}`);
}

function spawnLogged(command, argsList, { cwd, env, detached = false, logPath, label }) {
  const child = spawn(command, argsList, {
    cwd,
    env,
    detached,
    stdio: ["ignore", "pipe", "pipe"]
  });
  recordProcessOutput(child, logPath, label);
  if (detached) {
    child.unref();
  }
  return child;
}

function recordProcessOutput(child, logPath, label) {
  const prefix = `[${label}] `;
  child.stdout.on("data", (chunk) => appendFileSync(logPath, prefix + chunk.toString()));
  child.stderr.on("data", (chunk) => appendFileSync(logPath, prefix + chunk.toString()));
  child.on("error", (error) => {
    appendFileSync(logPath, `${prefix}error: ${error.message}\n`);
  });
}

async function runLoggedCommand(command, argsList, { cwd, env, logPath, label }) {
  const child = spawn(command, argsList, {
    cwd,
    env,
    stdio: ["ignore", "pipe", "pipe"]
  });
  recordProcessOutput(child, logPath, label);
  const exitCode = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve(0);
      } else {
        reject(new Error(`${label} exited with ${code ?? signal}`));
      }
    });
  });
  return exitCode;
}

async function importDesktopWebdriverio() {
  const webdriverioEntry = desktopPackageRequire.resolve("webdriverio");
  return await import(pathToFileURL(webdriverioEntry).href);
}

function resolveDebugAppBinary() {
  const cargoTargetDir = process.env.CARGO_TARGET_DIR;
  const candidates = [];
  if (cargoTargetDir) {
    candidates.push(join(cargoTargetDir, "debug", "matrix-desktop-app"));
    candidates.push(join(cargoTargetDir, "debug", "matrix-desktop"));
  }
  candidates.push(join(desktopDir, "src-tauri", "target", "debug", "matrix-desktop-app"));
  candidates.push(join(desktopDir, "src-tauri", "target", "debug", "matrix-desktop"));
  candidates.push(join(repoRoot, "target", "debug", "matrix-desktop-app"));
  candidates.push(join(repoRoot, "target", "debug", "matrix-desktop"));
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found) {
    throw new Error(`unable to resolve debug Tauri binary. Checked: ${candidates.join(", ")}`);
  }
  return found;
}

function webdriverCapabilities(appBinary) {
  return {
    browserName: "wry",
    "wdio:enforceWebDriverClassic": true,
    "tauri:options": {
      application: appBinary
    }
  };
}

function parseQaTitle(title) {
  const status = {};
  for (const token of title.split(/\s+/)) {
    const [key, value] = token.split("=");
    if (!value) {
      continue;
    }
    if (["rooms", "spaces", "timeline_items", "errors", "unread", "badge"].includes(key)) {
      status[key] = Number(value);
    } else if (["active_room", "timeline_subscribed"].includes(key)) {
      status[key] = value === "true";
    } else {
      status[key] = value;
    }
  }
  return status;
}

function qaStatusHasAttentionBaseline(status) {
  return status.unread === 0 && status.badge === 0 && status.notify === "none";
}

function qaWindowStatePathHasContract(path) {
  return normalizePath(path).endsWith("/app-shell/window-state.json");
}

function ensureDbusSession(logPath, env) {
  if (process.env.DBUS_SESSION_BUS_ADDRESS) {
    return {
      env: { DBUS_SESSION_BUS_ADDRESS: process.env.DBUS_SESSION_BUS_ADDRESS },
      pid: null
    };
  }

  const output = execFileSync(
    "dbus-daemon",
    ["--session", "--fork", "--print-address=1", "--print-pid=1"],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env
    }
  );
  appendFileSync(logPath, `[dbus-daemon] ${output}`);

  const [addressLine, pidLine] = output
    .trim()
    .split(/\s*\n\s*/)
    .filter((line) => line.length > 0);
  const pid = Number(pidLine);
  if (!addressLine || !Number.isFinite(pid) || pid <= 0) {
    throw new Error(`dbus-daemon did not return a usable session bus: ${output}`);
  }

  return {
    env: { DBUS_SESSION_BUS_ADDRESS: addressLine },
    pid
  };
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

function optionValue(name) {
  const prefix = `${name}=`;
  const value = process.argv.find((argument) => argument.startsWith(prefix));
  return value?.slice(prefix.length);
}

function requireNonEmptyFile(path, label) {
  if (!existsSync(path) || statSync(path).size === 0) {
    throw new Error(`${label} was not captured`);
  }
}

async function waitForDisplaySocket(display, timeout) {
  const socketPath = `/tmp/.X11-unix/X${display}`;
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeout) {
    if (existsSync(socketPath)) {
      return;
    }
    await sleep(100);
  }
  throw new Error(`Xvfb display :${display} did not become available`);
}

async function findFreeDisplayNumber() {
  for (let display = 90; display < 200; display += 1) {
    if (!existsSync(`/tmp/.X11-unix/X${display}`)) {
      return display;
    }
  }
  throw new Error("unable to find a free Xvfb display");
}

async function findFreePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address && typeof address === "object") {
        const { port } = address;
        server.close(() => resolve(port));
        return;
      }
      server.close(() => reject(new Error("failed to acquire a free port")));
    });
  });
}

async function waitForPort(hostname, port, timeout) {
  const startedAt = Date.now();
  let lastError = "";
  while (Date.now() - startedAt < timeout) {
    try {
      await connectOnce(hostname, port, 1000);
      return;
    } catch (error) {
      lastError = error.message;
      await sleep(100);
    }
  }
  throw new Error(`port ${port} on ${hostname} did not become ready: ${lastError}`);
}

function connectOnce(hostname, port, timeout) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: hostname, port });
    const fail = (error) => {
      socket.destroy();
      reject(error);
    };
    socket.setTimeout(timeout);
    socket.once("connect", () => {
      socket.end();
      resolve();
    });
    socket.once("timeout", () => fail(new Error(`timed out connecting to ${hostname}:${port}`)));
    socket.once("error", fail);
  });
}

async function safeDeleteSession(browser) {
  try {
    await browser.deleteSession();
  } catch {
    // ignore cleanup failures
  }
}

function normalizePath(path) {
  return path.replace(/\\/g, "/");
}

function startDbusMonitor(logPath, env) {
  const busAddress = env.DBUS_SESSION_BUS_ADDRESS;
  if (!busAddress) {
    throw new Error("DBUS_SESSION_BUS_ADDRESS is required to start the notification DBus monitor");
  }
  const child = spawn(
    "dbus-monitor",
    ["--address", busAddress, "interface='org.freedesktop.Notifications'"],
    {
      cwd: repoRoot,
      env,
      detached: true,
      stdio: ["ignore", "pipe", "pipe"]
    }
  );
  const state = { child, buffer: "" };
  recordProcessOutput(child, logPath, "dbus-monitor");
  child.stdout.on("data", (chunk) => {
    state.buffer += chunk.toString();
  });
  child.stderr.on("data", (chunk) => {
    state.buffer += chunk.toString();
  });
  child.once("exit", (code, signal) => {
    if (code !== 0 && signal === null) {
      state.buffer += `\ndbus-monitor exited with ${code}\n`;
    }
  });
  child.unref();
  return state;
}

async function triggerNotificationSmoke(browser, timeout) {
  const result = await browser.executeAsync((done) => {
    const notificationApi = window.Notification;
    if (!notificationApi) {
      done({ ok: false, reason: "notification_api_unavailable" });
      return;
    }

    Promise.resolve(notificationApi.requestPermission())
      .then((permission) => {
        if (permission !== "granted") {
          done({ ok: false, reason: `permission_${permission}` });
          return;
        }

        const notification = new notificationApi("Matrix Desktop QA", {
          body: "Notification smoke"
        });
        window.setTimeout(() => {
          try {
            notification.close();
          } catch {
            // ignore close errors
          }
          done({ ok: true });
        }, 0);
      })
      .catch((error) => {
        done({ ok: false, reason: String(error) });
      });
  });

  if (!result?.ok) {
    throw new Error(`notification smoke failed: ${result?.reason ?? "unknown error"}`);
  }

  await sleep(Math.min(timeout, 250));
}

async function waitForDbusMonitorToken(monitor, timeout) {
  const startedAt = Date.now();
  let lastBuffer = "";
  while (Date.now() - startedAt < timeout) {
    lastBuffer = monitor.buffer;
    if (
      lastBuffer.includes("org.freedesktop.Notifications") &&
      lastBuffer.includes("Notify")
    ) {
      return;
    }
    if (monitor.child.exitCode !== null || monitor.child.signalCode !== null) {
      throw new Error(`notification DBus monitor exited early. Last output: ${lastBuffer}`);
    }
    await sleep(100);
  }
  throw new Error(`notification DBus evidence not observed. Last monitor output: ${lastBuffer}`);
}

async function waitForDbusMonitorReady(monitor, timeout) {
  const startedAt = Date.now();
  let lastBuffer = "";
  while (Date.now() - startedAt < timeout) {
    lastBuffer = monitor.buffer;
    if (lastBuffer.includes("NameAcquired") || lastBuffer.includes("monitoring")) {
      return;
    }
    if (monitor.child.exitCode !== null || monitor.child.signalCode !== null) {
      throw new Error(`notification DBus monitor exited before readiness. Last output: ${lastBuffer}`);
    }
    await sleep(100);
  }
  throw new Error(`notification DBus monitor did not become ready. Last monitor output: ${lastBuffer}`);
}

function terminateProcessGroup(child, signal) {
  if (!child?.pid) {
    return;
  }
  try {
    process.kill(-child.pid, signal);
  } catch {
    try {
      child.kill(signal);
    } catch {
      // ignore cleanup failures
    }
  }
}

async function settleChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  await new Promise((resolve) => {
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

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, "-");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function printUsage() {
  console.log(
    "Usage: node scripts/desktop-linux-gui-qa.mjs --list|--check-tools|--child-env|--child-env-keys|--print-real-login-transport|--print-webdriver-capabilities --app-binary=PATH|--qa-title-panel=TITLE|--qa-title-panel-ready=TITLE [--required-panel=PANEL]|--qa-title-ready=TITLE|--qa-title-attention-ready=TITLE|--qa-window-state-ready=PATH|--qa-title-send-ready=TITLE|--qa-title-ready-require-recovered=TITLE|--run [--real-login-from-stdin] [--qa-profile=NAME] [--allow-empty-timeline] [--artifact-dir=PATH] [--timeout-ms=MS]"
  );
}
