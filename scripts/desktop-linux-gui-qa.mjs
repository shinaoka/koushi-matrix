#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { open } from "node:fs/promises";
import { createRequire } from "node:module";
import * as net from "node:net";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { deflateSync } from "node:zlib";

import {
  checkInstalledHomeserver,
  conduitConfig,
  createRoom,
  freePort,
  inviteUser as inviteUserToRoom,
  joinRoom,
  registerUser,
  sendRoomEmoteMessage,
  sendRoomFormattedMessage,
  sendRoomMessage,
  sendRoomNoticeMessage,
  setDisplayName,
  startHomeserver,
  stopProcess,
  tuwunelConfig,
  waitForHomeserver
} from "./lib/local-homeserver-qa.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const desktopPackageRequire = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const pngCrc32Table = Array.from({ length: 256 }, (_, index) => {
  let value = index;
  for (let bit = 0; bit < 8; bit += 1) {
    value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  }
  return value >>> 0;
});
const checks = [
  "scenario signed-out",
  "scenario local-login",
  "scenario local-send",
  "scenario local-create-room",
  "scenario local-create-space",
  "scenario local-logout-relogin",
  "scenario local-spaces-nav",
  "scenario local-invites-dm",
  "scenario local-reply",
  "scenario local-media",
  "scenario local-image-compression",
  "scenario local-room-tags",
  "scenario local-room-management",
  "scenario local-activity",
  "scenario local-explore",
  "scenario local-message-actions",
  "scenario local-pins",
  "scenario local-message-types",
  "scenario local-composer",
  "scenario local-scheduled-send",
  "scenario local-timeline-navigation",
  "scenario local-rich-formatting",
  "scenario local-alias",
  "scenario local-cjk",
  "scenario local-settings",
  "scenario local-e2ee-key-management",
  "verify local-settings trust section",
  "verify local-e2ee-key-management tokens",
  "verify Xvfb virtual display",
  "verify tauri-driver and WebKitWebDriver",
  "verify debug Tauri build",
  "drive WebdriverIO session",
  "exercise real IPC and DOM smoke",
  "optional local homeserver login via FIFO",
  "clean process teardown"
];

const args = new Set(process.argv.slice(2));
const guiScenario = optionValue("--scenario") ?? "signed-out";
const serverOption = optionValue("--server") ?? "conduit";
const qaProfile = optionValue("--qa-profile");
const realLoginFromStdin = args.has("--real-login-from-stdin");
const allowEmptyTimeline = args.has("--allow-empty-timeline");
const artifactRoot = resolveArtifactRoot(optionValue("--artifact-dir"));
const timeoutMs = Number(optionValue("--timeout-ms") ?? "120000");

if (args.has("--print-artifact-root")) {
  console.log(artifactRoot);
  process.exit(0);
}

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
  for (const key of Object.keys(childEnvironment("/tmp/koushi-desktop-linux-gui-qa")).sort()) {
    console.log(key);
  }
  process.exit(0);
}

if (args.has("--child-env")) {
  for (const [key, value] of Object.entries(
    childEnvironment(qaDataDirForRun("/tmp/koushi-desktop-linux-gui-qa"))
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

const TIMELINE_NAVIGATION_SEED_MESSAGE_COUNT = 24;
const TIMELINE_NAVIGATION_SEED_LINE_COUNT = 12;

if (args.has("--run")) {
  await run();
  process.exit(0);
}

printUsage();

async function run() {
  if (guiScenario === "signed-out") {
    await runSignedOutScenario();
    return;
  }
  if (guiScenario === "local-login") {
    await runLocalLoginScenario();
    return;
  }
  if (guiScenario === "local-send") {
    await runLocalSendScenario();
    return;
  }
  if (guiScenario === "local-create-room") {
    await runLocalCreateRoomScenario();
    return;
  }
  if (guiScenario === "local-create-space") {
    await runLocalCreateSpaceScenario();
    return;
  }
  if (guiScenario === "local-logout-relogin") {
    await runLocalLogoutReloginScenario();
    return;
  }
  if (guiScenario === "local-spaces-nav") {
    await runLocalSpacesNavScenario();
    return;
  }
  if (guiScenario === "local-invites-dm") {
    await runLocalInvitesDmScenario();
    return;
  }
  if (guiScenario === "local-reply") {
    await runLocalReplyScenario();
    return;
  }
  if (guiScenario === "local-media") {
    await runLocalMediaScenario();
    return;
  }
  if (guiScenario === "local-image-compression") {
    await runLocalImageCompressionScenario();
    return;
  }
  if (guiScenario === "local-room-tags") {
    await runLocalRoomTagsScenario();
    return;
  }
  if (guiScenario === "local-room-management") {
    await runLocalRoomManagementScenario();
    return;
  }
  if (guiScenario === "local-activity") {
    await runLocalActivityScenario();
    return;
  }
  if (guiScenario === "local-explore") {
    await runLocalExploreScenario();
    return;
  }
  if (guiScenario === "local-message-actions") {
    await runLocalMessageActionsScenario();
    return;
  }
  if (guiScenario === "local-pins") {
    await runLocalPinsScenario();
    return;
  }
  if (guiScenario === "local-message-types") {
    await runLocalMessageTypesScenario();
    return;
  }
  if (guiScenario === "local-composer") {
    await runLocalComposerScenario();
    return;
  }
  if (guiScenario === "local-scheduled-send") {
    await runLocalScheduledSendScenario();
    return;
  }
  if (guiScenario === "local-timeline-navigation") {
    await runLocalTimelineNavigationScenario();
    return;
  }
  if (guiScenario === "local-rich-formatting") {
    await runLocalRichFormattingScenario();
    return;
  }
  if (guiScenario === "local-alias") {
    await runLocalAliasScenario();
    return;
  }
  if (guiScenario === "local-cjk") {
    await runLocalCjkScenario();
    return;
  }
  if (guiScenario === "local-settings") {
    await runLocalSettingsScenario();
    return;
  }
  if (guiScenario === "local-e2ee-key-management") {
    await runLocalE2eeKeyManagementScenario();
    return;
  }
  throw new Error(`unsupported --scenario: ${guiScenario}`);
}

async function runSignedOutScenario() {
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
  const driverPort = await freePort();
  const nativePort = await freePort();
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
    const appBinary = await ensureAppBinary({ cwd: desktopDir, env: buildEnv, logPath });
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

    console.log("run_dir=artifact");
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

async function runLocalLoginScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_login=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalSendScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    const message = `Koushi GUI QA ${timestamp()}`;
    await composer.click();
    await composer.setValue(message);
    await session.browser.keys("Enter");
    await waitForLocalSendSuccess(session.browser, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_send=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalCreateRoomScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const baselineRooms = parseQaTitle(
      await session.browser.execute(() => document.title)
    ).rooms;

    const createButton = await session.browser.$('button[aria-label="Create room"]');
    await createButton.waitForDisplayed({ timeout: timeoutMs });
    await createButton.click();
    const nameInput = await session.browser.$('input[aria-label="Room name"]');
    await nameInput.waitForDisplayed({ timeout: timeoutMs });
    await nameInput.setValue(`QA Room ${timestamp()}`);
    const submit = await session.browser.$('button[aria-label="Submit create room"]');
    await submit.click();

    await waitForQaTitle(
      session.browser,
      (status) => status.rooms > baselineRooms,
      timeoutMs,
      "local GUI create room"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_create_room=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalCreateSpaceScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const baselineSpaces = parseQaTitle(
      await session.browser.execute(() => document.title)
    ).spaces;

    const createButton = await session.browser.$('button[aria-label="Create space"]');
    await createButton.waitForDisplayed({ timeout: timeoutMs });
    await createButton.click();
    const nameInput = await session.browser.$('input[aria-label="Space name"]');
    await nameInput.waitForDisplayed({ timeout: timeoutMs });
    await nameInput.setValue(`QA Space ${timestamp()}`);
    const submit = await session.browser.$('button[aria-label="Submit create space"]');
    await submit.click();

    await waitForQaTitle(
      session.browser,
      (status) => status.spaces > baselineSpaces,
      timeoutMs,
      "local GUI create space"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_create_space=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalLogoutReloginScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    await requestQaLogout(session.qaControlPipePath);
    await waitForSignedOutTitle(session.browser, timeoutMs);
    await waitForAuthScreen(session.browser, timeoutMs);
    console.log("gui_local_logout=ok");

    await submitLoginForm(session.browser, session.credentials, timeoutMs);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_relogin=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalSpacesNavScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const baselineSpaces = parseQaTitle(
      await session.browser.execute(() => document.title)
    ).spaces;
    const spaceName = `QA Nav Space ${safeTimestamp()}`;
    const createButton = await session.browser.$('button[aria-label="Create space"]');
    await createButton.waitForDisplayed({ timeout: timeoutMs });
    await createButton.click();
    const nameInput = await session.browser.$('input[aria-label="Space name"]');
    await nameInput.waitForDisplayed({ timeout: timeoutMs });
    await nameInput.setValue(spaceName);
    const submit = await session.browser.$('button[aria-label="Submit create space"]');
    await submit.click();
    await waitForQaTitle(
      session.browser,
      (status) => status.spaces > baselineSpaces,
      timeoutMs,
      "local GUI spaces navigation create"
    );
    await waitForWorkspaceButton(session.browser, spaceName, timeoutMs, "created space");

    await clickWorkspaceButton(session.browser, "Home", timeoutMs, "local GUI spaces home");
    await waitForWorkspaceActive(session.browser, "Home", true, timeoutMs, "local GUI spaces home");
    console.log("gui_local_spaces_home=ok");

    await clickWorkspaceButton(session.browser, spaceName, timeoutMs, "local GUI spaces select");
    await waitForWorkspaceActive(
      session.browser,
      spaceName,
      true,
      timeoutMs,
      "local GUI spaces select"
    );
    console.log("gui_local_spaces_nav=ok");

    const spaceInfo = await session.browser.$('button[aria-label="Space info and settings"]');
    await spaceInfo.waitForDisplayed({ timeout: timeoutMs });
    await spaceInfo.click();
    await waitForQaTitle(
      session.browser,
      (status) => status.panel === "spaceInfo",
      timeoutMs,
      "local GUI spaces info panel"
    );
    await waitForDocumentText(
      session.browser,
      [spaceName],
      timeoutMs,
      "local GUI spaces info panel"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_spaces_info=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalInvitesDmScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const inviteRoom = await createRoom(session.credentials.homeserver, session.helperAccessToken, {
      name: session.seedInviteRoomName
    });
    if (!inviteRoom.room_id) {
      throw new Error("local GUI invite setup did not return a room id");
    }
    await inviteUserToRoom(
      session.credentials.homeserver,
      session.helperAccessToken,
      inviteRoom.room_id,
      session.primaryUserId
    );

    const invitesButton = await session.browser.$('button[aria-label="Invites"]');
    await invitesButton.waitForDisplayed({ timeout: timeoutMs });
    await invitesButton.click();

    const baselineRooms = parseQaTitle(await session.browser.execute(() => document.title)).rooms;
    const acceptButton = await session.browser.$('button[aria-label="Accept invite"]');
    await acceptButton.waitForDisplayed({ timeout: timeoutMs });
    await acceptButton.click();
    await waitForQaTitle(
      session.browser,
      (status) => status.rooms > baselineRooms,
      timeoutMs,
      "local GUI invite accept"
    );
    await waitForDocumentText(
      session.browser,
      ["No pending invites"],
      timeoutMs,
      "local GUI invite accept"
    );

    const baselineDmCount = await elementCount(session.browser, '.room-item[data-room-kind="dm"]');
    const newDmButton = await session.browser.$('main[aria-labelledby="invites-title"] button[aria-label="New DM"]');
    await newDmButton.waitForDisplayed({ timeout: timeoutMs });
    await newDmButton.click();
    const userIdInput = await session.browser.$('input[aria-label="Matrix user ID"]');
    await userIdInput.waitForDisplayed({ timeout: timeoutMs });
    await userIdInput.setValue(session.dmTargetUserId);
    const startDmButton = await session.browser.$('button[aria-label="Start DM"]');
    await startDmButton.click();
    await waitForElementCountGreaterThan(
      session.browser,
      '.room-item[data-room-kind="dm"]',
      baselineDmCount,
      timeoutMs,
      "local GUI start DM"
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_invite_accept=ok");
    console.log("gui_local_dm_start=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalReplyScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    // A reply needs a real, server-acked event to target. Send one first so a
    // timeline row with a reply affordance exists.
    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(`QA reply root ${timestamp()}`);
    await session.browser.keys("Enter");
    await waitForLocalSendSuccess(session.browser, timeoutMs);

    // The reply action sits in a hover-revealed `.message-actions` container
    // (opacity:0 until `.message:hover`/`:focus-within`), so move the pointer
    // over it before interacting. Then open reply mode and confirm the composer
    // surfaced the Rust-backed reply state (Cancel reply affordance).
    const replyButton = await session.browser.$('[aria-label="Reply to message"]');
    await replyButton.waitForExist({ timeout: timeoutMs });
    await replyButton.moveTo();
    await replyButton.waitForDisplayed({ timeout: timeoutMs });
    await replyButton.click();
    const cancelReply = await session.browser.$('[aria-label="Cancel reply"]');
    await cancelReply.waitForDisplayed({ timeout: timeoutMs });

    // Send the reply and wait for it to land (a new timeline row, or a
    // `data-reply="true"` row when the reply relation is surfaced).
    const baselineMessages = await session.browser.execute(
      () => document.querySelectorAll(".message").length
    );
    await composer.click();
    await composer.setValue(`QA reply body ${timestamp()}`);
    await session.browser.keys("Enter");
    await waitForReplyLanded(session.browser, baselineMessages, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_reply=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalMediaScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await waitForQaTitle(
      session.browser,
      (status) => status.timeline_room === true,
      timeoutMs,
      "local GUI media timeline room"
    );

    const baselineMediaRows = await elementCount(session.browser, ".message-media");
    const filename = `qa-media-${safeTimestamp()}.txt`;
    const caption = `QA media caption ${safeTimestamp()}`;
    const fixturePath = join(session.runDir, filename);
    writeFileSync(fixturePath, "Koushi Linux GUI media fixture\n", "utf8");
    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    const fileInputSelector = 'input[type="file"][aria-label="Attach file input"]';
    await setSyntheticFileInput(
      session.browser,
      fileInputSelector,
      fixturePath,
      filename,
      "text/plain",
      "Koushi Linux GUI media fixture"
    );
    await waitForDocumentText(
      session.browser,
      [filename],
      timeoutMs,
      "local GUI staged media preview"
    );
    const captionInput = await session.browser.$(`input[aria-label="Caption for ${filename}"]`);
    await captionInput.waitForDisplayed({ timeout: timeoutMs });
    await setTextInputValueByLabel(session.browser, caption, `Caption for ${filename}`);
    await waitForInputValue(
      session.browser,
      `Caption for ${filename}`,
      caption,
      timeoutMs,
      "local GUI media staging caption"
    );
    const stagedMediaRows = await elementCount(session.browser, ".message-media");
    if (stagedMediaRows !== baselineMediaRows) {
      throw new Error(
        `local GUI media staged attachment sent before Send: baseline=${baselineMediaRows} observed=${stagedMediaRows}`
      );
    }
    console.log("gui_local_media_stage=ok");
    const sendButton = await session.browser.$('button[aria-label="Send"]');
    await sendButton.waitForDisplayed({ timeout: timeoutMs });
    await sendButton.click();
    await waitForElementCountGreaterThan(
      session.browser,
      ".message-media",
      baselineMediaRows,
      timeoutMs,
      "local GUI media render"
    );
    await waitForDocumentText(
      session.browser,
      [filename, caption],
      timeoutMs,
      "local GUI media caption render"
    );

    const downloadButton = await session.browser.$(`button[aria-label="Download ${filename}"]`);
    await downloadButton.waitForDisplayed({ timeout: timeoutMs });
    await downloadButton.click();
    await waitForQaTitle(
      session.browser,
      (status) => status.errors === 0,
      timeoutMs,
      "local GUI media download"
    );

    const galleryButton = await session.browser.$('button[aria-label="Open media gallery"]');
    await galleryButton.waitForDisplayed({ timeout: timeoutMs });
    await galleryButton.click();
    const galleryRegion = await session.browser.$('[role="region"][aria-label="Room media gallery"]');
    await galleryRegion.waitForDisplayed({ timeout: timeoutMs });
    await clickVisibleButtonByAriaLabel(
      session.browser,
      `Open ${filename}`,
      timeoutMs,
      "local GUI media gallery item",
      '[role="region"][aria-label="Room media gallery"]'
    );
    const mediaViewer = await session.browser.$('[role="dialog"][aria-label="Media viewer"]');
    await mediaViewer.waitForDisplayed({ timeout: timeoutMs });
    await waitForDocumentText(
      session.browser,
      [filename],
      timeoutMs,
      "local GUI media viewer"
    );
    const closeViewer = await session.browser.$('button[aria-label="Close media viewer"]');
    await closeViewer.waitForDisplayed({ timeout: timeoutMs });
    await closeViewer.click();

    await recordLocalGuiEvidence(session);
    console.log("gui_local_media=ok");
    console.log("gui_local_media_caption=ok");
    console.log("gui_local_media_viewer=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalImageCompressionScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await waitForQaTitle(
      session.browser,
      (status) => status.timeline_room === true,
      timeoutMs,
      "local GUI image compression timeline room"
    );

    const userSettings = await session.browser.$('button[aria-label="User settings"]');
    await userSettings.waitForDisplayed({ timeout: timeoutMs });
    await userSettings.click();
    const alwaysCompressionSelector =
      '//section[@aria-label="Media"]//button[normalize-space()="Always"]';
    const alwaysCompression = await session.browser.$(alwaysCompressionSelector);
    await alwaysCompression.waitForDisplayed({ timeout: timeoutMs });
    await alwaysCompression.click();
    await waitForElementAttribute(
      session.browser,
      alwaysCompressionSelector,
      "aria-pressed",
      "true",
      timeoutMs,
      "image compression Always setting"
    );
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);

    const baselineMediaRows = await elementCount(session.browser, ".message-media");
    const pngFilename = `qa-image-compress-${safeTimestamp()}.png`;
    const jpgFilename = pngFilename.replace(/\.png$/, ".jpg");
    const fixturePath = join(session.runDir, pngFilename);
    writePngFixture(fixturePath, 3000, 10);
    const fileInputSelector = 'input[type="file"][aria-label="Attach file input"]';
    await setSyntheticFileInput(
      session.browser,
      fileInputSelector,
      fixturePath,
      pngFilename,
      "image/png",
      { base64: readFileSync(fixturePath).toString("base64") }
    );
    await waitForDocumentText(
      session.browser,
      [pngFilename],
      timeoutMs,
      "local GUI image compression staged preview"
    );
    const stagedMediaRows = await elementCount(session.browser, ".message-media");
    if (stagedMediaRows !== baselineMediaRows) {
      throw new Error(
        `local GUI image compression sent before Send: baseline=${baselineMediaRows} observed=${stagedMediaRows}`
      );
    }

    const sendButton = await session.browser.$('button[aria-label="Send"]');
    await sendButton.waitForDisplayed({ timeout: timeoutMs });
    await sendButton.click();
    await waitForElementCountGreaterThan(
      session.browser,
      ".message-media",
      baselineMediaRows,
      timeoutMs,
      "local GUI compressed image render"
    );
    await waitForCompressedImageMedia(
      session.browser,
      {
        filename: jpgFilename,
        mimetype: "image/jpeg",
        dimensions: "2048x7"
      },
      timeoutMs
    );
    if ((await elementCount(session.browser, 'div[role="dialog"][aria-label="Compress image"]')) > 0) {
      throw new Error("local GUI Always compression unexpectedly opened the ask dialog");
    }

    await recordLocalGuiEvidence(session);
    console.log("gui_local_image_compress=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalRoomTagsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const roomName = "QA Seed Room";
    await waitForRoomInSection(
      session.browser,
      "rooms",
      roomName,
      true,
      timeoutMs,
      "local GUI room tag baseline"
    );
    await waitForRoomInSection(
      session.browser,
      "favourites",
      roomName,
      false,
      timeoutMs,
      "local GUI room tag baseline"
    );

    await openRoomContextMenu(session.browser, "rooms", roomName);
    await clickMenuItemByText(session.browser, "Add to Favourites", timeoutMs);
    await waitForRoomInSection(
      session.browser,
      "favourites",
      roomName,
      true,
      timeoutMs,
      "local GUI room tag set"
    );
    await waitForRoomInSection(
      session.browser,
      "rooms",
      roomName,
      false,
      timeoutMs,
      "local GUI room tag set"
    );
    console.log("gui_local_room_tag_set=ok");

    await openRoomContextMenu(session.browser, "favourites", roomName);
    await clickMenuItemByText(session.browser, "Remove from Favourites", timeoutMs);
    await waitForRoomInSection(
      session.browser,
      "rooms",
      roomName,
      true,
      timeoutMs,
      "local GUI room tag remove"
    );
    await waitForRoomInSection(
      session.browser,
      "favourites",
      roomName,
      false,
      timeoutMs,
      "local GUI room tag remove"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_room_tag_removed=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalRoomManagementScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const roomInfoButton = await session.browser.$('button[aria-label="Room info"]');
    await roomInfoButton.waitForDisplayed({ timeout: timeoutMs });
    await roomInfoButton.click();

    const topicInput = await session.browser.$('textarea[aria-label="Room topic"]');
    await topicInput.waitForDisplayed({ timeout: timeoutMs });
    await topicInput.setValue(session.roomManagementTopic);
    const saveTopicButton = await session.browser.$("//button[normalize-space()='Save topic']");
    await saveTopicButton.waitForDisplayed({ timeout: timeoutMs });
    await saveTopicButton.click();
    await waitForRoomManagementTopic(
      session.browser,
      session.roomManagementTopic,
      timeoutMs,
      "local GUI room management topic"
    );
    console.log("gui_local_room_topic=ok");

    await waitForElementCount(
      session.browser,
      ".room-member-row",
      1,
      timeoutMs,
      "local GUI room management member baseline"
    );
    const roleSelect = await session.browser.$('select[aria-label^="Member role for"]');
    await roleSelect.waitForDisplayed({ timeout: timeoutMs });
    await roleSelect.selectByAttribute("value", "50");
    await waitForRoomMemberRole(
      session.browser,
      "Moderator",
      "50",
      timeoutMs,
      "local GUI room management role"
    );
    console.log("gui_local_room_role=ok");

    const kickButton = await session.browser.$('.room-member-row button[data-action="kick"]');
    await kickButton.waitForDisplayed({ timeout: timeoutMs });
    await kickButton.click();
    await waitForElementCount(
      session.browser,
      ".room-member-row",
      0,
      timeoutMs,
      "local GUI room management kick"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_room_kick=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalActivityScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const activityButton = await session.browser.$('button[aria-label="Activity"]');
    await activityButton.waitForDisplayed({ timeout: timeoutMs });
    await activityButton.click();
    const activityMain = await session.browser.$('main[aria-labelledby="activity-title"]');
    await activityMain.waitForDisplayed({ timeout: timeoutMs });
    console.log("gui_local_activity_open=ok");

    const unreadTabSelector = "//button[@role='tab' and normalize-space()='Unread']";
    const unreadTab = await session.browser.$(unreadTabSelector);
    await unreadTab.waitForDisplayed({ timeout: timeoutMs });
    await unreadTab.click();
    await waitForElementAttribute(
      session.browser,
      unreadTabSelector,
      "aria-selected",
      "true",
      timeoutMs,
      "local GUI activity unread tab"
    );
    console.log("gui_local_activity_unread_tab=ok");

    const recentTabSelector = "//button[@role='tab' and normalize-space()='Recent']";
    const recentTab = await session.browser.$(recentTabSelector);
    await recentTab.waitForDisplayed({ timeout: timeoutMs });
    await recentTab.click();
    await waitForElementAttribute(
      session.browser,
      recentTabSelector,
      "aria-selected",
      "true",
      timeoutMs,
      "local GUI activity recent tab"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_activity_recent_tab=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalExploreScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const baselineRooms = parseQaTitle(await session.browser.execute(() => document.title)).rooms;
    const exploreButton = await session.browser.$('button[aria-label="Explore"]');
    await exploreButton.waitForDisplayed({ timeout: timeoutMs });
    await exploreButton.click();

    const searchInput = await session.browser.$('input[aria-label="Search public rooms"]');
    await searchInput.waitForDisplayed({ timeout: timeoutMs });
    await searchInput.setValue(session.directoryRoomName);
    const searchButton = await session.browser.$('button[aria-label="Search public rooms"]');
    await searchButton.click();

    await waitForDocumentText(
      session.browser,
      [session.directoryRoomName],
      timeoutMs,
      "local GUI public directory query"
    );
    console.log("gui_local_explore_query=ok");

    const joinButton = await session.browser.$(
      `button[aria-label=${JSON.stringify(`Join ${session.directoryRoomName}`)}]`
    );
    await joinButton.waitForDisplayed({ timeout: timeoutMs });
    await joinButton.click();

    await waitForQaTitle(
      session.browser,
      (status) => status.rooms > baselineRooms,
      timeoutMs,
      "local GUI public directory join"
    );
    await waitForRoomInSection(
      session.browser,
      "rooms",
      session.directoryRoomName,
      true,
      timeoutMs,
      "local GUI public directory joined room"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_explore_join=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalMessageActionsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForQaTitle(
      session.browser,
      (status) =>
        status.timeline_room === true &&
        status.timeline_subscribed === true,
      timeoutMs,
      "local GUI message actions timeline room"
    );
    await waitForTimelineViewMounted(session.browser, timeoutMs);
    await sleep(1000);
    const seedBaselineMessages = await elementCount(session.browser, ".message");
    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue("QA message action seed");
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI message actions seed");
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      seedBaselineMessages,
      timeoutMs,
      "local GUI message actions seed message render"
    );

    const actionButton = await waitForLatestMessageActionButton(session.browser, timeoutMs);
    await actionButton.moveTo();
    await actionButton.waitForDisplayed({ timeout: timeoutMs });
    await actionButton.click();
    await clickVisibleMenuItemByText(session.browser, "View source", timeoutMs);
    await waitForMessageSourceDialog(session.browser, timeoutMs);
    console.log("gui_local_message_source=ok");

    const closeSource = await session.browser.$('button[aria-label="Close message source"]');
    await closeSource.waitForDisplayed({ timeout: timeoutMs });
    await closeSource.click();

    const baselineMessages = await elementCount(session.browser, ".message[data-event-id]");
    const forwardActionButton = await waitForLatestMessageActionButton(session.browser, timeoutMs);
    await forwardActionButton.moveTo();
    await forwardActionButton.waitForDisplayed({ timeout: timeoutMs });
    await forwardActionButton.click();
    await clickVisibleMenuItemByText(session.browser, "Forward", timeoutMs);
    await clickVisibleMenuItemByText(session.browser, "QA Seed Room", timeoutMs);
    await waitForElementCountGreaterThan(
      session.browser,
      ".message[data-event-id]",
      baselineMessages,
      timeoutMs,
      "local GUI message forward"
    );
    console.log("gui_local_message_forward=ok");

    const redactedBaselineMessages = await elementCount(
      session.browser,
      '.message[data-redacted="true"]'
    );
    const hideRedactedBody = "QA hide redacted seed";
    const hideRedactedBaselineMessages = await elementCount(session.browser, ".message");
    const hideRedactedComposer = await session.browser.$('textarea[aria-label="Message composer"]');
    await hideRedactedComposer.waitForDisplayed({ timeout: timeoutMs });
    await hideRedactedComposer.click();
    await hideRedactedComposer.setValue(hideRedactedBody);
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(
      session.browser,
      timeoutMs,
      "local GUI hide redacted seed"
    );
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      hideRedactedBaselineMessages,
      timeoutMs,
      "local GUI hide redacted seed message render"
    );
    await clickLatestMessageRedactButtonByText(session.browser, hideRedactedBody, timeoutMs);
    await waitForElementCountGreaterThan(
      session.browser,
      '.message[data-redacted="true"]',
      redactedBaselineMessages,
      timeoutMs,
      "local GUI redacted message render"
    );
    await waitForDocumentText(
      session.browser,
      ["Message redacted"],
      timeoutMs,
      "local GUI redacted message placeholder"
    );

    const userSettings = await session.browser.$('button[aria-label="User settings"]');
    await userSettings.waitForDisplayed({ timeout: timeoutMs });
    await userSettings.click();
    const hideDeletedToggleSelector =
      '//button[@role="switch" and @aria-label="Hide deleted messages"]';
    const hideDeletedToggle = await session.browser.$(hideDeletedToggleSelector);
    await hideDeletedToggle.waitForDisplayed({ timeout: timeoutMs });
    await waitForElementAttribute(
      session.browser,
      hideDeletedToggleSelector,
      "aria-checked",
      "false",
      timeoutMs,
      "hide redacted setting before toggle"
    );
    await hideDeletedToggle.click();
    await waitForElementAttribute(
      session.browser,
      hideDeletedToggleSelector,
      "aria-checked",
      "true",
      timeoutMs,
      "hide redacted setting after toggle"
    );
    await waitForElementCount(
      session.browser,
      '.message[data-redacted="true"]',
      redactedBaselineMessages,
      timeoutMs,
      "local GUI hide redacted projection"
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_hide_redacted=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalPinsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForQaTitle(
      session.browser,
      (status) => status.timeline_room === true && status.timeline_subscribed === true,
      timeoutMs,
      "local GUI pins timeline room"
    );
    await waitForTimelineViewMounted(session.browser, timeoutMs);

    const row = await waitForLatestEventMessageRow(
      session.browser,
      timeoutMs,
      "local GUI pin target"
    );
    await row.moveTo();
    await clickVisibleButtonByAriaLabelInElement(
      row,
      "Pin message",
      timeoutMs,
      "local GUI pin message"
    );
    await waitForPinnedRegionVisible(session.browser, timeoutMs, "local GUI pin set");
    console.log("gui_local_pin_set=ok");

    await row.moveTo();
    await clickVisibleButtonByAriaLabelInElement(
      row,
      "Unpin message",
      timeoutMs,
      "local GUI unpin message"
    );
    await waitForPinnedRegionCleared(session.browser, timeoutMs, "local GUI pin clear");

    await recordLocalGuiEvidence(session);
    console.log("gui_local_pin_removed=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalMessageTypesScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForTimelineViewMounted(session.browser, timeoutMs);

    await waitForTimelineViewMounted(session.browser, timeoutMs);
    const baselineEmotes = await elementCount(session.browser, '.message[data-message-kind="emote"]');
    const emoteBody = `waves ${safeTimestamp()}`;
    await sendRoomEmoteMessage(
      session.credentials.homeserver,
      session.helperAccessToken,
      session.seedRoomId,
      emoteBody,
      `qa-emote-${safeTimestamp()}`
    );
    await waitForElementCountGreaterThan(
      session.browser,
      '.message[data-message-kind="emote"]',
      baselineEmotes,
      timeoutMs,
      "local GUI emote render"
    );
    await waitForDocumentText(session.browser, [emoteBody], timeoutMs, "local GUI emote text");
    console.log("gui_local_emote=ok");

    const noticeBody = `QA notice ${safeTimestamp()}`;
    const baselineNotices = await elementCount(
      session.browser,
      '.message[data-message-kind="notice"]'
    );
    await sendRoomNoticeMessage(
      session.credentials.homeserver,
      session.helperAccessToken,
      session.seedRoomId,
      noticeBody,
      `qa-notice-${safeTimestamp()}`
    );
    await waitForElementCountGreaterThan(
      session.browser,
      '.message[data-message-kind="notice"]',
      baselineNotices,
      timeoutMs,
      "local GUI notice render"
    );
    await waitForDocumentText(session.browser, [noticeBody], timeoutMs, "local GUI notice text");
    console.log("gui_local_notice=ok");

    const spoilerSecret = `secret-${safeTimestamp()}`;
    const spoilerBody = `QA spoiler keep ${spoilerSecret} hidden`;
    const baselineSpoilers = await elementCount(session.browser, ".message-spoiler");
    await sendRoomFormattedMessage(
      session.credentials.homeserver,
      session.helperAccessToken,
      session.seedRoomId,
      spoilerBody,
      `QA spoiler keep <span data-mx-spoiler="reason">${spoilerSecret}</span> hidden`,
      `qa-spoiler-${safeTimestamp()}`
    );
    await waitForElementCountGreaterThan(
      session.browser,
      ".message-spoiler",
      baselineSpoilers,
      timeoutMs,
      "local GUI spoiler render"
    );
    const leakedBeforeReveal = await session.browser.execute(
      (secret) => (document.body.textContent ?? "").includes(secret),
      spoilerSecret
    );
    if (leakedBeforeReveal) {
      throw new Error("local GUI spoiler text was visible before reveal");
    }
    const spoilerButton = await session.browser.$('.message-spoiler[data-revealed="false"]');
    await spoilerButton.waitForDisplayed({ timeout: timeoutMs });
    await spoilerButton.click();
    await waitForDocumentText(
      session.browser,
      [spoilerSecret],
      timeoutMs,
      "local GUI spoiler reveal"
    );
    console.log("gui_local_spoiler=ok");

    await recordLocalGuiEvidence(session);
    console.log("gui_local_message_types=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalComposerScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue("@qa");

    const mentionOption = await session.browser.$('button[role="option"]');
    await mentionOption.waitForDisplayed({ timeout: timeoutMs });
    await mentionOption.click();
    await waitForElementCountGreaterThan(
      session.browser,
      ".composer-mention-pills .mention-pill",
      0,
      timeoutMs,
      "local GUI mention pill"
    );
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI mention send");
    console.log("gui_local_mention=ok");

    await composer.click();
    await composer.setValue("world");
    await selectComposerText(session.browser);
    const boldButton = await session.browser.$('button[aria-label="Bold"]');
    await boldButton.waitForDisplayed({ timeout: timeoutMs });
    await boldButton.click();
    await waitForTextareaValue(
      session.browser,
      'textarea[aria-label="Message composer"]',
      "**world**",
      timeoutMs,
      "local GUI bold markdown"
    );
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI markdown send");
    console.log("gui_local_markdown=ok");

    await composer.click();
    await composer.setValue("/me waves");
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI slash send");
    await recordLocalGuiEvidence(session);
    console.log("gui_local_slash=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalScheduledSendScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(`QA scheduled body ${safeTimestamp()}`);

    const sendLater = await session.browser.$('button[aria-label="Send later"]');
    await sendLater.waitForDisplayed({ timeout: timeoutMs });
    await sendLater.click();

    const scheduleInput = await session.browser.$('input[aria-label="Scheduled send time"]');
    await scheduleInput.waitForDisplayed({ timeout: timeoutMs });
    const scheduledValue = await localDatetimeInputValue(
      session.browser,
      Date.now() + 24 * 60 * 60_000
    );
    await setDatetimeLocalValue(session.browser, scheduledValue, "Scheduled send time");
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Schedule send",
      timeoutMs,
      "local GUI scheduled send create"
    );
    await waitForDocumentText(
      session.browser,
      ["Scheduled messages", "Local fallback"],
      timeoutMs,
      "local GUI scheduled send create"
    );
    await waitForTextareaValue(
      session.browser,
      'textarea[aria-label="Message composer"]',
      "",
      timeoutMs,
      "local GUI scheduled send draft clear"
    );
    console.log("gui_local_scheduled_create=ok");

    const editButton = await session.browser.$('button[aria-label="Edit scheduled send"]');
    await editButton.waitForDisplayed({ timeout: timeoutMs });
    await editButton.click();
    const editedValue = await localDatetimeInputValue(
      session.browser,
      Date.now() + 48 * 60 * 60_000
    );
    await setDatetimeLocalValue(session.browser, editedValue, "Scheduled send time");
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Save scheduled send",
      timeoutMs,
      "local GUI scheduled send reschedule"
    );
    await waitForElementCount(
      session.browser,
      'section[aria-label="Scheduled messages"]',
      1,
      timeoutMs,
      "local GUI scheduled send reschedule"
    );
    console.log("gui_local_scheduled_reschedule=ok");

    const cancelButton = await session.browser.$('button[aria-label="Cancel scheduled send"]');
    await cancelButton.waitForDisplayed({ timeout: timeoutMs });
    await cancelButton.click();
    await waitForElementCount(
      session.browser,
      'section[aria-label="Scheduled messages"]',
      0,
      timeoutMs,
      "local GUI scheduled send cancel"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_scheduled_cancel=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalTimelineNavigationScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForTimelineViewMounted(session.browser, timeoutMs);
    await waitForTimelineScrollable(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation seed"
    );

    await driveTimelineToBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation initial bottom"
    );
    const baselineMessages = await elementCount(session.browser, ".message");
    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(`QA timeline navigation baseline ${safeTimestamp()}`);
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation baseline"
    );
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      baselineMessages,
      timeoutMs,
      "local GUI timeline navigation baseline render"
    );
    await driveTimelineToBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation baseline bottom"
    );

    await scrollTimelineToTop(session.browser);
    await waitForTimelineAwayFromBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation top viewport"
    );
    const beforeUnreadMessages = await elementCount(session.browser, ".message");
    let dateJumpEventId = null;
    for (let index = 0; index < 3; index += 1) {
      const response = await sendRoomMessage(
        session.credentials.homeserver,
        session.helperAccessToken,
        session.seedRoomId,
        `QA timeline navigation unread ${index} ${safeTimestamp()}`,
        `qa-timeline-nav-${index}-${safeTimestamp()}`
      );
      dateJumpEventId = response.event_id ?? dateJumpEventId;
    }
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      beforeUnreadMessages,
      timeoutMs,
      "local GUI timeline navigation unread render"
    );

    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Jump to first unread",
      timeoutMs,
      "local GUI timeline navigation first unread"
    );
    await waitForDocumentText(
      session.browser,
      ["Unread messages"],
      timeoutMs,
      "local GUI timeline navigation unread divider"
    );
    console.log("gui_local_timeline_unread_jump=ok");

    await scrollTimelineToTop(session.browser);
    await waitForTimelineAwayFromBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation bottom setup viewport"
    );
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Jump to bottom",
      timeoutMs,
      "local GUI timeline navigation bottom"
    );
    await waitForTimelineScrolledToBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation bottom"
    );
    console.log("gui_local_timeline_bottom_jump=ok");

    if (!dateJumpEventId) {
      throw new Error("local GUI timeline navigation date setup did not capture an event id");
    }
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Jump to date",
      timeoutMs,
      "local GUI timeline navigation date dialog"
    );
    const dateInput = await session.browser.$('input[aria-label="Jump to date"]');
    await dateInput.waitForDisplayed({ timeout: timeoutMs });
    const dateJumpEvent = await getRoomEvent(
      session.credentials.homeserver,
      session.helperAccessToken,
      session.seedRoomId,
      dateJumpEventId
    );
    const localDateValue = await localDatetimeInputValue(
      session.browser,
      dateJumpEvent.origin_server_ts
    );
    await setDatetimeLocalValue(session.browser, localDateValue);
    const dateInputDiagnostics = await timelineDateJumpDiagnostics(
      session.browser,
      localDateValue
    );
    if (
      !dateInputDiagnostics.inputExists ||
      !dateInputDiagnostics.valuePresent ||
      !dateInputDiagnostics.valueMatchesExpected ||
      !dateInputDiagnostics.valid
    ) {
      throw new Error(
        `local GUI timeline navigation date input did not accept the synthetic value. Diagnostics: ${JSON.stringify(
          dateInputDiagnostics
        )}`
      );
    }
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Open date in timeline",
      timeoutMs,
      "local GUI timeline navigation date submit"
    );
    await waitForTimelineFocusedContextReady(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation focused context title"
    );
    await waitForDocumentText(
      session.browser,
      ["Focused context"],
      timeoutMs,
      "local GUI timeline navigation date jump"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_timeline_date_jump=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalAliasScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForTimelineViewMounted(session.browser, timeoutMs);
    await waitForTimelineSenderLabel(
      session.browser,
      session.aliasMemberDisplayName,
      timeoutMs,
      "local GUI alias timeline original label"
    );

    const actionButton = await waitForLatestMessageActionButton(session.browser, timeoutMs);
    await actionButton.moveTo();
    await actionButton.waitForDisplayed({ timeout: timeoutMs });
    await actionButton.click();
    await clickVisibleMenuItemByText(
      session.browser,
      `Set alias for ${session.aliasMemberDisplayName}`,
      timeoutMs
    );
    const aliasInput = await session.browser.$('input[aria-label="Alias"]');
    await aliasInput.waitForDisplayed({ timeout: timeoutMs });
    await aliasInput.setValue(session.aliasLocalDisplayName);
    const saveAliasButton = await session.browser.$("//button[normalize-space()='Save alias']");
    await saveAliasButton.waitForDisplayed({ timeout: timeoutMs });
    await saveAliasButton.click();

    await waitForTimelineSenderLabel(
      session.browser,
      session.aliasLocalDisplayName,
      timeoutMs,
      "local GUI alias timeline set"
    );
    const roomInfoButton = await session.browser.$('button[aria-label="Room info"]');
    await roomInfoButton.waitForDisplayed({ timeout: timeoutMs });
    await roomInfoButton.click();
    await waitForRoomMemberAlias(
      session.browser,
      session.aliasLocalDisplayName,
      session.aliasMemberDisplayName,
      timeoutMs,
      "local GUI alias member set"
    );
    console.log("gui_local_alias_set=ok");

    await clickRoomMemberAliasClear(session.browser, session.aliasLocalDisplayName, timeoutMs);
    await waitForTimelineSenderLabel(
      session.browser,
      session.aliasMemberDisplayName,
      timeoutMs,
      "local GUI alias timeline clear"
    );
    await waitForRoomMemberAlias(
      session.browser,
      session.aliasMemberDisplayName,
      null,
      timeoutMs,
      "local GUI alias member clear"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_alias_clear=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalRichFormattingScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);

    await waitForRichFormattedTimeline(session.browser, session.richFormatted, "pre-wrap", timeoutMs);

    const userSettings = await session.browser.$('button[aria-label="User settings"]');
    await userSettings.waitForDisplayed({ timeout: timeoutMs });
    await userSettings.click();
    const wrapToggleSelector =
      '//button[@role="switch" and @aria-label="Wrap long lines in code blocks"]';
    const wrapToggle = await session.browser.$(wrapToggleSelector);
    await wrapToggle.waitForDisplayed({ timeout: timeoutMs });
    await waitForElementAttribute(
      session.browser,
      wrapToggleSelector,
      "aria-checked",
      "true",
      timeoutMs,
      "code block wrap setting before toggle"
    );
    await wrapToggle.click();
    await waitForElementAttribute(
      session.browser,
      wrapToggleSelector,
      "aria-checked",
      "false",
      timeoutMs,
      "code block wrap setting after toggle"
    );
    await waitForRichFormattedTimeline(session.browser, session.richFormatted, "pre", timeoutMs);

    await recordLocalGuiEvidence(session);
    console.log("gui_local_rich_formatting=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalCjkScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);
    await waitForDocumentText(
      session.browser,
      [session.cjkRoomName],
      timeoutMs,
      "local GUI CJK room name"
    );

    const composer = await session.browser.$('textarea[aria-label="Message composer"]');
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(session.cjkMessageBody);
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI CJK send");
    await waitForDocumentText(
      session.browser,
      [session.cjkMessageBody],
      timeoutMs,
      "local GUI CJK message render"
    );
    await waitForCjkVisualContract(
      session.browser,
      {
        roomName: session.cjkRoomName,
        messageBody: session.cjkMessageBody
      },
      timeoutMs
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_cjk=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalSettingsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    const keyboardSettings = await session.browser.$('button[aria-label="Keyboard settings"]');
    await keyboardSettings.waitForDisplayed({ timeout: timeoutMs });
    await keyboardSettings.click();
    const modEnterButtonSelector =
      "//button[normalize-space()='Ctrl+Enter sends' or normalize-space()='Cmd+Enter sends']";
    const modEnterButton = await session.browser.$(modEnterButtonSelector);
    await modEnterButton.waitForDisplayed({ timeout: timeoutMs });
    await modEnterButton.click();
    await waitForElementAttribute(
      session.browser,
      modEnterButtonSelector,
      "aria-pressed",
      "true",
      timeoutMs,
      "composer shortcut setting"
    );

    const userSettings = await session.browser.$('button[aria-label="User settings"]');
    await userSettings.waitForDisplayed({ timeout: timeoutMs });
    await userSettings.click();
    const darkThemeButton = await session.browser.$("//button[normalize-space()='Dark']");
    await darkThemeButton.waitForDisplayed({ timeout: timeoutMs });
    await darkThemeButton.click();
    await waitForElementAttribute(
      session.browser,
      "//button[normalize-space()='Dark']",
      "aria-pressed",
      "true",
      timeoutMs,
      "dark theme setting"
    );
    await waitForDocumentTheme(session.browser, "dark", timeoutMs);
    await waitForDocumentText(
      session.browser,
      ["Encryption", "Cross-signing", "Key backup", "Identity reset", "Devices"],
      timeoutMs,
      "E2EE trust settings section"
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_settings=ok");
    console.log("gui_local_trust_settings=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function runLocalE2eeKeyManagementScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session.browser, timeoutMs);

    await ensureUserSettingsKeyManagementOpen(session.browser, timeoutMs);

    const keyFilePath = join(session.runDir, "room-keys.txt");
    const recoveryKeyPath = join(session.runDir, "secure-backup-recovery.txt");
    const keyFilePassphrase = `koushi-key-transfer-${safeTimestamp()}`;
    const secureBackupPassphrase = `koushi-desktop-secure-backup-${safeTimestamp()}`;

    await setKeyManagementFormInput(
      session.browser,
      "Room key export",
      "Key export destination",
      keyFilePath
    );
    await setKeyManagementFormInput(
      session.browser,
      "Room key export",
      "Room key passphrase",
      keyFilePassphrase
    );
    await clickKeyManagementFormButton(
      session.browser,
      "Room key export",
      "Export room keys",
      timeoutMs
    );
    await waitForKeyManagementStatus(
      session.browser,
      "room-key-export-state",
      ["Exported", "sessions exported"],
      timeoutMs,
      "local GUI room-key export"
    );
    await waitForFileExists(keyFilePath, timeoutMs, "local GUI room-key export artifact");
    console.log("gui_room_key_export=ok");

    await setKeyManagementFormInput(
      session.browser,
      "Room key import",
      "Key import source",
      keyFilePath
    );
    await setKeyManagementFormInput(
      session.browser,
      "Room key import",
      "Room key passphrase",
      keyFilePassphrase
    );
    await clickKeyManagementFormButton(
      session.browser,
      "Room key import",
      "Import room keys",
      timeoutMs
    );
    await waitForKeyManagementStatus(
      session.browser,
      "room-key-import-state",
      ["imported"],
      timeoutMs,
      "local GUI room-key import"
    );
    console.log("gui_room_key_import=ok");

    await setKeyManagementFormInput(
      session.browser,
      "Secure backup",
      "Secure backup passphrase",
      secureBackupPassphrase
    );
    await setKeyManagementFormInput(
      session.browser,
      "Secure backup",
      "Recovery key destination",
      recoveryKeyPath
    );
    await waitForKeyManagementStatus(
      session.browser,
      "secure-backup-state",
      ["Not set up"],
      timeoutMs,
      "local GUI secure-backup initial status"
    );
    await clickKeyManagementFormButton(
      session.browser,
      "Secure backup",
      "Set up secure backup",
      timeoutMs
    );
    await waitForFileExists(recoveryKeyPath, timeoutMs, "local GUI secure-backup artifact");
    await waitForSecureBackupSetupEvidence(session.browser, timeoutMs);
    console.log("gui_secure_backup_setup=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function waitForQaTitle(browser, predicate, timeout, description) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`${description} reported errors. Last title: ${lastTitle}`);
    }
    if (predicate(status)) {
      return lastTitle;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not reach its expected state. Last title: ${lastTitle}`);
}

async function waitForTimelineFocusedContextReady(browser, timeout, description) {
  const startedAt = Date.now();
  let lastTitle = "";
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`${description} reported errors. Last title: ${lastTitle}`);
    }
    lastDiagnostics = await timelineDateJumpDiagnostics(browser);
    if (
      status.panel === "focusedContext" &&
      (status.focused === "opening" || status.focused === "open")
    ) {
      return lastTitle;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not reach its expected state. Last title: ${lastTitle}. Last diagnostics: ${JSON.stringify(
      lastDiagnostics
    )}`
  );
}

async function waitForElementAttribute(browser, selector, attribute, expected, timeout, description) {
  const startedAt = Date.now();
  let lastValue = "";
  while (Date.now() - startedAt < timeout) {
    const element = await browser.$(selector);
    if (await element.isExisting()) {
      lastValue = (await element.getAttribute(attribute)) ?? "";
      if (lastValue === expected) {
        return;
      }
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not reach ${attribute}=${expected}. Last value: ${lastValue}`
  );
}

async function waitForComposerSendSettled(browser, timeout, description) {
  await waitForTextareaValue(
    browser,
    'textarea[aria-label="Message composer"]',
    "",
    timeout,
    `${description} clear`
  );
  await waitForLocalSendSuccess(browser, timeout);
}

async function waitForTextareaValue(browser, selector, expected, timeout, description) {
  const startedAt = Date.now();
  let lastValue = "";
  while (Date.now() - startedAt < timeout) {
    lastValue = await browser.execute((cssSelector) => {
      const textarea = document.querySelector(cssSelector);
      return textarea instanceof HTMLTextAreaElement ? textarea.value : "";
    }, selector);
    if (lastValue === expected) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not reach expected textarea value. Last value: ${lastValue}`);
}

async function waitForDocumentTheme(browser, expected, timeout) {
  const startedAt = Date.now();
  let lastTheme = "";
  while (Date.now() - startedAt < timeout) {
    lastTheme = await browser.execute(() => document.documentElement.dataset.theme ?? "");
    if (lastTheme === expected) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`document theme did not become ${expected}. Last theme: ${lastTheme}`);
}

async function waitForDocumentText(browser, expectedTexts, timeout, description) {
  const startedAt = Date.now();
  let missing = expectedTexts;
  while (Date.now() - startedAt < timeout) {
    const observed = await browser.execute((texts) => {
      const bodyText = document.body.textContent ?? "";
      return texts.filter((text) => !bodyText.includes(text));
    }, expectedTexts);
    missing = observed;
    if (missing.length === 0) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} missing expected text: ${missing.join(", ")}`);
}

async function ensureUserSettingsKeyManagementOpen(browser, timeout) {
  const expectedTexts = ["Key management", "Room key export", "Room key import", "Secure backup"];
  if (await documentContainsAll(browser, expectedTexts)) {
    return;
  }
  const userSettings = await browser.$('button[aria-label="User settings"]');
  await userSettings.waitForDisplayed({ timeout });
  await userSettings.click();
  try {
    await waitForDocumentText(
      browser,
      expectedTexts,
      timeout,
      "local GUI key-management settings"
    );
  } catch (error) {
    const diagnostics = await safeUserSettingsDiagnostics(browser);
    throw new Error(`${error.message}. Diagnostics: ${JSON.stringify(diagnostics)}`);
  }
}

async function documentContainsAll(browser, expectedTexts) {
  return browser.execute((texts) => {
    const bodyText = document.body.textContent ?? "";
    return texts.every((text) => bodyText.includes(text));
  }, expectedTexts);
}

async function safeUserSettingsDiagnostics(browser) {
  return browser.execute(() => {
    const userSettings = document.querySelector('button[aria-label="User settings"]');
    const active = document.activeElement;
    return {
      title: document.title,
      bodyChildCount: document.body.childElementCount,
      bodyTextLength: document.body.textContent?.length ?? 0,
      hasAuthScreen: document.querySelector('[data-testid="auth-screen"]') !== null,
      hasMain: document.querySelector('main[aria-label="Conversation timeline"]') !== null,
      hasUserSettingsButton: userSettings !== null,
      hasKeyManagementHeading: Array.from(document.querySelectorAll("h3,h4")).some(
        (element) => (element.textContent ?? "").includes("Key management")
      ),
      keyManagementForms: document.querySelectorAll(
        'form[aria-label="Room key export"],form[aria-label="Room key import"],form[aria-label="Secure backup"]'
      ).length,
      settingsSections: document.querySelectorAll(".settings-section").length,
      activeElement:
        active?.getAttribute("aria-label") ??
        active?.getAttribute("data-testid") ??
        active?.tagName ??
        null
    };
  });
}

async function setKeyManagementFormInput(browser, formLabel, fieldLabel, value) {
  const selector = keyManagementFormInputXpath(formLabel, fieldLabel);
  const input = await browser.$(selector);
  await input.waitForDisplayed({ timeout: timeoutMs });
  await input.setValue(value);
}

async function clickKeyManagementFormButton(browser, formLabel, buttonLabel, timeout) {
  const selector = `//form[@aria-label=${xpathLiteral(
    formLabel
  )}]//button[normalize-space()=${xpathLiteral(buttonLabel)}]`;
  const button = await browser.$(selector);
  await button.waitForDisplayed({ timeout });
  await button.click();
}

async function waitForKeyManagementStatus(browser, testId, expectedTexts, timeout, description) {
  const startedAt = Date.now();
  let lastText = "";
  while (Date.now() - startedAt < timeout) {
    lastText = await browser.execute((id) => {
      const element = document.querySelector(`[data-testid="${id}"]`);
      return (element?.textContent ?? "").replace(/\s+/g, " ").trim();
    }, testId);
    if (expectedTexts.some((expectedText) => lastText.includes(expectedText))) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not reach expected status. Last text: ${lastText}`);
}

async function waitForSecureBackupSetupEvidence(browser, timeout) {
  const startedAt = Date.now();
  let last = { title: "", statusText: "" };
  while (Date.now() - startedAt < timeout) {
    last = await browser.execute(() => {
      const statusElement = document.querySelector('[data-testid="secure-backup-state"]');
      return {
        title: document.title,
        statusText: (statusElement?.textContent ?? "").replace(/\s+/g, " ").trim()
      };
    });
    if (["Recovery key saved", "Enabled"].some((text) => last.statusText.includes(text))) {
      return;
    }
    const status = parseQaTitle(last.title);
    if (
      status.errors === 0 &&
      status.panel === "recovery" &&
      (status.session === "needsRecovery" || status.session === "recovering")
    ) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `local GUI secure-backup setup did not reach status or recovery panel. Last status=${last.statusText} title=${last.title}`
  );
}

async function waitForFileExists(path, timeout, description) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeout) {
    if (existsSync(path)) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} was not produced`);
}

async function getRoomEvent(homeserver, accessToken, roomId, eventId) {
  const response = await fetch(
    `${homeserver}/_matrix/client/v3/rooms/${encodeURIComponent(roomId)}/event/${encodeURIComponent(eventId)}`,
    {
      headers: {
        authorization: `Bearer ${accessToken}`
      }
    }
  );
  if (!response.ok) {
    throw new Error(`getRoomEvent failed with HTTP ${response.status}`);
  }
  const event = await response.json();
  if (typeof event.origin_server_ts !== "number") {
    throw new Error("getRoomEvent response did not include origin_server_ts");
  }
  return event;
}

async function localDatetimeInputValue(browser, timestampMs) {
  return browser.execute((value) => {
    const date = new Date(value);
    const offset = date.getTimezoneOffset() * 60_000;
    return new Date(date.getTime() - offset).toISOString().slice(0, 16);
  }, timestampMs);
}

async function setDatetimeLocalValue(browser, value, label = "Jump to date") {
  const result = await browser.execute(({ nextValue, ariaLabel }) => {
    const input = Array.from(document.querySelectorAll("input")).find(
      (candidate) => candidate.getAttribute("aria-label") === ariaLabel
    );
    if (!(input instanceof HTMLInputElement)) {
      return {
        ok: false,
        reason: "missing-input",
        inputExists: false,
        valuePresent: false,
        valueLength: 0,
        valid: false
      };
    }
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value"
    )?.set;
    valueSetter?.call(input, nextValue);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return {
      ok: input.value === nextValue && input.validity.valid,
      reason: input.value === nextValue ? "set" : "value-mismatch",
      inputExists: true,
      valuePresent: input.value.length > 0,
      valueLength: input.value.length,
      valid: input.validity.valid
    };
  }, { nextValue: value, ariaLabel: label });
  if (!result.ok) {
    throw new Error(
      `local GUI datetime input setter failed for ${label}. Diagnostics: ${JSON.stringify(
        result
      )}`
    );
  }
}

async function timelineDateJumpDiagnostics(browser, expectedValue = null) {
  return browser.execute((expected) => {
    const textFor = (element) =>
      element ? (element.textContent ?? "").replace(/\s+/g, " ").trim() : "";
    const input = document.querySelector('input[aria-label="Jump to date"]');
    const form = input?.closest("form") ?? null;
    const submitButtons = Array.from(form?.querySelectorAll("button") ?? [])
      .map((button) => textFor(button))
      .filter(Boolean);
    return {
      title: document.title,
      inputExists: input instanceof HTMLInputElement,
      valuePresent: input instanceof HTMLInputElement ? input.value.length > 0 : false,
      valueLength: input instanceof HTMLInputElement ? input.value.length : 0,
      valueMatchesExpected:
        input instanceof HTMLInputElement && expected !== null ? input.value === expected : null,
      valid: input instanceof HTMLInputElement ? input.validity.valid : false,
      formExists: Boolean(form),
      submitButtons
    };
  }, expectedValue);
}

async function waitForCompressedImageMedia(browser, expected, timeout) {
  const startedAt = Date.now();
  let lastRows = [];
  while (Date.now() - startedAt < timeout) {
    lastRows = await browser.execute(() =>
      Array.from(document.querySelectorAll(".message-media")).map((row) => ({
        kind: row.getAttribute("data-media-kind"),
        title: row.querySelector(".message-media-title")?.textContent?.trim() ?? "",
        meta: row.querySelector(".message-media-meta")?.textContent?.trim() ?? ""
      }))
    );
    const found = lastRows.some(
      (row) =>
        row.kind === "Image" &&
        row.title === expected.filename &&
        row.meta.includes(expected.mimetype) &&
        row.meta.includes(expected.dimensions)
    );
    if (found) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `compressed image media row missing ${JSON.stringify(expected)}. Last rows: ${JSON.stringify(lastRows)}`
  );
}

async function waitForRichFormattedTimeline(browser, expected, expectedWhiteSpace, timeout) {
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    lastDiagnostics = await browser.execute(({ expected, expectedWhiteSpace }) => {
      const rows = Array.from(document.querySelectorAll(".message"));
      const row = rows.find((candidate) =>
        (candidate.querySelector(".message-formatted-body")?.textContent ?? "").includes(
          expected.strongText
        )
      );
      if (!row) {
        return { found: false, expectedWhiteSpace };
      }
      const link = Array.from(row.querySelectorAll("a")).find(
        (candidate) =>
          candidate.getAttribute("href") === expected.linkHref &&
          (candidate.textContent ?? "").trim() === expected.linkText
      );
      const pre = row.querySelector(".message-code-block-pre");
      const copyButton = Array.from(row.querySelectorAll("button")).find((button) =>
        (button.textContent ?? "").includes("Copy code")
      );
      return {
        found: true,
        strong: (row.querySelector("strong")?.textContent ?? "").trim(),
        quote: (row.querySelector("blockquote")?.textContent ?? "").trim(),
        list: (row.querySelector("li")?.textContent ?? "").trim(),
        linkOk: Boolean(link),
        code: (row.querySelector("pre code.language-rust")?.textContent ?? "").trim(),
        copyButtonOk: Boolean(copyButton),
        whiteSpace: pre ? window.getComputedStyle(pre).whiteSpace : "",
        expectedWhiteSpace
      };
    }, { expected, expectedWhiteSpace });

    if (
      lastDiagnostics.found &&
      lastDiagnostics.strong === expected.strongText &&
      lastDiagnostics.quote === expected.quoteText &&
      lastDiagnostics.list === expected.listText &&
      lastDiagnostics.linkOk &&
      lastDiagnostics.code === expected.codeText &&
      lastDiagnostics.copyButtonOk &&
      lastDiagnostics.whiteSpace === expectedWhiteSpace
    ) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `rich formatted timeline did not reach expected DOM state. Last diagnostics: ${JSON.stringify(
      lastDiagnostics
    )}`
  );
}

async function waitForCjkVisualContract(browser, expected, timeout) {
  const startedAt = Date.now();
  let diagnostics = null;
  while (Date.now() - startedAt < timeout) {
    diagnostics = await browser.execute(({ roomName, messageBody }) => {
      const byText = (selector, text) =>
        Array.from(document.querySelectorAll(selector)).find((element) =>
          (element.textContent ?? "").includes(text)
        );
      const metricsFor = (element) => {
        if (!element) {
          return null;
        }
        const style = window.getComputedStyle(element);
        return {
          clientWidth: element.clientWidth,
          hyphens: style.hyphens,
          lineBreak: style.lineBreak,
          scrollWidth: element.scrollWidth,
          textOverflow: style.textOverflow,
          wordBreak: style.wordBreak
        };
      };
      const contractOk = (metrics) =>
        metrics?.hyphens === "none" &&
        metrics?.lineBreak === "strict" &&
        metrics?.wordBreak === "normal";
      const roomMetrics = metricsFor(byText(".room-name", roomName));
      const bodyMetrics = metricsFor(byText(".message-body", messageBody));
      const roomOk =
        contractOk(roomMetrics) &&
        roomMetrics.textOverflow === "ellipsis" &&
        roomMetrics.scrollWidth > roomMetrics.clientWidth;
      const bodyOk =
        contractOk(bodyMetrics) &&
        bodyMetrics.scrollWidth <= bodyMetrics.clientWidth + 1;
      const documentOk = document.documentElement.scrollWidth <= window.innerWidth + 2;
      return {
        ok: Boolean(roomOk && bodyOk && documentOk),
        documentOk,
        roomMetrics,
        bodyMetrics
      };
    }, expected);
    if (diagnostics?.ok) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`local GUI CJK visual contract failed: ${JSON.stringify(diagnostics)}`);
}

async function elementCount(browser, selector) {
  return browser.execute((cssSelector) => document.querySelectorAll(cssSelector).length, selector);
}

async function waitForElementCount(browser, selector, expected, timeout, description) {
  const startedAt = Date.now();
  let lastCount = -1;
  while (Date.now() - startedAt < timeout) {
    lastCount = await elementCount(browser, selector);
    if (lastCount === expected) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not reach expected element count. Last count: ${lastCount}`
  );
}

async function waitForPinnedRegionVisible(browser, timeout, description) {
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    lastDiagnostics = await pinnedRegionDiagnostics(browser);
    if (lastDiagnostics.regionCount > 0) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not render pinned region: ${JSON.stringify(lastDiagnostics)}`);
}

async function waitForPinnedRegionCleared(browser, timeout, description) {
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    lastDiagnostics = await pinnedRegionDiagnostics(browser);
    if (lastDiagnostics.regionCount === 0) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not clear pinned region: ${JSON.stringify(lastDiagnostics)}`);
}

async function pinnedRegionDiagnostics(browser) {
  return browser.execute(() => {
    const regions = Array.from(
      document.querySelectorAll('section.pinned-events[aria-label="Pinned messages"]')
    );
    return {
      title: document.title,
      regionCount: regions.length,
      pinButtons: document.querySelectorAll('button[aria-label="Pin message"]').length,
      unpinButtons: document.querySelectorAll('button[aria-label="Unpin message"]').length
    };
  });
}

async function waitForRoomManagementTopic(browser, expectedTopic, timeout, description) {
  const startedAt = Date.now();
  let matched = false;
  while (Date.now() - startedAt < timeout) {
    matched = await browser.execute((topic) => {
      const rowText = (element) => (element.textContent ?? "").replace(/\s+/g, " ").trim();
      return Array.from(document.querySelectorAll(".settings-detail-row")).some((row) => {
        const text = rowText(row);
        return text.includes("Current topic") && text.includes(topic);
      });
    }, expectedTopic);
    if (matched) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not observe the Rust-owned topic snapshot`);
}

async function waitForRoomMemberRole(browser, expectedLabel, expectedValue, timeout, description) {
  const startedAt = Date.now();
  let observed = { label: "", value: "" };
  while (Date.now() - startedAt < timeout) {
    observed = await browser.execute(() => {
      const row = document.querySelector(".room-member-row");
      const select = row?.querySelector('select[aria-label^="Member role for"]');
      const label = Array.from(row?.querySelectorAll(".room-member-main small") ?? [])
        .map((element) => (element.textContent ?? "").trim())
        .find((text) => ["Creator", "Administrator", "Moderator", "User"].includes(text));
      return {
        label: label ?? "",
        value: select instanceof HTMLSelectElement ? select.value : ""
      };
    });
    if (observed.label === expectedLabel && observed.value === expectedValue) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not observe the Rust-owned role snapshot. Last label=${observed.label} value=${observed.value}`
  );
}

async function waitForRoomMemberAlias(browser, expectedLabel, expectedOriginal, timeout, description) {
  const startedAt = Date.now();
  let observed = null;
  while (Date.now() - startedAt < timeout) {
    observed = await browser.execute(({ label, original }) => {
      const rowText = (element) => (element?.textContent ?? "").replace(/\s+/g, " ").trim();
      const rows = Array.from(document.querySelectorAll(".room-member-row"));
      const matchedRow = rows.find((row) => rowText(row).includes(label));
      if (!matchedRow) {
        return { matched: false, rows: rows.length };
      }
      const contextText = rowText(matchedRow.querySelector(".room-member-original-context"));
      return {
        matched: original === null ? !contextText : contextText.includes(original),
        rows: rows.length
      };
    }, { label: expectedLabel, original: expectedOriginal });
    if (observed?.matched) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not observe the Rust-owned member alias projection. Last rows=${observed?.rows ?? 0}`
  );
}

async function waitForTimelineSenderLabel(browser, expectedLabel, timeout, description) {
  const startedAt = Date.now();
  let observedCount = 0;
  while (Date.now() - startedAt < timeout) {
    observedCount = await browser.execute((label) => {
      return Array.from(document.querySelectorAll(".sender")).filter((element) =>
        (element.textContent ?? "").includes(label)
      ).length;
    }, expectedLabel);
    if (observedCount > 0) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not observe the Rust-owned timeline sender projection. Last matches=${observedCount}`
  );
}

async function clickRoomMemberAliasClear(browser, aliasLabel, timeout) {
  const startedAt = Date.now();
  let observedRows = 0;
  while (Date.now() - startedAt < timeout) {
    const result = await browser.execute((label) => {
      const textFor = (element) => (element.textContent ?? "").replace(/\s+/g, " ").trim();
      const rows = Array.from(document.querySelectorAll(".room-member-row"));
      const targetRow = rows.find((row) => textFor(row).includes(label));
      const button = targetRow
        ? Array.from(targetRow.querySelectorAll("button")).find((candidate) =>
            textFor(candidate) === "Clear alias"
          )
        : null;
      if (!(button instanceof HTMLButtonElement)) {
        return { clicked: false, rows: rows.length };
      }
      button.click();
      return { clicked: true, rows: rows.length };
    }, aliasLabel);
    observedRows = result?.rows ?? 0;
    if (result?.clicked) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `local GUI alias clear control was not found in the Rust-owned member list. Last rows=${observedRows}`
  );
}

async function selectComposerText(browser) {
  await browser.execute(() => {
    const textarea = document.querySelector('textarea[aria-label="Message composer"]');
    if (!(textarea instanceof HTMLTextAreaElement)) {
      return;
    }
    textarea.focus();
    textarea.setSelectionRange(0, textarea.value.length);
  });
}

async function openRoomContextMenu(browser, sectionId, roomName) {
  const roomButton = await browser.$(roomButtonXpath(sectionId, roomName));
  await roomButton.waitForDisplayed({ timeout: timeoutMs });
  await roomButton.moveTo();
  await roomButton.click({ button: "right" });
}

async function selectRoomByName(browser, roomName, timeout) {
  const roomButton = await browser.$(
    `//button[@data-testid="room-item"][.//span[normalize-space()=${xpathLiteral(roomName)}]]`
  );
  await roomButton.waitForDisplayed({ timeout });
  await roomButton.click();
}

async function waitForWorkspaceButton(browser, label, timeout, description) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await workspaceButtonState(browser, label);
    if (lastState.exists) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} workspace button did not appear. Last state=${JSON.stringify(lastState)}`
  );
}

async function waitForWorkspaceActive(browser, label, expected, timeout, description) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await workspaceButtonState(browser, label);
    if (lastState.exists && lastState.active === expected) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} workspace active state did not become ${expected}. Last state=${JSON.stringify(
      lastState
    )}`
  );
}

async function clickWorkspaceButton(browser, label, timeout, description) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute((targetLabel) => {
      const rail = document.querySelector(".workspace-rail");
      const button = Array.from(rail?.querySelectorAll("button") ?? []).find(
        (candidate) => candidate.getAttribute("aria-label") === targetLabel
      );
      if (!(button instanceof HTMLButtonElement)) {
        return { clicked: false, exists: false };
      }
      button.click();
      return {
        clicked: true,
        exists: true,
        active: button.classList.contains("is-active")
      };
    }, label);
    if (lastState?.clicked) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} workspace button was not clickable. Last state=${JSON.stringify(lastState)}`
  );
}

async function workspaceButtonState(browser, label) {
  return browser.execute((targetLabel) => {
    const rail = document.querySelector(".workspace-rail");
    const button = Array.from(rail?.querySelectorAll("button") ?? []).find(
      (candidate) => candidate.getAttribute("aria-label") === targetLabel
    );
    return {
      exists: button instanceof HTMLButtonElement,
      active: button instanceof HTMLButtonElement && button.classList.contains("is-active")
    };
  }, label);
}

async function waitForActiveRoomName(browser, roomName, timeout) {
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    lastDiagnostics = await activeRoomDiagnostics(browser);
    if (
      lastDiagnostics.activeHeader === roomName ||
      lastDiagnostics.activeRows.includes(roomName)
    ) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `active room did not become ${roomName}. Last diagnostics: ${JSON.stringify(
      lastDiagnostics
    )}`
  );
}

async function waitForTimelineViewMounted(browser, timeout) {
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    lastDiagnostics = await messageActionDiagnostics(browser);
    if (lastDiagnostics.timelineViews > 0) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `timeline view was not mounted. Last diagnostics: ${JSON.stringify(lastDiagnostics)}`
  );
}

async function scrollTimelineToTop(browser) {
  await browser.execute(() => {
    const timeline = document.querySelector('[data-testid="timeline-view"]');
    if (timeline instanceof HTMLElement) {
      timeline.scrollTop = 0;
      timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
    }
  });
}

async function scrollTimelineToBottom(browser) {
  await browser.execute(() => {
    const timeline = document.querySelector('[data-testid="timeline-view"]');
    if (timeline instanceof HTMLElement) {
      timeline.scrollTop = timeline.scrollHeight;
      timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
    }
  });
}

async function waitForTimelineScrolledToBottom(browser, timeout, description) {
  const startedAt = Date.now();
  let lastMetrics = null;
  while (Date.now() - startedAt < timeout) {
    lastMetrics = await timelineScrollMetrics(browser);
    if (lastMetrics?.atBottom) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not reach timeline bottom. Last metrics=${JSON.stringify(lastMetrics)}`
  );
}

async function driveTimelineToBottom(browser, timeout, description) {
  const startedAt = Date.now();
  let lastMetrics = null;
  while (Date.now() - startedAt < timeout) {
    await scrollTimelineToBottom(browser);
    lastMetrics = await timelineScrollMetrics(browser);
    if (lastMetrics?.atBottom) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not settle at timeline bottom. Last metrics=${JSON.stringify(lastMetrics)}`
  );
}

async function waitForTimelineScrollable(browser, timeout, description) {
  const startedAt = Date.now();
  let lastMetrics = null;
  while (Date.now() - startedAt < timeout) {
    lastMetrics = await timelineScrollMetrics(browser);
    if (lastMetrics?.scrollable) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not become scrollable. Last metrics=${JSON.stringify(lastMetrics)}`
  );
}

async function waitForTimelineAwayFromBottom(browser, timeout, description) {
  const startedAt = Date.now();
  let lastMetrics = null;
  while (Date.now() - startedAt < timeout) {
    lastMetrics = await timelineScrollMetrics(browser);
    if (lastMetrics?.scrollable && !lastMetrics.atBottom) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} did not move away from bottom. Last metrics=${JSON.stringify(lastMetrics)}`
  );
}

async function timelineScrollMetrics(browser) {
  return browser.execute(() => {
    const timeline = document.querySelector('[data-testid="timeline-view"]');
    if (!(timeline instanceof HTMLElement)) {
      return null;
    }
    const bottomOffset = Math.abs(
      timeline.scrollHeight - timeline.clientHeight - timeline.scrollTop
    );
    return {
      scrollTop: timeline.scrollTop,
      scrollHeight: timeline.scrollHeight,
      clientHeight: timeline.clientHeight,
      bottomOffset,
      messageCount: document.querySelectorAll(".message").length,
      scrollable: timeline.scrollHeight > timeline.clientHeight,
      atBottom: bottomOffset <= 2
    };
  });
}

function timelineNavigationSeedBody(index) {
  return Array.from(
    { length: TIMELINE_NAVIGATION_SEED_LINE_COUNT },
    (_, lineIndex) =>
      `QA timeline navigation seed ${index}.${lineIndex} scroll contract`
  ).join("\n");
}

async function activeRoomDiagnostics(browser) {
  return browser.execute(() => {
    const textFor = (element) =>
      element ? (element.textContent ?? "").replace(/\s+/g, " ").trim() : "";
    const roomRows = Array.from(document.querySelectorAll('button[data-testid="room-item"]'));
    return {
      title: document.title,
      qaLastError: window.__matrixDesktopQaLastError ?? null,
      activeHeader: textFor(document.querySelector(".channel-title > span")),
      activeRows: roomRows
        .filter((row) => row.classList.contains("is-active"))
        .map((row) => textFor(row.querySelector(".room-name")))
        .filter(Boolean),
      roomRows: roomRows
        .map((row) => ({
          name: textFor(row.querySelector(".room-name")),
          active: row.classList.contains("is-active")
        }))
        .slice(0, 8)
    };
  });
}

async function clickVisibleButtonByTextPrefix(browser, prefix, timeout, description) {
  const startedAt = Date.now();
  let observed = [];
  while (Date.now() - startedAt < timeout) {
    const result = await browser.execute((targetPrefix) => {
      const textFor = (element) => (element.textContent ?? "").replace(/\s+/g, " ").trim();
      const isVisible = (element) => {
        const style = window.getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          Number(style.opacity) !== 0 &&
          rect.width > 0 &&
          rect.height > 0
        );
      };
      const buttons = Array.from(document.querySelectorAll("button"));
      const labels = buttons.map(textFor).filter(Boolean);
      const target = buttons.find(
        (button) => textFor(button).startsWith(targetPrefix) && isVisible(button)
      );
      if (!target) {
        return { clicked: false, labels };
      }
      target.click();
      return { clicked: true, labels };
    }, prefix);
    observed = result?.labels ?? [];
    if (result?.clicked) {
      return;
    }
    await sleep(250);
  }
  const metrics = await timelineScrollMetrics(browser).catch(() => null);
  throw new Error(
    `${description} button starting with ${prefix} was not found. Observed: ${observed.join(", ")}. Timeline metrics=${JSON.stringify(metrics)}`
  );
}

async function clickVisibleButtonByAriaLabel(browser, label, timeout, description, scopeSelector = null) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute((targetLabel, targetScopeSelector) => {
      const visible = (element) => {
        const style = window.getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          Number(style.opacity) !== 0 &&
          rect.width > 0 &&
          rect.height > 0
        );
      };
      const labelFor = (element) =>
        element?.getAttribute("aria-label") ?? element?.textContent?.replace(/\s+/g, " ").trim() ?? "";
      const scope = targetScopeSelector ? document.querySelector(targetScopeSelector) : document;
      if (!scope) {
        return { clicked: false, reason: "missing-scope", labels: [] };
      }
      const buttons = Array.from(scope.querySelectorAll("button"));
      const labels = buttons.map(labelFor).filter(Boolean);
      const target = buttons.find(
        (button) => button.getAttribute("aria-label") === targetLabel && visible(button)
      );
      if (!target) {
        return { clicked: false, reason: "missing", labels };
      }
      target.scrollIntoView({ block: "center", inline: "center" });
      const rect = target.getBoundingClientRect();
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      const topElement = document.elementFromPoint(centerX, centerY);
      if (topElement !== target && !target.contains(topElement)) {
        return {
          clicked: false,
          reason: "covered",
          labels,
          topLabel: labelFor(topElement),
          topTag: topElement?.tagName ?? null,
          topClass: topElement instanceof HTMLElement ? topElement.className : null
        };
      }
      target.click();
      return { clicked: true, labels };
    }, label, scopeSelector);
    if (lastState?.clicked) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} button with aria-label ${label} was not clickable. Last state=${JSON.stringify(lastState)}`
  );
}

async function clickVisibleButtonByAriaLabelInElement(element, label, timeout, description) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await element.execute((root, targetLabel) => {
      const visible = (candidate) => {
        const style = window.getComputedStyle(candidate);
        const rect = candidate.getBoundingClientRect();
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          Number(style.opacity) !== 0 &&
          rect.width > 0 &&
          rect.height > 0
        );
      };
      const labelFor = (candidate) =>
        candidate?.getAttribute("aria-label") ??
        candidate?.textContent?.replace(/\s+/g, " ").trim() ??
        "";
      const buttons = Array.from(root.querySelectorAll("button"));
      const labels = buttons.map(labelFor).filter(Boolean);
      const target = buttons.find(
        (button) => button.getAttribute("aria-label") === targetLabel && visible(button)
      );
      if (!target) {
        return { clicked: false, reason: "missing", labels };
      }
      target.scrollIntoView({ block: "center", inline: "center" });
      const rect = target.getBoundingClientRect();
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      const topElement = document.elementFromPoint(centerX, centerY);
      if (topElement !== target && !target.contains(topElement)) {
        return {
          clicked: false,
          reason: "covered",
          labels,
          topLabel: labelFor(topElement),
          topTag: topElement?.tagName ?? null,
          topClass: topElement instanceof HTMLElement ? topElement.className : null
        };
      }
      target.click();
      return { clicked: true, labels };
    }, label);
    if (lastState?.clicked) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} button with aria-label ${label} was not clickable in target row. Last state=${JSON.stringify(lastState)}`
  );
}

async function clickMenuItemByText(browser, label, timeout) {
  const menuItemSelector = 'button[role="menuitem"]';
  const startedAt = Date.now();
  let observed = [];
  while (Date.now() - startedAt < timeout) {
    const items = await browser.$$(menuItemSelector);
    observed = [];
    for (const item of items) {
      const text = (await item.getText()).trim();
      observed.push(text);
      if (text === label) {
        await item.waitForDisplayed({ timeout: 1000 });
        await item.click();
        return;
      }
    }
    await sleep(250);
  }
  throw new Error(`menu item ${label} was not found. Observed: ${observed.join(", ")}`);
}

async function clickVisibleMenuItemByText(browser, label, timeout) {
  const startedAt = Date.now();
  let observed = [];
  while (Date.now() - startedAt < timeout) {
    const result = await browser.execute((targetLabel) => {
      const textFor = (element) => (element.textContent ?? "").replace(/\s+/g, " ").trim();
      const isVisible = (element) => {
        const style = window.getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return (
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          Number(style.opacity) !== 0 &&
          rect.width > 0 &&
          rect.height > 0
        );
      };
      const items = Array.from(document.querySelectorAll('button[role="menuitem"]'));
      const labels = items.map(textFor).filter(Boolean);
      const target = items.find(
        (item) => textFor(item) === targetLabel && isVisible(item)
      );
      if (!target) {
        return { clicked: false, labels };
      }
      target.click();
      return { clicked: true, labels };
    }, label);
    observed = result?.labels ?? [];
    if (result?.clicked) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`visible menu item ${label} was not found. Observed: ${observed.join(", ")}`);
}

async function waitForMessageSourceDialog(browser, timeout) {
  const selector = '[role="dialog"][aria-label="Message source"]';
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    const present = await browser.execute(
      (cssSelector) => document.querySelector(cssSelector) !== null,
      selector
    );
    if (present) {
      return;
    }
    lastDiagnostics = await messageActionDiagnostics(browser);
    await sleep(250);
  }
  throw new Error(
    `local GUI message source dialog was not found. Last diagnostics: ${JSON.stringify(
      lastDiagnostics
    )}`
  );
}

async function messageActionDiagnostics(browser) {
  return browser.execute(() => {
    const textFor = (element) =>
      element ? (element.textContent ?? "").replace(/\s+/g, " ").trim() : "";
    const labelsFor = (selector) =>
      Array.from(document.querySelectorAll(selector))
        .map((element) => element.getAttribute("aria-label") ?? textFor(element))
        .filter(Boolean)
        .slice(0, 8);
    const active = document.activeElement;
    return {
      title: document.title,
      qaLastError: window.__matrixDesktopQaLastError ?? null,
      activeHeader: textFor(document.querySelector(".channel-title > span")),
      timelineViews: document.querySelectorAll('[data-testid="timeline-view"]').length,
      messageLists: document.querySelectorAll(".message-list").length,
      messages: document.querySelectorAll(".message").length,
      eventRows: document.querySelectorAll(".message[data-event-id]").length,
      transactionRows: Array.from(document.querySelectorAll(".message")).filter(
        (row) => !row.hasAttribute("data-event-id")
      ).length,
      actionButtons: document.querySelectorAll('button[aria-label="Message actions"]').length,
      actionMenus: document.querySelectorAll(".message-action-menu").length,
      menuLabels: labelsFor('button[role="menuitem"]'),
      dialogs: document.querySelectorAll('[role="dialog"]').length,
      dialogLabels: labelsFor('[role="dialog"]'),
      activeTag: active?.tagName ?? null,
      activeLabel: active?.getAttribute("aria-label") ?? null
    };
  });
}

async function waitForLatestMessageActionButton(browser, timeout) {
  const selector = 'button[aria-label="Message actions"]';
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    const buttons = await browser.$$(selector);
    if (buttons.length) {
      return buttons[buttons.length - 1];
    }
    lastDiagnostics = await messageActionDiagnostics(browser);
    await sleep(250);
  }
  throw new Error(
    `message action button was not found. Last diagnostics: ${JSON.stringify(lastDiagnostics)}`
  );
}

async function clickLatestMessageRedactButtonByText(browser, bodyText, timeout) {
  const row = await waitForLatestEventMessageRowByText(
    browser,
    bodyText,
    timeout,
    "local GUI redaction target"
  );
  await row.moveTo();
  const redactButton = await row.$('button[aria-label="Redact message"]');
  await redactButton.waitForDisplayed({ timeout });
  await redactButton.click();
}

async function waitForLatestEventMessageRow(browser, timeout, description) {
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    const rows = await browser.$$(".message[data-event-id]");
    if (rows.length > 0) {
      return rows[rows.length - 1];
    }
    lastDiagnostics = await messageActionDiagnostics(browser);
    await sleep(250);
  }
  throw new Error(
    `${description} event row was not found. Last diagnostics: ${JSON.stringify(lastDiagnostics)}`
  );
}

async function waitForLatestEventMessageRowByText(browser, bodyText, timeout, description) {
  const startedAt = Date.now();
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    const rows = await browser.$$(".message[data-event-id]");
    for (let index = rows.length - 1; index >= 0; index -= 1) {
      const rowText = (await rows[index].getText()).replace(/\s+/g, " ").trim();
      if (rowText.includes(bodyText)) {
        return rows[index];
      }
    }
    lastDiagnostics = await messageActionDiagnostics(browser);
    await sleep(250);
  }
  throw new Error(
    `${description} event row was not found. Last diagnostics: ${JSON.stringify(lastDiagnostics)}`
  );
}

async function waitForRoomInSection(browser, sectionId, roomName, expected, timeout, description) {
  const startedAt = Date.now();
  let lastTitle = "";
  let lastPresent = false;
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`${description} reported errors. Last title: ${lastTitle}`);
    }
    lastPresent = await roomExistsInSection(browser, sectionId, roomName);
    if (lastPresent === expected) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `${description} expected ${roomName} in ${sectionId} to be ${expected}; last present=${lastPresent}. Last title: ${lastTitle}`
  );
}

async function roomExistsInSection(browser, sectionId, roomName) {
  const sectionSelector = roomSectionSelector(sectionId);
  const roomButtonSelector = 'button[data-testid="room-item"]';
  return browser.execute(
    (targetSectionSelector, targetRoomName, selector) => {
      const section = document.querySelector(targetSectionSelector);
      if (!section) {
        return false;
      }
      return Array.from(section.querySelectorAll(selector)).some(
        (button) => button.textContent?.includes(targetRoomName) ?? false
      );
    },
    sectionSelector,
    roomName,
    roomButtonSelector
  );
}

function roomSectionSelector(sectionId) {
  switch (sectionId) {
    case "favourites":
      return 'section[data-room-section="favourites"]';
    case "rooms":
      return 'section[data-room-section="rooms"]';
    default:
      throw new Error(`unknown room section: ${sectionId}`);
  }
}

async function setTextInputValueByLabel(browser, value, label) {
  const result = await browser.execute(({ nextValue, ariaLabel }) => {
    const input = Array.from(document.querySelectorAll("input")).find(
      (candidate) => candidate.getAttribute("aria-label") === ariaLabel
    );
    if (!(input instanceof HTMLInputElement)) {
      return { ok: false, reason: "missing-input" };
    }
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value"
    )?.set;
    valueSetter?.call(input, nextValue);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return {
      ok: true,
      reason: input.value === nextValue ? "set" : "value-mismatch"
    };
  }, { nextValue: value, ariaLabel: label });
  if (!result?.ok) {
    throw new Error(`text input set failed: ${result?.reason ?? "unknown"}`);
  }
}

async function waitForInputValue(browser, label, expectedValue, timeout, description) {
  const startedAt = Date.now();
  let lastValue = "";
  while (Date.now() - startedAt < timeout) {
    const observed = await browser.execute((ariaLabel) => {
      const input = Array.from(document.querySelectorAll("input")).find(
        (candidate) => candidate.getAttribute("aria-label") === ariaLabel
      );
      return input instanceof HTMLInputElement ? input.value : null;
    }, label);
    lastValue = observed ?? "";
    if (observed === expectedValue) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not become expected value. Last value: ${lastValue}`);
}

function roomButtonXpath(sectionId, roomName) {
  return `//section[@data-room-section=${xpathLiteral(sectionId)}]//button[@data-testid="room-item"][.//span[normalize-space()=${xpathLiteral(roomName)}]]`;
}

function keyManagementFormInputXpath(formLabel, fieldLabel) {
  return `//form[@aria-label=${xpathLiteral(formLabel)}]//label[.//span[normalize-space()=${xpathLiteral(fieldLabel)}]]//input`;
}

function xpathLiteral(value) {
  if (!value.includes("'")) {
    return `'${value}'`;
  }
  if (!value.includes('"')) {
    return `"${value}"`;
  }
  return `concat(${value
    .split("'")
    .map((part) => `'${part}'`)
    .join(`, "\"'\"", `)})`;
}

function writePngFixture(path, width, height) {
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y += 1) {
    const rowOffset = y * (width * 4 + 1);
    raw[rowOffset] = 0;
    for (let x = 0; x < width; x += 1) {
      const offset = rowOffset + 1 + x * 4;
      raw[offset] = x % 2 === 0 ? 45 : 255;
      raw[offset + 1] = y % 2 === 0 ? 111 : 255;
      raw[offset + 2] = 239;
      raw[offset + 3] = 255;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  writeFileSync(
    path,
    Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      pngChunk("IHDR", ihdr),
      pngChunk("IDAT", deflateSync(raw)),
      pngChunk("IEND", Buffer.alloc(0))
    ])
  );
}

function pngChunk(type, data) {
  const typeBuffer = Buffer.from(type, "ascii");
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])), 0);
  return Buffer.concat([length, typeBuffer, data, crc]);
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc = pngCrc32Table[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

async function setSyntheticFileInput(browser, selector, fixturePath, filename, mimeType, contents) {
  await makeFileInputInteractable(browser, selector);
  const input = await browser.$(selector);
  try {
    await input.waitForDisplayed({ timeout: timeoutMs });
    await input.setValue(fixturePath);
    const hasNativeFiles = await fileInputHasFiles(browser, selector);
    if (!hasNativeFiles) {
      await setSyntheticFileList(browser, selector, filename, mimeType, contents);
    }
    await dispatchFileInputChange(browser, selector);
  } finally {
    await restoreFileInputPresentation(browser, selector);
  }
}

async function fileInputHasFiles(browser, selector) {
  return browser.execute((cssSelector) => {
    const input = document.querySelector(cssSelector);
    return input instanceof HTMLInputElement && (input.files?.length ?? 0) > 0;
  }, selector);
}

async function setSyntheticFileList(browser, selector, filename, mimeType, contents) {
  const result = await browser.execute(
    (cssSelector, fileName, type, payload) => {
      const input = document.querySelector(cssSelector);
      if (!(input instanceof HTMLInputElement)) {
        return { ok: false, reason: "input_missing" };
      }
      if (typeof DataTransfer !== "function") {
        return { ok: false, reason: "data_transfer_unavailable" };
      }
      const filePart =
        typeof payload === "string"
          ? payload
          : Uint8Array.from(atob(payload.base64), (character) => character.charCodeAt(0));
      const transfer = new DataTransfer();
      transfer.items.add(new File([filePart], fileName, { type }));
      Object.defineProperty(input, "files", {
        configurable: true,
        get() {
          return transfer.files;
        }
      });
      return { ok: (input.files?.length ?? 0) > 0 };
    },
    selector,
    filename,
    mimeType,
    contents
  );
  if (!result?.ok) {
    throw new Error(`synthetic file list unavailable: ${result?.reason ?? "empty"}`);
  }
}

async function makeFileInputInteractable(browser, selector) {
  const result = await browser.execute((cssSelector) => {
    const input = document.querySelector(cssSelector);
    if (!(input instanceof HTMLInputElement)) {
      return { ok: false, reason: "input_missing" };
    }
    if (!input.dataset.matrixDesktopQaOriginalStyle) {
      input.dataset.matrixDesktopQaOriginalStyle = input.getAttribute("style") ?? "";
    }
    Object.assign(input.style, {
      height: "32px",
      left: "8px",
      opacity: "1",
      overflow: "visible",
      pointerEvents: "auto",
      position: "fixed",
      top: "8px",
      width: "260px",
      zIndex: "2147483647"
    });
    return { ok: true };
  }, selector);
  if (!result?.ok) {
    throw new Error(`file input was not found: ${result?.reason ?? "unknown"}`);
  }
}

async function restoreFileInputPresentation(browser, selector) {
  await browser.execute((cssSelector) => {
    const input = document.querySelector(cssSelector);
    if (!(input instanceof HTMLInputElement)) {
      return;
    }
    const originalStyle = input.dataset.matrixDesktopQaOriginalStyle;
    if (originalStyle) {
      input.setAttribute("style", originalStyle);
    } else {
      input.removeAttribute("style");
    }
    delete input.dataset.matrixDesktopQaOriginalStyle;
  }, selector);
}

async function dispatchFileInputChange(browser, selector) {
  const result = await browser.execute((cssSelector) => {
    const input = document.querySelector(cssSelector);
    if (!(input instanceof HTMLInputElement)) {
      return { ok: false, reason: "input_missing" };
    }
    const fileCount = input.files?.length ?? 0;
    if (fileCount < 1) {
      return { ok: false, reason: "file_list_empty" };
    }
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true };
  }, selector);
  if (!result?.ok) {
    throw new Error(`file input change dispatch failed: ${result?.reason ?? "unknown"}`);
  }
}

async function waitForElementCountGreaterThan(browser, selector, baseline, timeout, description) {
  const startedAt = Date.now();
  let lastCount = baseline;
  let lastDiagnostics = null;
  while (Date.now() - startedAt < timeout) {
    lastCount = await elementCount(browser, selector);
    if (lastCount > baseline) {
      return;
    }
    if (selector.includes(".message")) {
      lastDiagnostics = await messageActionDiagnostics(browser);
    }
    await sleep(250);
  }
  const diagnosticSuffix = lastDiagnostics
    ? `. Last diagnostics: ${JSON.stringify(lastDiagnostics)}`
    : "";
  throw new Error(
    `${description} did not increase ${selector}. Baseline: ${baseline}; last count: ${lastCount}${diagnosticSuffix}`
  );
}

async function waitForReplyLanded(browser, baselineMessages, timeout) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`local GUI reply reported errors. Last title: ${lastTitle}`);
    }
    if (status.send === "failed") {
      throw new Error(`local GUI reply send failed. Last title: ${lastTitle}`);
    }
    const observed = await browser.execute(() => ({
      messages: document.querySelectorAll(".message").length,
      replyRows: document.querySelectorAll('[data-reply="true"]').length
    }));
    if (observed.replyRows > 0 || observed.messages > baselineMessages) {
      return lastTitle;
    }
    await sleep(250);
  }
  throw new Error(`local GUI reply did not land. Last title: ${lastTitle}`);
}

async function startLocalGuiScenario() {
  checkLinuxTools();

  const runDir = join(artifactRoot, `${timestamp()}-${guiScenario}`);
  const appDataDir = qaDataDirForRun(runDir);
  const serverDataDir = join(runDir, "homeserver-data");
  const logPath = join(runDir, "run.log");
  mkdirSync(runDir, { recursive: true });
  mkdirSync(appDataDir, { recursive: true });
  mkdirSync(serverDataDir, { recursive: true });

  const session = {
    appDataDir,
    browser: null,
    buildEnv: null,
    dbusMonitor: null,
    dbusSession: null,
    logPath,
    qaControlPipePath: null,
    qaLoginPipePath: null,
    runDir,
    serverProcess: null,
    tauriDriver: null,
    xvfb: null,
    credentials: null,
    dmTargetUserId: null,
    helperAccessToken: null,
    composerMentionDisplayName: null,
    cjkMessageBody: null,
    cjkRoomName: null,
    directoryRoomName: null,
    roomManagementTopic: null,
    aliasMemberDisplayName: null,
    aliasLocalDisplayName: null,
    primaryUserId: null,
    seedRoomId: null,
    seedInviteRoomName: null
  };

  try {
    const serverKind = guiScenarioServerKind();
    checkInstalledHomeserver(serverKind);
    const port = await freePort();
    const serverName = `localhost:${port}`;
    const homeserver = `http://127.0.0.1:${port}`;
    const configPath = join(runDir, `${serverKind}.toml`);
    writeFileSync(
      configPath,
      serverKind === "conduit"
        ? conduitConfig({ serverName, port, dataDir: serverDataDir })
        : tuwunelConfig({ serverName, port, dataDir: serverDataDir })
    );

    session.serverProcess = startHomeserver(serverKind, configPath, logPath);
    await waitForHomeserver(homeserver, session.serverProcess, timeoutMs, logPath);

    const userSuffix = safeTimestamp();
    const username = `qa_local_${userSuffix}`;
    const password = `koushi-desktop-local-${userSuffix}`;
    const registration = await registerUser(homeserver, username, password);
    const accessToken = registration.access_token;
    const userId = registration.user_id;
    if (!accessToken) {
      throw new Error("local GUI setup did not return an access token");
    }
    if (!userId) {
      throw new Error("local GUI setup did not return a user id");
    }
    const seedRoomName =
      guiScenario === "local-cjk"
        ? `日本語幅確認ルーム${"長い名前".repeat(24)}`
        : "QA Seed Room";
    const seedRoom = await createRoom(homeserver, accessToken, { name: seedRoomName });
    const seedRoomId = seedRoom.room_id;
    if (!seedRoomId) {
      throw new Error("local GUI setup did not return a seed room id");
    }
    session.seedRoomId = seedRoomId;
    session.primaryUserId = userId;

    if (guiScenario === "local-cjk") {
      session.cjkRoomName = seedRoomName;
      session.cjkMessageBody = `日本語の長文メッセージ${"かなカナ漢字と幅確認".repeat(20)}`;
    }

    if (guiScenario === "local-composer") {
      const helperUsername = `qa_mention_${userSuffix}`;
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      const helperRegistration = await registerUser(homeserver, helperUsername, helperPassword);
      const helperAccessToken = helperRegistration.access_token;
      const helperUserId = helperRegistration.user_id;
      if (!helperAccessToken || !helperUserId) {
        throw new Error("local GUI composer setup did not return helper credentials");
      }
      session.composerMentionDisplayName = "Mention Helper";
      await setDisplayName(
        homeserver,
        helperAccessToken,
        helperUserId,
        session.composerMentionDisplayName
      );
      await inviteUserToRoom(homeserver, accessToken, seedRoomId, helperUserId);
      await joinRoom(homeserver, helperAccessToken, seedRoomId);
      await sendRoomMessage(
        homeserver,
        helperAccessToken,
        seedRoomId,
        "QA helper seed message",
        `qa-helper-${userSuffix}`
      );
    }

    if (guiScenario === "local-timeline-navigation") {
      const helperUsername = `qa_timeline_nav_${userSuffix}`;
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      const helperRegistration = await registerUser(homeserver, helperUsername, helperPassword);
      const helperAccessToken = helperRegistration.access_token;
      const helperUserId = helperRegistration.user_id;
      if (!helperAccessToken || !helperUserId) {
        throw new Error("local GUI timeline navigation setup did not return helper credentials");
      }
      session.helperAccessToken = helperAccessToken;
      await inviteUserToRoom(homeserver, accessToken, seedRoomId, helperUserId);
      await joinRoom(homeserver, helperAccessToken, seedRoomId);
      for (let index = 0; index < TIMELINE_NAVIGATION_SEED_MESSAGE_COUNT; index += 1) {
        await sendRoomMessage(
          homeserver,
          accessToken,
          seedRoomId,
          timelineNavigationSeedBody(index),
          `qa-timeline-nav-seed-${index}-${userSuffix}`
        );
      }
    }

    if (guiScenario === "local-message-types") {
      const helperUsername = `qa_message_types_${userSuffix}`;
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      const helperRegistration = await registerUser(homeserver, helperUsername, helperPassword);
      const helperAccessToken = helperRegistration.access_token;
      const helperUserId = helperRegistration.user_id;
      if (!helperAccessToken || !helperUserId) {
        throw new Error("local GUI message types setup did not return helper credentials");
      }
      session.helperAccessToken = helperAccessToken;
      await inviteUserToRoom(homeserver, accessToken, seedRoomId, helperUserId);
      await joinRoom(homeserver, helperAccessToken, seedRoomId);
    }

    if (guiScenario === "local-rich-formatting") {
      session.richFormatted = {
        strongText: "Formatted keyword",
        quoteText: "Quoted body",
        listText: "List item",
        linkText: "safe link",
        linkHref: "https://example.invalid/path",
        codeText:
          'const veryLongToken = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";'
      };
      await sendRoomFormattedMessage(
        homeserver,
        accessToken,
        seedRoomId,
        [
          session.richFormatted.strongText,
          session.richFormatted.quoteText,
          session.richFormatted.listText,
          session.richFormatted.linkText,
          session.richFormatted.codeText
        ].join(" "),
        `<strong>${session.richFormatted.strongText}</strong>` +
          `<blockquote>${session.richFormatted.quoteText}</blockquote>` +
          `<ul><li>${session.richFormatted.listText}</li></ul>` +
          `<a href="${session.richFormatted.linkHref}">${session.richFormatted.linkText}</a>` +
          `<pre><code class="language-rust">${session.richFormatted.codeText}</code></pre>`,
        `qa-rich-formatting-${userSuffix}`
      );
    }

    if (guiScenario === "local-alias") {
      const helperUsername = `qa_alias_${userSuffix}`;
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      const helperRegistration = await registerUser(homeserver, helperUsername, helperPassword);
      const helperAccessToken = helperRegistration.access_token;
      const helperUserId = helperRegistration.user_id;
      if (!helperAccessToken || !helperUserId) {
        throw new Error("local GUI alias setup did not return helper credentials");
      }
      session.aliasMemberDisplayName = "Alias Helper";
      session.aliasLocalDisplayName = "Local Remark";
      await setDisplayName(homeserver, helperAccessToken, helperUserId, session.aliasMemberDisplayName);
      await inviteUserToRoom(homeserver, accessToken, seedRoomId, helperUserId);
      await joinRoom(homeserver, helperAccessToken, seedRoomId);
      await sendRoomMessage(
        homeserver,
        helperAccessToken,
        seedRoomId,
        "QA alias seed message",
        `qa-alias-${userSuffix}`
      );
    }

    if (guiScenario === "local-room-management") {
      const helperUsername = `qa_management_${userSuffix}`;
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      const helperRegistration = await registerUser(homeserver, helperUsername, helperPassword);
      const helperAccessToken = helperRegistration.access_token;
      const helperUserId = helperRegistration.user_id;
      if (!helperAccessToken || !helperUserId) {
        throw new Error("local GUI room management setup did not return helper credentials");
      }
      await setDisplayName(homeserver, helperAccessToken, helperUserId, "Management Helper");
      await inviteUserToRoom(homeserver, accessToken, seedRoomId, helperUserId);
      await joinRoom(homeserver, helperAccessToken, seedRoomId);
      await sendRoomMessage(
        homeserver,
        helperAccessToken,
        seedRoomId,
        "QA room management seed message",
        `qa-management-${userSuffix}`
      );
      session.roomManagementTopic = "QA managed topic";
    }

    if (guiScenario === "local-invites-dm") {
      const helperUsername = `qa_inviter_${userSuffix}`;
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      const helperRegistration = await registerUser(homeserver, helperUsername, helperPassword);
      const helperAccessToken = helperRegistration.access_token;
      const helperUserId = helperRegistration.user_id;
      if (!helperAccessToken || !helperUserId) {
        throw new Error("local GUI invite setup did not return helper credentials");
      }
      session.seedInviteRoomName = "QA Invite Room";
      session.dmTargetUserId = helperUserId;
      session.helperAccessToken = helperAccessToken;
    }

    if (guiScenario === "local-explore") {
      const helperUsername = `qa_directory_${userSuffix}`;
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      const helperRegistration = await registerUser(homeserver, helperUsername, helperPassword);
      const helperAccessToken = helperRegistration.access_token;
      if (!helperAccessToken) {
        throw new Error("local GUI explore setup did not return helper credentials");
      }
      session.directoryRoomName = "QA Public Room";
      const publicRoom = await createRoom(homeserver, helperAccessToken, {
        visibility: "public",
        preset: "public_chat",
        room_alias_name: `qa-public-${userSuffix}`,
        name: session.directoryRoomName,
        topic: "QA public directory room"
      });
      if (!publicRoom.room_id) {
        throw new Error("local GUI explore setup did not return a public room id");
      }
    }

    session.qaLoginPipePath = join(appDataDir, "qa-login.pipe");
    createNamedPipe(session.qaLoginPipePath);
    if (guiScenario === "local-logout-relogin") {
      session.qaControlPipePath = join(appDataDir, "qa-control.pipe");
      createNamedPipe(session.qaControlPipePath);
    }

    const baseEnv = childEnvironment(
      appDataDir,
      session.qaLoginPipePath,
      session.qaControlPipePath
    );
    session.dbusSession = ensureDbusSession(logPath, baseEnv);
    session.buildEnv = {
      ...baseEnv,
      ...session.dbusSession.env
    };
    session.xvfb = await startXvfb(logPath, session.buildEnv);
    const driverPort = await freePort();
    const nativePort = await freePort();
    session.tauriDriver = spawnLogged(
      "tauri-driver",
      ["--port", String(driverPort), "--native-port", String(nativePort)],
      {
        cwd: desktopDir,
        env: { ...session.buildEnv, DISPLAY: `:${session.xvfb.display}` },
        detached: true,
        logPath,
        label: "tauri-driver"
      }
    );

    const appBinary = await ensureAppBinary({ cwd: desktopDir, env: session.buildEnv, logPath });
    await waitForPort("127.0.0.1", driverPort, timeoutMs);

    const { remote } = await importDesktopWebdriverio();
    session.browser = await remote({
      hostname: "127.0.0.1",
      port: driverPort,
      logLevel: "error",
      capabilities: webdriverCapabilities(appBinary)
    });
    session.credentials = {
      homeserver,
      username,
      password,
      deviceName: "Koushi Local QA"
    };

    return session;
  } catch (error) {
    await cleanupLocalGuiScenario(session);
    throw error;
  }
}

async function cleanupLocalGuiScenario(session) {
  try {
    if (session.dbusMonitor) {
      terminateProcessGroup(session.dbusMonitor.child, "SIGTERM");
      await settleChild(session.dbusMonitor.child);
    }
    if (session.browser) {
      await safeDeleteSession(session.browser);
    }
  } finally {
    if (session.dbusSession?.pid) {
      try {
        process.kill(session.dbusSession.pid, "SIGTERM");
      } catch {
        // ignore cleanup failures
      }
    }
    if (session.tauriDriver) {
      terminateProcessGroup(session.tauriDriver, "SIGTERM");
      await settleChild(session.tauriDriver);
    }
    if (session.xvfb) {
      terminateProcessGroup(session.xvfb.child, "SIGTERM");
      await settleChild(session.xvfb.child);
    }
    if (session.serverProcess) {
      await stopProcess(session.serverProcess);
    }
  }
}

async function recordLocalGuiEvidence(session) {
  session.dbusMonitor = startDbusMonitor(session.logPath, session.buildEnv);
  await waitForDbusMonitorReady(session.dbusMonitor, timeoutMs);
  await triggerNotificationSmoke(session.browser, timeoutMs);
  await waitForDbusMonitorToken(session.dbusMonitor, timeoutMs);
  console.log("notification_dbus=ok");

  console.log("window_state_path_contract=ok");
  console.log("run_dir=artifact");
}

async function waitForAuthScreen(browser, timeout) {
  const authScreen = await browser.$('[data-testid="auth-screen"]');
  await authScreen.waitForDisplayed({ timeout });
}

async function waitForLocalLoginReady(browser, timeout) {
  const startedAt = Date.now();
  let lastTitle = "";
  let selectedRoom = false;
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`local GUI login reported errors. Last title: ${lastTitle}`);
    }
    if (qaStatusIsReady(status, false, true)) {
      return lastTitle;
    }
    if (shouldSelectFirstRoom(status, selectedRoom)) {
      selectedRoom = await selectFirstRoom(browser);
    }
    await sleep(250);
  }
  throw new Error(`local GUI login did not reach a ready state. Last title: ${lastTitle}`);
}

async function waitForLocalSendSuccess(browser, timeout) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`local GUI send reported errors. Last title: ${lastTitle}`);
    }
    if (status.send === "failed") {
      throw new Error(`local GUI send failed. Last title: ${lastTitle}`);
    }
    if (qaStatusHasSendSuccess(status)) {
      return lastTitle;
    }
    await sleep(250);
  }
  throw new Error(`local GUI send did not reach send=sent. Last title: ${lastTitle}`);
}

async function selectFirstRoom(browser) {
  const roomItems = await browser.$$("[data-testid='room-item'], .room-item");
  if (!roomItems.length) {
    return false;
  }
  try {
    await roomItems[0].waitForDisplayed({ timeout: 1000 });
    await roomItems[0].click();
    return true;
  } catch {
    return false;
  }
}

function shouldSelectFirstRoom(status, selectedRoom) {
  if (selectedRoom) {
    return false;
  }
  if (status.session !== "ready" || status.rooms <= 0) {
    return false;
  }
  return status.active_room === false || status.timeline_subscribed === false;
}

async function writeLocalLoginPipe(path, credentials) {
  const payloadObject = {
    homeserver: credentials.homeserver,
    username: credentials.username,
    password: credentials.password,
    device_display_name: credentials.deviceName
  };
  const payload = JSON.stringify(payloadObject) + "\n";
  await writeSensitivePayloadToPath(path, payload, 10000);
}

async function requestQaLogout(path) {
  if (!path) {
    throw new Error("local GUI logout scenario requires a QA control pipe");
  }
  const payload = JSON.stringify({ command: "logout" }) + "\n";
  await writeSensitivePayloadToPath(path, payload, 10000);
}

async function submitLoginForm(browser, credentials, timeout) {
  const homeserverInput = await browser.$('input[name="homeserver"]');
  await homeserverInput.waitForDisplayed({ timeout });
  await homeserverInput.setValue(credentials.homeserver);

  const usernameInput = await browser.$('input[name="username"]');
  await usernameInput.waitForDisplayed({ timeout });
  await usernameInput.setValue(credentials.username);

  const passwordInput = await browser.$('input[name="password"]');
  await passwordInput.waitForDisplayed({ timeout });
  await passwordInput.setValue(credentials.password);

  const deviceNameInput = await browser.$('input[name="deviceName"]');
  await deviceNameInput.waitForDisplayed({ timeout });
  await deviceNameInput.setValue(`${credentials.deviceName} Relogin`);

  const submit = await browser.$("button.auth-submit");
  await submit.waitForDisplayed({ timeout });
  await submit.click();
}

function createNamedPipe(path) {
  execFileSync("mkfifo", [path], { stdio: "ignore" });
}

function guiScenarioServerKind() {
  if (serverOption === "conduit" || serverOption === "tuwunel") {
    return serverOption;
  }
  throw new Error("--server must be conduit or tuwunel for local GUI scenarios");
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
        setTimeout(() => reject(new Error("local GUI login FIFO write timed out")), timeout)
      )
    ]);
  } finally {
    await handle?.close();
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
    "DBUS_SESSION_BUS_ADDRESS",
    "DISPLAY",
    "GDK_BACKEND",
    "HOME",
    "KOUSHI_CORE_ACTOR_TRACE",
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
  env.KOUSHI_RESTORE_SESSION = qaProfile !== undefined ? "1" : "0";
  env.KOUSHI_SKIP_SAVED_SESSIONS = "1";
  env.KOUSHI_SKIP_KEYCHAIN_PERSISTENCE = "1";
  env.KOUSHI_DATA_DIR = dataDir;
  env.KOUSHI_QA_TITLE = "1";
  env.VITE_KOUSHI_QA_TITLE = "1";
  env.KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR = join(dataDir, "qa-credential-store");
  if (guiScenario === "local-invites-dm") {
    env.KOUSHI_QA_FORCE_SYNC_BACKEND = "legacy";
  }
  env.NO_COLOR = "1";
  if (qaProfile !== undefined) {
    env.KOUSHI_RESTORE_SESSION = "1";
  }
  if (qaLoginPipePath) {
    env.KOUSHI_QA_LOGIN_PIPE = qaLoginPipePath;
  } else if (realLoginFromStdin) {
    env.KOUSHI_QA_LOGIN_PIPE = join(dataDir, "qa-login.pipe");
  }
  if (qaControlPipePath) {
    env.KOUSHI_QA_CONTROL_PIPE = qaControlPipePath;
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
  writeFileSync(passwdPath, `koushi-desktop:x:${uid}:${gid}:Koushi:/tmp:/bin/sh\n`);
  writeFileSync(groupPath, `koushi-desktop:x:${gid}:\n`);

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
  try {
    await waitForDisplaySocket(display, timeoutMs);
    return { child, display };
  } catch (error) {
    terminateProcessGroup(child, "SIGTERM");
    await settleChild(child);
    throw error;
  }
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
    candidates.push(join(cargoTargetDir, "debug", "koushi-desktop"));
  }
  candidates.push(join(desktopDir, "src-tauri", "target", "debug", "koushi-desktop"));
  candidates.push(join(repoRoot, "target", "debug", "koushi-desktop"));
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found) {
    throw new Error(`unable to resolve debug Tauri binary. Checked: ${candidates.join(", ")}`);
  }
  return found;
}

/**
 * Resolve the debug Tauri binary to drive, building it first unless
 * `--skip-build` is passed. `--skip-build` is the fast inner-loop path: it
 * reuses an already-built binary (via `--app-binary=PATH` or the default
 * debug target) so iterating on a scenario does not pay the full Tauri
 * rebuild each time.
 */
async function ensureAppBinary({ cwd, env, logPath }) {
  if (!args.has("--skip-build")) {
    await runLoggedCommand("npm", ["run", "tauri", "build", "--", "--debug", "--no-bundle"], {
      cwd,
      env,
      logPath,
      label: "tauri build"
    });
  }
  const explicit = optionValue("--app-binary");
  const appBinary = explicit
    ? isAbsolute(explicit)
      ? explicit
      : resolve(explicit)
    : resolveDebugAppBinary();
  if (!existsSync(appBinary)) {
    throw new Error(
      `app binary not found at ${appBinary}. With --skip-build, pass --app-binary=PATH ` +
        `or build once first: npm --prefix apps/desktop run tauri build -- --debug --no-bundle`
    );
  }
  return appBinary;
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
    if (
      ["rooms", "spaces", "timeline_items", "pinned", "pin_ops", "errors", "unread", "badge"].includes(key)
    ) {
      status[key] = Number(value);
    } else if (["active_room", "timeline_room", "timeline_subscribed"].includes(key)) {
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
    status.timeline_room !== false &&
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

function resolveArtifactRoot(artifactDirOption) {
  if (!artifactDirOption) {
    return join(repoRoot, "artifacts", "linux-gui-qa");
  }
  return isAbsolute(artifactDirOption) ? artifactDirOption : resolve(repoRoot, artifactDirOption);
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

        const notification = new notificationApi("Koushi QA", {
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

function safeTimestamp() {
  return `${Date.now()}_${process.pid}`.replaceAll("-", "_");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function printUsage() {
  console.log(
    "Usage: node scripts/desktop-linux-gui-qa.mjs --list|--check-tools|--child-env|--child-env-keys|--print-artifact-root|--print-real-login-transport|--print-webdriver-capabilities --app-binary=PATH|--qa-title-panel=TITLE|--qa-title-panel-ready=TITLE [--required-panel=PANEL]|--qa-title-ready=TITLE|--qa-title-attention-ready=TITLE|--qa-window-state-ready=PATH|--qa-title-send-ready=TITLE|--qa-title-ready-require-recovered=TITLE|--run [--skip-build] [--real-login-from-stdin] [--qa-profile=NAME] [--allow-empty-timeline] [--artifact-dir=PATH] [--timeout-ms=MS]"
  );
}
