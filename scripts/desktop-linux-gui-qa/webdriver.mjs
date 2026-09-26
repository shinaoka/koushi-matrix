import { existsSync,writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { deflateSync } from "node:zlib";
import { parseQaTitle } from "./evidence.mjs";
import { desktopPackageRequire,timeoutMs } from "./options.mjs";
import { sleep } from "./runtime.mjs";

const pngCrc32Table = Array.from({ length: 256 }, (_, index) => {
  let value = index;
  for (let bit = 0; bit < 8; bit += 1) {
    value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  }
  return value >>> 0;
});

export async function waitForQaTitle(browser, predicate, timeout, description) {
  const startedAt = Date.now();
  let lastTitle = "";
  while (Date.now() - startedAt < timeout) {
    lastTitle = await browser.execute(() => document.title);
    const status = parseQaTitle(lastTitle);
    if (status.errors > 0) {
      throw new Error(`${description} reported errors. Last title: ${lastTitle}`);
    }
    if (await predicate(status)) {
      return lastTitle;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not reach its expected state. Last title: ${lastTitle}`);
}


export async function waitForTimelineFocusedContextReady(browser, timeout, description) {
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


export async function waitForElementAttribute(browser, selector, attribute, expected, timeout, description) {
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


export const MESSAGE_COMPOSER_SELECTOR = '.composer-inline-editor[role="textbox"]';

export async function waitForEditableValue(browser, selector, expected, timeout, description) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeout) {
    const value = await browser.execute((cssSelector) => {
      const editable = document.querySelector(cssSelector);
      if (editable instanceof HTMLInputElement || editable instanceof HTMLTextAreaElement) {
        return editable.value;
      }
      return editable instanceof HTMLElement && editable.isContentEditable
        ? editable.textContent ?? ""
        : null;
    }, selector);
    if (value === expected) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not reach the expected private-safe editable state`);
}


export async function waitForDocumentTheme(browser, expected, timeout) {
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


export async function waitForDocumentText(browser, expectedTexts, timeout, description) {
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


export async function ensureUserSettingsKeyManagementOpen(browser, timeout) {
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


export async function setKeyManagementFormInput(browser, formLabel, fieldLabel, value) {
  const selector = keyManagementFormInputXpath(formLabel, fieldLabel);
  const input = await browser.$(selector);
  await input.waitForDisplayed({ timeout: timeoutMs });
  await input.setValue(value);
}


export async function clickKeyManagementFormButton(browser, formLabel, buttonLabel, timeout) {
  const selector = `//form[@aria-label=${xpathLiteral(
    formLabel
  )}]//button[normalize-space()=${xpathLiteral(buttonLabel)}]`;
  const button = await browser.$(selector);
  await button.waitForDisplayed({ timeout });
  await button.click();
}


export async function waitForKeyManagementStatus(browser, testId, expectedTexts, timeout, description) {
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


export async function waitForSecureBackupSetupEvidence(browser, timeout) {
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


export async function waitForFileExists(path, timeout, description) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeout) {
    if (existsSync(path)) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} was not produced`);
}


export async function getRoomEvent(homeserver, accessToken, roomId, eventId) {
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


export async function localDatetimeInputValue(browser, timestampMs) {
  return browser.execute((value) => {
    const date = new Date(value);
    const offset = date.getTimezoneOffset() * 60_000;
    return new Date(date.getTime() - offset).toISOString().slice(0, 16);
  }, timestampMs);
}


export async function setDatetimeLocalValue(browser, value, label = "Jump to date") {
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
      valid: input.validity.valid,
      badInput: input.validity.badInput,
      rangeUnderflow: input.validity.rangeUnderflow,
      rangeOverflow: input.validity.rangeOverflow,
      inputType: input.type
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


export async function timelineDateJumpDiagnostics(browser, expectedValue = null) {
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


export async function waitForCompressedImageMedia(browser, expected, timeout) {
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


export async function clickReadyComposerSendButton(browser, timeout, description) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute(() => {
      const labelFor = (element) =>
        element?.getAttribute("aria-label") ??
        element?.textContent?.replace(/\s+/g, " ").trim() ??
        "";
      const button = document.querySelector("button.send-button");
      if (!(button instanceof HTMLButtonElement)) {
        return { clicked: false, reason: "missing" };
      }
      const style = window.getComputedStyle(button);
      const rect = button.getBoundingClientRect();
      const visible =
        style.display !== "none" &&
        style.visibility !== "hidden" &&
        Number(style.opacity) !== 0 &&
        rect.width > 0 &&
        rect.height > 0;
      const topElement = visible
        ? document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2)
        : null;
      const covered = Boolean(topElement && topElement !== button && !button.contains(topElement));
      const state = {
        clicked: false,
        reason: "not-ready",
        ariaLabel: button.getAttribute("aria-label") ?? "",
        className: button.className,
        disabled: button.disabled,
        visible,
        covered,
        topLabel: labelFor(topElement),
        topTag: topElement?.tagName ?? null,
        stagedItems: document.querySelectorAll(".upload-staging-item").length,
        title: document.title
      };
      if (button.disabled || !visible || covered || !button.classList.contains("ready")) {
        return state;
      }
      button.click();
      return { ...state, clicked: true, reason: "clicked" };
    });
    if (lastState?.clicked) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} send button was not clickable. Last state: ${JSON.stringify(lastState)}`);
}


export async function waitForStagedUploadsCleared(browser, timeout, description) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute(() => ({
      count: document.querySelectorAll(".upload-staging-item").length,
      dialogs: Array.from(document.querySelectorAll('[role="dialog"]')).map((dialog) => ({
        ariaLabel: dialog.getAttribute("aria-label") ?? "",
        text: dialog.textContent?.replace(/\s+/g, " ").trim().slice(0, 180) ?? ""
      })),
      title: document.title
    }));
    if (lastState?.count === 0) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not clear staged uploads. Last state: ${JSON.stringify(lastState)}`);
}


export async function waitForStagedUpload(browser, filename, timeout, description) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute((expectedFilename) => {
      const stagedItems = Array.from(document.querySelectorAll(".upload-staging-item")).map(
        (item) => ({
          name: item.querySelector(".upload-staging-name")?.textContent?.trim() ?? "",
          text: item.textContent?.replace(/\s+/g, " ").trim() ?? "",
          buttons: Array.from(item.querySelectorAll("button"))
            .map((button) => button.textContent?.replace(/\s+/g, " ").trim() || button.getAttribute("aria-label") || "")
            .filter(Boolean)
        })
      );
      const fileInputs = Array.from(document.querySelectorAll('input[type="file"]')).map(
        (input) => ({
          ariaLabel: input.getAttribute("aria-label") ?? "",
          files: input instanceof HTMLInputElement ? Array.from(input.files ?? []).map((file) => file.name) : []
        })
      );
      const dialogs = Array.from(document.querySelectorAll('[role="dialog"]')).map((dialog) => ({
        ariaLabel: dialog.getAttribute("aria-label") ?? "",
        text: dialog.textContent?.replace(/\s+/g, " ").trim().slice(0, 180) ?? ""
      }));
      return {
        found: stagedItems.some((item) => item.name === expectedFilename || item.text.includes(expectedFilename)),
        stagedItems,
        fileInputs,
        dialogs,
        title: document.title
      };
    }, filename);
    if (lastState?.found) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} missing staged upload ${filename}. Last state: ${JSON.stringify(lastState)}`);
}


export async function ensureReadyImageMedia(browser, filename, timeout) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute((expectedFilename) => {
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
      const image = Array.from(document.querySelectorAll("img.message-media-image")).find(
        (candidate) => candidate.getAttribute("alt") === expectedFilename
      );
      const readyRow = image?.closest(".message-media");
      if (readyRow?.classList.contains("message-media-image-ready")) {
        return { ready: true, clickedDownload: false };
      }
      const buttons = Array.from(document.querySelectorAll("button"));
      const downloadButton = buttons.find(
        (button) =>
          button.getAttribute("aria-label") === `Download ${expectedFilename}` &&
          visible(button)
      );
      const diagnostics = Array.from(document.querySelectorAll(".message-media")).map((row) => ({
        kind: row.getAttribute("data-media-kind"),
        downloadState: row.getAttribute("data-download-state"),
        title: row.querySelector(".message-media-title")?.textContent?.trim() ?? "",
        imageAlt: row.querySelector("img.message-media-image")?.getAttribute("alt") ?? "",
        labels: Array.from(row.querySelectorAll("button"))
          .map((button) => button.getAttribute("aria-label") ?? "")
          .filter(Boolean)
      }));
      if (downloadButton instanceof HTMLButtonElement) {
        downloadButton.click();
        return { ready: false, clickedDownload: true, diagnostics };
      }
      return { ready: false, clickedDownload: false, diagnostics };
    }, filename);
    if (lastState?.ready) {
      return;
    }
    if (lastState?.clickedDownload) {
      await waitForReadyImageMedia(browser, filename, Math.max(1000, timeout - (Date.now() - startedAt)));
      return;
    }
    await sleep(250);
  }
  throw new Error(`ready inline image media unavailable ${filename}. Last state: ${JSON.stringify(lastState)}`);
}


async function waitForReadyImageMedia(browser, filename, timeout) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute((expectedFilename) => {
      const rows = Array.from(document.querySelectorAll(".message-media"));
      const diagnostics = rows.map((row) => ({
        kind: row.getAttribute("data-media-kind"),
        downloadState: row.getAttribute("data-download-state"),
        title: row.querySelector(".message-media-title")?.textContent?.trim() ?? "",
        imageAlt: row.querySelector("img.message-media-image")?.getAttribute("alt") ?? "",
        labels: Array.from(row.querySelectorAll("button"))
          .map((button) => button.getAttribute("aria-label") ?? "")
          .filter(Boolean)
      }));
      const image = Array.from(document.querySelectorAll("img.message-media-image")).find(
        (candidate) => candidate.getAttribute("alt") === expectedFilename
      );
      const row = image?.closest(".message-media");
      const labels = row
        ? Array.from(row.querySelectorAll("button"))
            .map((button) => button.getAttribute("aria-label") ?? "")
            .filter(Boolean)
        : [];
      return {
        ready:
          Boolean(image) &&
          row?.classList.contains("message-media-image-ready") === true &&
          labels.includes(`Show media details for ${expectedFilename}`) &&
          labels.includes(`Download ${expectedFilename}`),
        diagnostics
      };
    }, filename);
    if (lastState?.ready) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `ready inline image media row missing ${filename}. Last state: ${JSON.stringify(lastState)}`
  );
}


export async function waitForReadyImageHoverActions(browser, filename, timeout) {
  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeout) {
    lastState = await browser.execute((expectedFilename) => {
      const image = Array.from(document.querySelectorAll("img.message-media-image")).find(
        (candidate) => candidate.getAttribute("alt") === expectedFilename
      );
      const row = image?.closest(".message-media");
      const actions = row?.querySelector(".message-media-hover-actions");
      const labels = row
        ? Array.from(row.querySelectorAll("button"))
            .map((button) => button.getAttribute("aria-label") ?? "")
            .filter(Boolean)
        : [];
      const opacity = actions ? Number(window.getComputedStyle(actions).opacity) : null;
      return {
        visible:
          opacity !== null &&
          opacity > 0.5 &&
          labels.includes(`Show media details for ${expectedFilename}`) &&
          labels.includes(`Download ${expectedFilename}`),
        opacity,
        labels
      };
    }, filename);
    if (lastState?.visible) {
      return;
    }
    await sleep(250);
  }
  throw new Error(
    `ready inline image hover actions missing ${filename}. Last state: ${JSON.stringify(lastState)}`
  );
}


export async function waitForRichFormattedTimeline(browser, expected, expectedWhiteSpace, timeout) {
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


export async function waitForCjkVisualContract(browser, expected, timeout) {
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


export async function elementCount(browser, selector) {
  return browser.execute((cssSelector) => document.querySelectorAll(cssSelector).length, selector);
}


export async function waitForElementCount(browser, selector, expected, timeout, description) {
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


export async function waitForPinnedRegionVisible(browser, timeout, description) {
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


export async function waitForPinnedRegionCleared(browser, timeout, description) {
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


export async function waitForRoomManagementTopic(browser, expectedTopic, timeout, description) {
  const startedAt = Date.now();
  let matched = false;
  while (Date.now() - startedAt < timeout) {
    matched = await browser.execute((topic) => {
      // Issue #1008: the topic card shows the Rust-confirmed value while no
      // editor is open in it.
      const card = document.querySelector('[data-setting-property="topic"]');
      if (!card || card.querySelector("textarea")) return false;
      const value = card.querySelector(".settings-property-value");
      return (value?.textContent ?? "").replace(/\s+/g, " ").trim() === topic;
    }, expectedTopic);
    if (matched) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`${description} did not observe the Rust-owned topic snapshot`);
}


export async function waitForRoomMemberRole(browser, expectedLabel, expectedValue, timeout, description) {
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


export async function waitForRoomMemberAlias(browser, expectedLabel, expectedOriginal, timeout, description) {
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


export async function waitForTimelineSenderLabel(browser, expectedLabel, timeout, description) {
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


export async function clickRoomMemberAliasClear(browser, aliasLabel, timeout) {
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


export async function selectComposerText(browser) {
  await browser.execute((selector) => {
    const editor = document.querySelector(selector);
    if (!(editor instanceof HTMLElement) || !editor.isContentEditable) {
      return;
    }
    editor.focus();
    const range = document.createRange();
    range.selectNodeContents(editor);
    const selection = document.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  }, MESSAGE_COMPOSER_SELECTOR);
}


export async function openRoomContextMenu(browser, sectionId, roomName) {
  const roomButton = await browser.$(roomButtonXpath(sectionId, roomName));
  await roomButton.waitForDisplayed({ timeout: timeoutMs });
  await roomButton.moveTo();
  await roomButton.click({ button: "right" });
}


export async function selectRoomByName(browser, roomName, timeout) {
  const roomButton = await browser.$(
    `//button[@data-testid="room-item"][.//span[normalize-space()=${xpathLiteral(roomName)}]]`
  );
  await roomButton.waitForDisplayed({ timeout });
  await roomButton.click();
}


export async function waitForWorkspaceButton(browser, label, timeout, description) {
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


export async function waitForWorkspaceActive(browser, label, expected, timeout, description) {
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


export async function clickWorkspaceButton(browser, label, timeout, description) {
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


export async function waitForActiveRoomName(browser, roomName, timeout) {
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


export async function waitForTimelineViewMounted(browser, timeout) {
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


export async function scrollTimelineToTop(browser) {
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


export async function waitForTimelineScrolledToBottom(browser, timeout, description) {
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


export async function driveTimelineToBottom(browser, timeout, description) {
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


export async function waitForTimelineScrollable(browser, timeout, description) {
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


export async function waitForTimelineAwayFromBottom(browser, timeout, description) {
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


export async function clickVisibleButtonByTextPrefix(browser, prefix, timeout, description) {
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


export async function clickVisibleButtonByAriaLabel(browser, label, timeout, description, scopeSelector = null) {
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


export async function clickVisibleButtonByAriaLabelInElement(element, label, timeout, description) {
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


export async function clickMenuItemByText(browser, label, timeout) {
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


export async function clickVisibleMenuItemByText(browser, label, timeout) {
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


export async function waitForMessageSourceDialog(browser, timeout) {
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


export async function waitForLatestMessageActionButton(browser, timeout) {
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


export async function clickLatestMessageRedactButtonByText(browser, bodyText, timeout) {
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


export async function waitForLatestEventMessageRow(browser, timeout, description) {
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


export async function waitForRoomInSection(browser, sectionId, roomName, expected, timeout, description) {
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


export async function setTextInputValueByLabel(browser, value, label) {
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


export async function waitForInputValue(browser, label, expectedValue, timeout, description) {
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


export function readyImageMediaXpath(filename) {
  return `//img[contains(concat(" ", normalize-space(@class), " "), " message-media-image ") and @alt=${xpathLiteral(filename)}]`;
}


export function readyImageOpenButtonXpath(filename) {
  return `${readyImageMediaXpath(filename)}/ancestor::button[contains(concat(" ", normalize-space(@class), " "), " message-media-open ")][1]`;
}


export function xpathLiteral(value) {
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


export function writePngFixture(path, width, height) {
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


export async function setSyntheticFileInput(browser, selector, fixturePath, filename, mimeType, contents) {
  await makeFileInputInteractable(browser, selector);
  const input = await browser.$(selector);
  try {
    await input.waitForDisplayed({ timeout: timeoutMs });
    await input.setValue(fixturePath);
    const nativeFileNames = await fileInputFileNames(browser, selector);
    if (!nativeFileNames.includes(filename)) {
      await setSyntheticFileList(browser, selector, filename, mimeType, contents);
    }
    await dispatchFileInputChange(browser, selector);
  } finally {
    await restoreFileInputPresentation(browser, selector);
  }
}


async function fileInputFileNames(browser, selector) {
  return browser.execute((cssSelector) => {
    const input = document.querySelector(cssSelector);
    return input instanceof HTMLInputElement
      ? Array.from(input.files ?? []).map((file) => file.name)
      : [];
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


export async function waitForElementCountGreaterThan(browser, selector, baseline, timeout, description) {
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


export async function waitForReplyLanded(browser, baselineMessages, timeout) {
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


export async function importDesktopWebdriverio() {
  const webdriverioEntry = desktopPackageRequire.resolve("webdriverio");
  return await import(pathToFileURL(webdriverioEntry).href);
}



export function webdriverCapabilities(appBinary) {
  return {
    browserName: "wry",
    "wdio:enforceWebDriverClassic": true,
    "tauri:options": {
      application: appBinary
    }
  };
}



export async function safeDeleteSession(browser) {
  try {
    await browser.deleteSession();
  } catch {
    // ignore cleanup failures
  }
}
