import { randomBytes } from "node:crypto";
import { join } from "node:path";
import { safeTimestamp } from "../evidence.mjs";
import { cleanupLocalGuiScenario,recordLocalGuiEvidence,startLocalGuiScenario,waitForAuthScreen,waitForLocalLoginReady,writeLocalLoginPipe } from "../local-session.mjs";
import { timeoutMs } from "../options.mjs";
import { clickKeyManagementFormButton,clickVisibleButtonByTextPrefix,ensureUserSettingsKeyManagementOpen,setKeyManagementFormInput,waitForDocumentText,waitForDocumentTheme,waitForElementAttribute,waitForFileExists,waitForKeyManagementStatus } from "../webdriver.mjs";

export async function runLocalSettingsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

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

export async function runLocalE2eeKeyManagementScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    await ensureUserSettingsKeyManagementOpen(session.browser, timeoutMs);

    const keyFilePath = join(session.runDir, "room-keys.txt");
    const keyFilePassphrase = `koushi-key-transfer-${safeTimestamp()}`;

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

    await exerciseSecureBackupRevealFlows(session);
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

// The secure-backup half of `local-e2ee-key-management`, also runnable alone.
export async function runLocalSecureBackupScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await ensureUserSettingsKeyManagementOpen(session.browser, timeoutMs);
    await exerciseSecureBackupRevealFlows(session);
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

// #1049: the new-identity login already created the Secure Backup through
// the gate's on-screen reveal (Create, then "I saved the recovery key"), so
// Settings must report it enabled; Core rejects a second InitialSetup. The
// Settings on-screen reveal is exercised by a passphrase change. The revealed
// key is never read, saved, or logged; the optional save is skipped.
async function exerciseSecureBackupRevealFlows(session) {
  if (!session.bootstrapPassphrase) {
    throw new Error("local GUI login did not complete the new-identity bootstrap reveal");
  }
  await waitForKeyManagementStatus(
    session.browser,
    "secure-backup-state",
    ["Enabled"],
    timeoutMs,
    "local GUI secure backup created by the bootstrap reveal"
  );
  console.log("gui_secure_backup_setup=ok");

  const formLabel = "Change secure backup passphrase";
  await setSecretFormField(
    session.browser,
    formLabel,
    "Current recovery secret",
    session.bootstrapPassphrase
  );
  await setSecretFormField(
    session.browser,
    formLabel,
    "New secure backup passphrase",
    randomBytes(32).toString("base64url")
  );
  await clickKeyManagementFormButton(
    session.browser,
    formLabel,
    "Update secure backup passphrase",
    timeoutMs
  );
  await waitForKeyManagementStatus(
    session.browser,
    "secure-backup-passphrase-change-state",
    ["Changed"],
    timeoutMs,
    "local GUI passphrase change reveal"
  );
  await clickVisibleButtonByTextPrefix(
    session.browser,
    "I saved the recovery key",
    timeoutMs,
    "local GUI passphrase change confirmation"
  );
  await waitForKeyManagementStatus(
    session.browser,
    "secure-backup-passphrase-change-state",
    ["No passphrase change"],
    timeoutMs,
    "local GUI passphrase change confirmed"
  );
  console.log("gui_secure_backup_passphrase_change=ok");
}

// Sets a secret field through the native value setter (as the login gate
// does) and reports only whether the value landed intact, never the value.
async function setSecretFormField(browser, formLabel, fieldLabel, value) {
  const landed = await browser.execute(({ form, field, nextValue }) => {
    const input = Array.from(document.querySelectorAll(`form[aria-label="${form}"] label`))
      .find((label) => label.querySelector("span")?.textContent?.trim() === field)
      ?.querySelector("input");
    if (!(input instanceof HTMLInputElement)) return "missing-input";
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, nextValue);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return input.value === nextValue ? "set" : "value-mismatch";
  }, { form: formLabel, field: fieldLabel, nextValue: value });
  if (landed !== "set") {
    throw new Error(`local GUI ${fieldLabel} input failed: ${landed}`);
  }
}
