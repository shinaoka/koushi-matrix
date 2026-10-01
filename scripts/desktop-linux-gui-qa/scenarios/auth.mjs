import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { createRoom,freePort,inviteUser as inviteUserToRoom } from "../../lib/local-homeserver-qa.mjs";
import { parseQaTitle,requireNonEmptyFile,timestamp } from "../evidence.mjs";
import { cleanupLocalGuiScenario,createNamedPipe,exerciseRealRoomSelection,exerciseRealSpaceSelection,readRealLoginCredentials,recordLocalGuiEvidence,requestQaLogout,startLocalGuiScenario,submitLoginForm,waitForAuthScreen,waitForLocalLoginReady,writeLocalLoginPipe,writeRealLoginPipe } from "../local-session.mjs";
import { artifactRoot,desktopDir,realLoginFromStdin,timeoutMs } from "../options.mjs";
import { childEnvironment } from "../redaction.mjs";
import { checkLinuxTools,ensureAppBinary,ensureDbusSession,qaDataDirForRun,settleChild,spawnLogged,startDbusMonitor,startXvfb,terminateProcessGroup,triggerNotificationSmoke,waitForDbusMonitorReady,waitForDbusMonitorToken,waitForPort,waitForSignedOutTitle } from "../runtime.mjs";
import { elementCount,importDesktopWebdriverio,safeDeleteSession,waitForDocumentText,waitForElementCountGreaterThan,waitForQaTitle,webdriverCapabilities } from "../webdriver.mjs";

export const INVITE_ACCEPT_BUTTON_SELECTOR = 'button[aria-label="Accept"]';

export async function runSignedOutScenario() {
  checkLinuxTools();
  const realLogin = realLoginFromStdin ? await readRealLoginCredentials() : null;

  const runDir = join(artifactRoot, timestamp());
  const screenshotDir = join(runDir, "screenshots");
  const dataDir = qaDataDirForRun(runDir);
  const logPath = join(runDir, "run.log");
  const qaLoginPipePath = realLogin ? join(dataDir, "qa-login.pipe") : null;
  const qaControlPipePath = realLogin ? join(dataDir, "qa-control.pipe") : null;
  mkdirSync(screenshotDir, { recursive: true });
  mkdirSync(dataDir, { recursive: true });
  if (qaLoginPipePath) {
    createNamedPipe(qaLoginPipePath);
  }
  if (qaControlPipePath) {
    createNamedPipe(qaControlPipePath);
  }

  const baseEnv = childEnvironment(dataDir, qaLoginPipePath, qaControlPipePath);
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
  let realLoginCleanupRequired = false;
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

    if (realLogin) {
      await writeRealLoginPipe(qaLoginPipePath, realLogin);
      realLoginCleanupRequired = true;
      await waitForLocalLoginReady(
        { browser, allowNewIdentityBootstrap: false, bootstrapTempDirs: new Set() },
        timeoutMs
      );
      console.log("gui_real_login=ok");
      await exerciseRealRoomSelection(browser, timeoutMs);
      await exerciseRealSpaceSelection(browser, timeoutMs);
    }

    dbusMonitor = startDbusMonitor(logPath, buildEnv);
    await waitForDbusMonitorReady(dbusMonitor, timeoutMs);
    await triggerNotificationSmoke(browser, timeoutMs);
    await waitForDbusMonitorToken(dbusMonitor, timeoutMs);
    console.log("notification_dbus=ok");

    console.log("run_dir=artifact");
  } finally {
    try {
      if (qaControlPipePath && realLoginCleanupRequired && browser) {
        try {
          await requestQaLogout(qaControlPipePath);
          await waitForSignedOutTitle(browser, timeoutMs);
          console.log("gui_real_logout=ok");
        } catch (cleanupError) {
          console.error(`real login logout cleanup failed: ${cleanupError.message}`);
        }
      }
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

export async function runLocalLoginScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_login=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalLogoutReloginScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    await requestQaLogout(session.qaControlPipePath);
    await waitForSignedOutTitle(session.browser, timeoutMs);
    await waitForAuthScreen(session.browser, timeoutMs);
    console.log("gui_local_logout=ok");

    session.allowNewIdentityBootstrap = false;
    await submitLoginForm(session.browser, session.credentials, timeoutMs);
    await waitForLocalLoginReady(session, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_relogin=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalInvitesDmScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

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
    const acceptButton = await session.browser.$(INVITE_ACCEPT_BUTTON_SELECTOR);
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
