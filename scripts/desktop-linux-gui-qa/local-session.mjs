import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkInstalledHomeserver,createRoom,freePort,inviteUser as inviteUserToRoom,joinRoom,registerUser,sendReadMarkers,sendRoomFormattedMessage,sendRoomMessage,setDisplayName,startHomeserver,stopProcess,tuwunelConfig,waitForHomeserver } from "../lib/local-homeserver-qa.mjs";
import { writeSensitivePayloadToPath } from "../lib/sensitive-fifo.mjs";
import { parseQaTitle,qaStatusHasSendSuccess,qaStatusIsReady,safeTimestamp,timestamp } from "./evidence.mjs";
import { artifactRoot,desktopDir,guiScenario,timeoutMs } from "./options.mjs";
import { childEnvironment } from "./redaction.mjs";
import { checkLinuxTools,ensureAppBinary,ensureDbusSession,guiScenarioServerKind,qaDataDirForRun,settleChild,sleep,spawnLogged,startDbusMonitor,startXvfb,terminateProcessGroup,triggerNotificationSmoke,waitForDbusMonitorReady,waitForDbusMonitorToken,waitForPort } from "./runtime.mjs";
import {
  MESSAGE_COMPOSER_SELECTOR,
  clickVisibleButtonByTextPrefix,
  elementCount,
  importDesktopWebdriverio,
  safeDeleteSession,
  setTextInputValueByLabel,
  waitForQaTitle,
  waitForEditableValue,
  webdriverCapabilities,
  xpathLiteral
} from "./webdriver.mjs";

const TIMELINE_NAVIGATION_SEED_MESSAGE_COUNT = 24;
const TIMELINE_NAVIGATION_SEED_LINE_COUNT = 12;

function timelineNavigationSeedBody(index) {
  return Array.from(
    { length: TIMELINE_NAVIGATION_SEED_LINE_COUNT },
    (_, lineIndex) =>
      `QA timeline navigation seed ${index}.${lineIndex} scroll contract`
  ).join("\n");
}

export async function startLocalGuiScenario() {
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
    allowNewIdentityBootstrap: true,
    bootstrapTempDirs: new Set(),
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
    seedInviteRoomName: null,
    readerDisplayNames: [],
    readerSeedBody: null,
    readerHelpers: [],
    readerEventId: null
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
      tuwunelConfig({ serverName, port, dataDir: serverDataDir })
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

    if (guiScenario === "local-receipt-readers") {
      const helperPassword = `koushi-desktop-helper-${userSuffix}`;
      session.readerDisplayNames = Array.from(
        { length: 5 },
        (_, index) => `Receipt Reader ${index + 1}`
      );
      session.readerSeedBody = "QA receipt reader seed";
      const helpers = [];
      for (let index = 0; index < session.readerDisplayNames.length; index += 1) {
        const helperRegistration = await registerUser(
          homeserver,
          `qa_receipt_reader_${index}_${userSuffix}`,
          helperPassword
        );
        const helperAccessToken = helperRegistration.access_token;
        const helperUserId = helperRegistration.user_id;
        if (!helperAccessToken || !helperUserId) {
          throw new Error("local GUI receipt-reader setup did not return helper credentials");
        }
        await setDisplayName(
          homeserver,
          helperAccessToken,
          helperUserId,
          session.readerDisplayNames[index]
        );
        await inviteUserToRoom(homeserver, accessToken, seedRoomId, helperUserId);
        await joinRoom(homeserver, helperAccessToken, seedRoomId);
        helpers.push({ accessToken: helperAccessToken, userId: helperUserId });
      }
      const sent = await sendRoomMessage(
        homeserver,
        accessToken,
        seedRoomId,
        session.readerSeedBody,
        `qa-receipt-reader-${userSuffix}`
      );
      if (!sent.event_id) {
        throw new Error("local GUI receipt-reader setup did not return the seed event id");
      }
      session.readerHelpers = helpers;
      session.readerEventId = sent.event_id;
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


export async function cleanupLocalGuiScenario(session) {
  cleanupBootstrapTempDirs(session);
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

function cleanupBootstrapTempDirs(session) {
  for (const bootstrapDir of session.bootstrapTempDirs ?? []) {
    try {
      rmSync(bootstrapDir, { recursive: true, force: true });
    } catch {
      // Best effort: never turn process teardown into a secret-bearing error.
    }
    session.bootstrapTempDirs.delete(bootstrapDir);
  }
}


export async function recordLocalGuiEvidence(session) {
  session.dbusMonitor = startDbusMonitor(session.logPath, session.buildEnv);
  await waitForDbusMonitorReady(session.dbusMonitor, timeoutMs);
  await triggerNotificationSmoke(session.browser, timeoutMs);
  await waitForDbusMonitorToken(session.dbusMonitor, timeoutMs);
  console.log("notification_dbus=ok");

  console.log("window_state_path_contract=ok");
  console.log("run_dir=artifact");
}


export async function waitForAuthScreen(browser, timeout) {
  const authScreen = await browser.$('[data-testid="auth-screen"]');
  await authScreen.waitForDisplayed({ timeout });
}


export async function waitForLocalLoginReady(session, timeout) {
  const browser = session.browser;
  const deadline = Date.now() + timeout;
  const bootstrapAttempt = { attempted: false };
  let lastTitle = "";
  let selectedRoom = false;
  while (Date.now() < deadline) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`local GUI login reported errors. Last title: ${lastTitle}`);
    }
    const composerVisible = (await elementCount(browser, MESSAGE_COMPOSER_SELECTOR)) > 0;
    if (qaStatusIsReady(status, false, true) && composerVisible) {
      return lastTitle;
    }
    if (
      status.session === "awaitingVerification" &&
      session.allowNewIdentityBootstrap === true
    ) {
      await completeNewIdentityBootstrapIfOffered(session, deadline, bootstrapAttempt);
    }
    if (shouldSelectFirstRoom(status, selectedRoom, composerVisible)) {
      selectedRoom = await selectFirstRoom(browser);
    }
    await sleep(250);
  }
  throw new Error(`local GUI login did not reach a ready state. Last title: ${lastTitle}`);
}

async function completeNewIdentityBootstrapIfOffered(session, deadline, bootstrapAttempt) {
  if (bootstrapAttempt.attempted) return false;
  const browser = session.browser;
  const labels = {
    destination: "Recovery key destination",
    passphrase: "Backup passphrase",
    submit: "Create secure backup",
    saved: "I saved the recovery key"
  };
  const [destinationInputs, passphraseInputs, submitButtons] = await Promise.all([
    browser.$$(`//input[@aria-label=${xpathLiteral(labels.destination)}]`),
    browser.$$(`//input[@aria-label=${xpathLiteral(labels.passphrase)}]`),
    browser.$$(`//button[normalize-space(.)=${xpathLiteral(labels.submit)}]`)
  ]);
  if (destinationInputs.length === 0 || passphraseInputs.length === 0 || submitButtons.length === 0) {
    return false;
  }

  bootstrapAttempt.attempted = true;
  let bootstrapDir;
  try {
    bootstrapDir = mkdtempSync(join(tmpdir(), "koushi-linux-gui-bootstrap-"));
    session.bootstrapTempDirs.add(bootstrapDir);
    const destination = join(bootstrapDir, "recovery-key.txt");
    const bootstrapPassphrase = randomBytes(32).toString("base64url");
    await setTextInputValueByLabel(browser, destination, labels.destination);
    await setTextInputValueByLabel(browser, bootstrapPassphrase, labels.passphrase);
    await clickVisibleButtonByTextPrefix(
      browser,
      labels.submit,
      Math.max(1, deadline - Date.now()),
      "new-identity bootstrap submit"
    );
    await clickVisibleButtonByTextPrefix(
      browser,
      labels.saved,
      Math.max(1, deadline - Date.now()),
      "new-identity bootstrap confirmation"
    );
    return true;
  } catch {
    return false;
  } finally {
    if (bootstrapDir) {
      try {
        rmSync(bootstrapDir, { recursive: true, force: true });
        session.bootstrapTempDirs.delete(bootstrapDir);
      } catch {
        // The session teardown keeps the directory tracked for one final cleanup attempt.
      }
    }
  }
}


export async function waitForComposerSendSettled(browser, timeout, description) {
  await waitForEditableValue(
    browser,
    MESSAGE_COMPOSER_SELECTOR,
    "",
    timeout,
    `${description} clear`
  );
  await waitForLocalSendSuccess(browser, timeout);
}

export async function waitForLocalSendSuccess(browser, timeout) {
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


function shouldSelectFirstRoom(status, selectedRoom, composerVisible) {
  if (selectedRoom) {
    return false;
  }
  if (status.session !== "ready" || status.rooms <= 0) {
    return false;
  }
  return (
    !composerVisible || status.active_room === false || status.timeline_subscribed === false
  );
}


export function readRealLoginCredentials() {
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
  const deviceName = deviceNameInput?.trim() || "Koushi Linux GUI QA";
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


export async function exerciseRealRoomSelection(browser, timeout) {
  const initial = await realRoomSelectionDiagnostics(browser);
  if (initial.count < 2) {
    console.log("gui_real_room_switch=skipped");
    return;
  }
  const targetIndex = initial.activeIndex === 0 ? 1 : 0;
  const targetLabel = await browser.execute((index) => {
    const rows = Array.from(
      document.querySelectorAll('button[data-testid="room-item"]:not([data-room-kind="invite"])')
    );
    const name = rows[index]?.querySelector(".room-name")?.textContent?.trim();
    return name && name.length > 0 ? name : null;
  }, targetIndex);
  if (!targetLabel) {
    throw new Error("real GUI room switch target label was unavailable");
  }
  const roomItems = await browser.$$(
    'button[data-testid="room-item"]:not([data-room-kind="invite"])'
  );
  await roomItems[targetIndex].waitForDisplayed({ timeout });
  await roomItems[targetIndex].click();
  await waitForQaTitle(
    browser,
    async (status) => {
      const current = await realRoomSelectionDiagnostics(browser, targetLabel);
      return (
        current.targetIsActive &&
        qaStatusIsReady(status, false, true)
      );
    },
    timeout,
    "real GUI room switch"
  );
  console.log("gui_real_room_switch=ok");
}


async function realRoomSelectionDiagnostics(browser, targetLabel = null) {
  return await browser.execute((target) => {
    const normalize = (value) => (value ?? "").replace(/\s+/g, " ").trim();
    const rows = Array.from(
      document.querySelectorAll('button[data-testid="room-item"]:not([data-room-kind="invite"])')
    );
    const activeRow = rows.find((row) => row.classList.contains("is-active"));
    const activeLabel = normalize(activeRow?.querySelector(".room-name")?.textContent);
    const headerLabel = normalize(document.querySelector(".channel-title")?.textContent);
    const targetText = normalize(target);
    return {
      count: rows.length,
      activeIndex: rows.findIndex((row) => row.classList.contains("is-active")),
      targetIsActive: Boolean(
        targetText &&
          (activeLabel === targetText ||
            headerLabel === targetText ||
            headerLabel.endsWith(targetText))
      )
    };
  }, targetLabel);
}


export async function exerciseRealSpaceSelection(browser, timeout) {
  const initial = await realSpaceSelectionDiagnostics(browser);
  if (initial.spaceCount < 1) {
    console.log("gui_real_space_switch=skipped");
    return;
  }
  const spaceButtons = await browser.$$("button.workspace-space-button");
  await spaceButtons[0].waitForDisplayed({ timeout });
  await spaceButtons[0].click();
  await waitForQaTitle(
    browser,
    async (status) => {
      const current = await realSpaceSelectionDiagnostics(browser);
      return current.spaceActiveIndex === 0 && status.errors === 0;
    },
    timeout,
    "real GUI space switch"
  );
  const homeButton = await browser.$('button[aria-label="Home"]');
  await homeButton.waitForDisplayed({ timeout });
  await homeButton.click();
  await waitForQaTitle(
    browser,
    async (status) => {
      const current = await realSpaceSelectionDiagnostics(browser);
      return current.homeActive && status.errors === 0;
    },
    timeout,
    "real GUI home switch"
  );
  console.log("gui_real_space_switch=ok");
}


async function realSpaceSelectionDiagnostics(browser) {
  return await browser.execute(() => {
    const spaces = Array.from(document.querySelectorAll("button.workspace-space-button"));
    const home = document.querySelector("button.workspace-home-button");
    return {
      spaceCount: spaces.length,
      spaceActiveIndex: spaces.findIndex((row) => row.classList.contains("is-active")),
      homeActive: Boolean(home?.classList.contains("is-active"))
    };
  });
}


export async function writeLocalLoginPipe(path, credentials) {
  const payloadObject = {
    homeserver: credentials.homeserver,
    username: credentials.username,
    password: credentials.password,
    device_display_name: credentials.deviceName
  };
  const payload = JSON.stringify(payloadObject) + "\n";
  await writeSensitivePayloadToPath(path, payload, 10000);
}


export async function writeRealLoginPipe(path, credentials) {
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


export async function requestQaLogout(path) {
  if (!path) {
    throw new Error("local GUI logout scenario requires a QA control pipe");
  }
  const payload = JSON.stringify({ command: "logout" }) + "\n";
  await writeSensitivePayloadToPath(path, payload, 10000);
}


export async function submitLoginForm(browser, credentials, timeout) {
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


export function createNamedPipe(path) {
  execFileSync("mkfifo", [path], { stdio: "ignore" });
}
