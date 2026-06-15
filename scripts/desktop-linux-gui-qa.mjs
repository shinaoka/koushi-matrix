#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { open } from "node:fs/promises";
import { createRequire } from "node:module";
import * as net from "node:net";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  checkInstalledHomeserver,
  conduitConfig,
  createRoom,
  freePort,
  inviteUser as inviteUserToRoom,
  joinRoom,
  registerUser,
  sendRoomMessage,
  setDisplayName,
  startHomeserver,
  stopProcess,
  tuwunelConfig,
  waitForHomeserver
} from "./lib/local-homeserver-qa.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopDir = join(repoRoot, "apps", "desktop");
const desktopPackageRequire = createRequire(new URL("../apps/desktop/package.json", import.meta.url));
const checks = [
  "scenario signed-out",
  "scenario local-login",
  "scenario local-send",
  "scenario local-create-room",
  "scenario local-create-space",
  "scenario local-invites-dm",
  "scenario local-reply",
  "scenario local-media",
  "scenario local-room-tags",
  "scenario local-room-management",
  "scenario local-activity",
  "scenario local-explore",
  "scenario local-message-actions",
  "scenario local-composer",
  "scenario local-settings",
  "verify local-settings trust section",
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
  if (guiScenario === "local-composer") {
    await runLocalComposerScenario();
    return;
  }
  if (guiScenario === "local-settings") {
    await runLocalSettingsScenario();
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
    const message = `Matrix Desktop GUI QA ${timestamp()}`;
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
    const fixturePath = join(session.runDir, filename);
    writeFileSync(fixturePath, "Matrix Desktop Linux GUI media fixture\n", "utf8");
    const fileInputSelector = 'input[type="file"][aria-label="Attach file input"]';
    await setSyntheticFileInput(
      session.browser,
      fileInputSelector,
      fixturePath,
      filename,
      "text/plain",
      "Matrix Desktop Linux GUI media fixture"
    );
    await waitForElementCountGreaterThan(
      session.browser,
      ".message-media",
      baselineMediaRows,
      timeoutMs,
      "local GUI media render"
    );
    await waitForDocumentText(session.browser, [filename], timeoutMs, "local GUI media render");

    const downloadButton = await session.browser.$(`button[aria-label="Download ${filename}"]`);
    await downloadButton.waitForDisplayed({ timeout: timeoutMs });
    await downloadButton.click();
    await waitForQaTitle(
      session.browser,
      (status) => status.errors === 0,
      timeoutMs,
      "local GUI media download"
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_media=ok");
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

    const baselineMessages = await elementCount(session.browser, ".message");
    const forwardActionButton = await waitForLatestMessageActionButton(session.browser, timeoutMs);
    await forwardActionButton.moveTo();
    await forwardActionButton.waitForDisplayed({ timeout: timeoutMs });
    await forwardActionButton.click();
    await clickVisibleMenuItemByText(session.browser, "Forward", timeoutMs);
    await clickVisibleMenuItemByText(session.browser, "QA Seed Room", timeoutMs);
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      baselineMessages,
      timeoutMs,
      "local GUI message forward"
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_message_forward=ok");
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

async function activeRoomDiagnostics(browser) {
  return browser.execute(() => {
    const textFor = (element) =>
      (element.textContent ?? "").replace(/\s+/g, " ").trim();
    const roomRows = Array.from(document.querySelectorAll('button[data-testid="room-item"]'));
    return {
      title: document.title,
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
      (element.textContent ?? "").replace(/\s+/g, " ").trim();
    const labelsFor = (selector) =>
      Array.from(document.querySelectorAll(selector))
        .map((element) => element.getAttribute("aria-label") ?? textFor(element))
        .filter(Boolean)
        .slice(0, 8);
    const active = document.activeElement;
    return {
      title: document.title,
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

function roomButtonXpath(sectionId, roomName) {
  return `//section[@data-room-section=${xpathLiteral(sectionId)}]//button[@data-testid="room-item"][.//span[normalize-space()=${xpathLiteral(roomName)}]]`;
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
    (cssSelector, fileName, type, text) => {
      const input = document.querySelector(cssSelector);
      if (!(input instanceof HTMLInputElement)) {
        return { ok: false, reason: "input_missing" };
      }
      if (typeof DataTransfer !== "function") {
        return { ok: false, reason: "data_transfer_unavailable" };
      }
      const transfer = new DataTransfer();
      transfer.items.add(new File([text], fileName, { type }));
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
    qaLoginPipePath: null,
    runDir,
    serverProcess: null,
    tauriDriver: null,
    xvfb: null,
    credentials: null,
    dmTargetUserId: null,
    helperAccessToken: null,
    composerMentionDisplayName: null,
    directoryRoomName: null,
    roomManagementTopic: null,
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
    const password = `matrix-desktop-local-${userSuffix}`;
    const registration = await registerUser(homeserver, username, password);
    const accessToken = registration.access_token;
    const userId = registration.user_id;
    if (!accessToken) {
      throw new Error("local GUI setup did not return an access token");
    }
    if (!userId) {
      throw new Error("local GUI setup did not return a user id");
    }
    const seedRoom = await createRoom(homeserver, accessToken, { name: "QA Seed Room" });
    const seedRoomId = seedRoom.room_id;
    if (!seedRoomId) {
      throw new Error("local GUI setup did not return a seed room id");
    }
    session.seedRoomId = seedRoomId;
    session.primaryUserId = userId;

    if (guiScenario === "local-composer") {
      const helperUsername = `qa_mention_${userSuffix}`;
      const helperPassword = `matrix-desktop-helper-${userSuffix}`;
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

    if (guiScenario === "local-room-management") {
      const helperUsername = `qa_management_${userSuffix}`;
      const helperPassword = `matrix-desktop-helper-${userSuffix}`;
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
      const helperPassword = `matrix-desktop-helper-${userSuffix}`;
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
      const helperPassword = `matrix-desktop-helper-${userSuffix}`;
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

    const baseEnv = childEnvironment(appDataDir, session.qaLoginPipePath);
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
      deviceName: "Matrix Desktop Local QA"
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

  const windowStatePath = join(session.appDataDir, "app-shell", "window-state.json");
  console.log(`window_state_path=${windowStatePath}`);
  console.log("window_state_path_contract=ok");
  console.log(`run_dir=${session.runDir}`);
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
  if (guiScenario === "local-invites-dm") {
    env.MATRIX_DESKTOP_QA_FORCE_SYNC_BACKEND = "legacy";
  }
  env.NO_COLOR = "1";
  if (qaProfile !== undefined) {
    env.MATRIX_DESKTOP_RESTORE_SESSION = "1";
  }
  if (qaLoginPipePath) {
    env.MATRIX_DESKTOP_QA_LOGIN_PIPE = qaLoginPipePath;
  } else if (realLoginFromStdin) {
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
    if (["rooms", "spaces", "timeline_items", "errors", "unread", "badge"].includes(key)) {
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

function safeTimestamp() {
  return `${Date.now()}_${process.pid}`.replaceAll("-", "_");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function printUsage() {
  console.log(
    "Usage: node scripts/desktop-linux-gui-qa.mjs --list|--check-tools|--child-env|--child-env-keys|--print-artifact-root|--print-real-login-transport|--print-webdriver-capabilities --app-binary=PATH|--qa-title-panel=TITLE|--qa-title-panel-ready=TITLE [--required-panel=PANEL]|--qa-title-ready=TITLE|--qa-title-attention-ready=TITLE|--qa-window-state-ready=PATH|--qa-title-send-ready=TITLE|--qa-title-ready-require-recovered=TITLE|--run [--real-login-from-stdin] [--qa-profile=NAME] [--allow-empty-timeline] [--artifact-dir=PATH] [--timeout-ms=MS]"
  );
}
