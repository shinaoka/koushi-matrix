import { expect, test, type Page } from "@playwright/test";

/**
 * #1748: the desktop shell renders the Rust-projected read/send admission.
 *
 * These specs exercise the state matrix the issue asks for — readable
 * provisional, readable authoritative, send-blocked, restore failure, and trust
 * invalidation — plus locale switching while the status is visible, accessible
 * announcements, and read-only composer / main / thread / upload controls. They
 * assert measured geometry and DOM role state rather than only `toBeVisible()`,
 * and they never inject an admission the harness could not have received from
 * Rust: `setCurrentSnapshot` restamps `conversation_access` from the session,
 * backup gate, and room-list readiness a spec pushes.
 */

const PROVISIONAL_COPY =
  "Showing saved conversations. Rooms and messages may be missing or out of date, and unread counts are the last values saved on this device.";
const SEND_BLOCKED_COPY = "Sending is unavailable until Secure Backup is ready.";

const PROVISIONAL_COPY_BY_LOCALE = {
  ja: "保存済みの会話を表示しています。同期が完了していないため、ルームやメッセージが欠けていたり古い可能性があります。未読数は端末に保存されている最後の値です。",
  "zh-Hans":
    "正在显示已保存的会话。同步尚未完成，房间或消息可能缺失或已过期，未读数为本机保存的最后数值。",
  "zh-Hant":
    "正在顯示已儲存的會話。同步尚未完成，聊天室或訊息可能缺少或已過期，未讀數為本機儲存的最後數值。",
  ko: "저장된 대화를 표시하고 있습니다. 동기화가 끝나지 않아 방이나 메시지가 빠져 있거나 오래되었을 수 있고, 읽지 않은 수는 이 기기에 저장된 마지막 값입니다."
} as const;

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
}

/** Push one synthetic Rust snapshot and let the renderer apply it. */
async function pushAdmission(
  page: Page,
  options: {
    gate?: Record<string, unknown>;
    readiness?: Record<string, unknown>;
    session?: Record<string, unknown>;
  }
): Promise<void> {
  await page.evaluate((next) => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          ...(next.session ? { session: { ...snapshot.state.domain.session, ...next.session } as never } : {}),
          ...(next.gate ? { secure_backup_gate: next.gate as never } : {})
        },
        ui: {
          ...snapshot.state.ui,
          ...(next.readiness
            ? {
                room_list: {
                  ...snapshot.state.ui.room_list,
                  readiness: next.readiness as never
                }
              }
            : {})
        }
      }
    });
    window.__harness.pushStateUpdate();
  }, options);
}

// The sidebar's accessible name is translated with the catalog, so locate the
// notice by its own class instead of a locale-dependent landmark name.
function sidebarStatus(page: Page) {
  return page.locator(".room-list-status");
}

function composerNotice(page: Page) {
  return page.locator("main .composer-read-only-notice");
}

function messageComposer(page: Page) {
  return page.getByRole("textbox", { name: "Message composer" });
}

test("a provisional cache opens the normal shell with two separate, honest notices", async ({
  page
}) => {
  await gotoReadyShell(page);
  // A trusted restored session that may read the cached rooms while the first
  // sync response and the Secure Backup inspection are both still pending.
  await pushAdmission(page, {
    gate: { kind: "checking" },
    readiness: { kind: "loading", source: "cache", generation: 4 }
  });

  // The blocking screen is gone; the cache is readable.
  await expect(page.locator("main.session-verification-gate")).toHaveCount(0);
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();

  // The provisional-cache warning is a live status region in the sidebar.
  const status = sidebarStatus(page);
  await expect(status).toHaveAttribute("role", "status");
  await expect(status).toHaveText(PROVISIONAL_COPY);

  // The send-blocked reason is separate copy, beside the composer it explains.
  const notice = composerNotice(page);
  await expect(notice).toHaveCount(1);
  await expect(notice).toHaveAttribute("role", "status");
  await expect(notice).toHaveText(SEND_BLOCKED_COPY);
  expect(await status.textContent()).not.toBe(await notice.textContent());

  // A delayed first response is never described as offline.
  await expect(page.locator("body")).not.toContainText(/offline/i);

  // Measured geometry: both notices occupy real space above the content they
  // qualify, and the sidebar notice does not overlap the room rows.
  const statusBox = await status.boundingBox();
  const firstRoomBox = await page.locator(".room-item").first().boundingBox();
  expect(statusBox?.width ?? 0).toBeGreaterThan(0);
  expect(statusBox?.height ?? 0).toBeGreaterThan(0);
  expect(firstRoomBox).not.toBeNull();
  expect(statusBox!.y + statusBox!.height).toBeLessThanOrEqual(firstRoomBox!.y + 1);
  const noticeBox = await notice.boundingBox();
  const timelineBox = await page.getByRole("main", { name: "Conversation timeline" }).boundingBox();
  expect(noticeBox?.width ?? 0).toBeGreaterThan(0);
  expect(noticeBox!.y + noticeBox!.height).toBeLessThanOrEqual(
    timelineBox!.y + timelineBox!.height + 1
  );
});

test("the provisional cache keeps the composer, thread and upload surfaces read-only", async ({
  page
}) => {
  await gotoReadyShell(page);
  await pushAdmission(page, {
    gate: { kind: "checking" },
    readiness: { kind: "loading", source: "cache", generation: 4 }
  });
  await page.evaluate(() => window.__harness.clearInvocations());

  // Main composer: DOM-owned, non-editable, send disabled.
  const composer = messageComposer(page);
  await expect(composer).toHaveAttribute("contenteditable", "false");
  await expect(page.getByRole("button", { name: "Attach file", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Send" }).first()).toBeDisabled();

  // Typing into it is rejected by the DOM and sends nothing. `force` bypasses
  // Playwright's own enabled check so the keypresses really reach the DOM.
  await composer.click({ force: true });
  await page.keyboard.type("cached read only draft");
  await expect(composer).toHaveText("");
  await expect(page.getByRole("button", { name: "Send" }).first()).toBeDisabled();
  expect(await page.evaluate(() => window.__harness.invocationsOf("send_text").length)).toBe(0);

  // Upload surface: the file picker trigger is disabled and a forced file
  // selection stages nothing, so nothing can be uploaded from a read-only shell.
  await expect(page.getByRole("button", { name: "Attach file", exact: true })).toBeDisabled();
  await page.locator("input.composer-file-input").setInputFiles({
    name: "fixture.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("synthetic")
  });
  await page.waitForTimeout(50);
  expect(
    await page.evaluate(() => window.__harness.invocationsOf("stage_upload_bytes").length)
  ).toBe(0);

  // Thread composer follows the same Rust admission. Open the thread through
  // its ordinary pill so the real right-panel path renders it.
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setCommandResponse("open_thread", {
      ...snapshot,
      state: {
        ...snapshot.state,
        ui: {
          ...snapshot.state.ui,
          thread: {
            kind: "open",
            room_id: "!harness-room:example.invalid",
            root_event_id: "$seed-event:example.invalid",
            intent: "existingThread",
            is_subscribed: true,
            staged_uploads: [],
            composer: { ...snapshot.state.ui.timeline.composer }
          }
        }
      }
    });
  });
  await page.getByRole("button", { name: /2 replies/ }).click();
  const panel = page.getByRole("complementary", { name: "Context panel" });
  await expect(
    panel.getByRole("textbox", { name: "Thread composer" })
  ).toHaveAttribute("contenteditable", "false");
  await expect(panel.locator(".composer-read-only-notice")).toHaveText(SEND_BLOCKED_COPY);
  await expect(
    panel.getByRole("button", { name: "Attach file", exact: true })
  ).toBeDisabled();

  // Opening cached content never writes a receipt or a fully-read marker.
  expect(await page.evaluate(() => window.__harness.invocationsOf("send_read_receipt").length)).toBe(0);
  expect(await page.evaluate(() => window.__harness.invocationsOf("set_fully_read").length)).toBe(0);
});

test("an authoritative list clears the provisional notice and admits sending", async ({ page }) => {
  await gotoReadyShell(page);
  await pushAdmission(page, {
    gate: { kind: "ready" },
    readiness: { kind: "ready", source: "live", generation: 5 }
  });

  await expect(sidebarStatus(page)).toHaveCount(0);
  await expect(composerNotice(page)).toHaveCount(0);
  await expect(messageComposer(page)).toHaveAttribute("contenteditable", "true");
  await expect(page.getByRole("button", { name: "Attach file", exact: true })).toBeEnabled();

  const composer = messageComposer(page);
  await composer.click();
  await page.keyboard.type("authoritative draft");
  await expect(page.getByRole("button", { name: "Send" }).first()).toBeEnabled();
});

test("an authoritative list with Secure Backup still pending keeps only the send-blocked reason", async ({
  page
}) => {
  await gotoReadyShell(page);
  await pushAdmission(page, {
    gate: { kind: "checking" },
    readiness: { kind: "ready", source: "live", generation: 5 }
  });

  // Counts may claim authority now, so the provisional warning is gone...
  await expect(sidebarStatus(page)).toHaveCount(0);
  // ...while sending stays closed for its own, separate reason.
  await expect(composerNotice(page)).toHaveText(SEND_BLOCKED_COPY);
  await expect(messageComposer(page)).toHaveAttribute("contenteditable", "false");
});

test("a retryable restore failure opens the shell and keeps the retry reachable", async ({
  page
}) => {
  await gotoReadyShell(page);
  await pushAdmission(page, {
    gate: { kind: "blockedFailed", failure: "network" },
    readiness: { kind: "loading", source: "cache", generation: 4 }
  });
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.locator("main.session-verification-gate")).toHaveCount(0);

  await page.evaluate(() => window.__harness.clearInvocations());
  await page.getByRole("button", { name: "Account Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Account Settings", exact: true });
  await settings.getByRole("tab", { name: "Sessions", exact: true }).click();
  const warnings = settings.getByRole("region", { name: "Runtime warnings" });
  await expect(warnings).toContainText("Secure Backup unavailable");
  await warnings.getByRole("button", { name: "Retry secure backup" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.invocationsOf("retry_secure_backup_inspection").length)
    )
    .toBe(1);
});

test("states Rust keeps closed keep the blocking screen and its recovery actions", async ({
  page
}) => {
  await gotoReadyShell(page);

  // Trust invalidation: the session is locked, so no cached content is exposed.
  await pushAdmission(page, {
    session: { kind: "locked" },
    gate: { kind: "checking" }
  });
  await page.evaluate(() => {
    const snapshot = window.__harness.currentSnapshot();
    window.__harness.setSnapshot({
      ...snapshot,
      state: {
        ...snapshot.state,
        domain: {
          ...snapshot.state.domain,
          session_lock_reason: { kind: "unknownToken", soft_logout: false }
        }
      }
    });
    window.__harness.pushStateUpdate();
  });
  await expect(page.locator("main.session-verification-gate")).toBeVisible();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toHaveCount(0);
  await expect(messageComposer(page)).toHaveCount(0);

  // Affirmative recovery requirement: still blocking, with the recovery form
  // and the diagnostics action intact.
  await pushAdmission(page, {
    session: { kind: "ready" },
    gate: { kind: "existingBackupNeedsRecovery", failure: "network" }
  });
  await expect(page.locator("main.session-verification-gate")).toBeVisible();
  await expect(page.getByLabel("Secure backup recovery key")).toBeVisible();
  await expect(page.getByRole("button", { name: "Recover secure backup" })).toBeVisible();

  // A non-retryable inspection failure opens no read exception either, and the
  // gate keeps both its retry and its diagnostics action.
  await pushAdmission(page, {
    gate: { kind: "blockedFailed", failure: "unauthorized" }
  });
  await expect(page.locator("main.session-verification-gate")).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry secure backup" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open secure backup diagnostics" })).toBeVisible();
});

test("Secure Backup setup required still blocks the shell", async ({ page }) => {
  await gotoReadyShell(page);
  await pushAdmission(page, { gate: { kind: "setupRequired" } });
  await expect(page.locator("main.session-verification-gate")).toBeVisible();
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toHaveCount(0);
});

test("the status copy follows the active catalog while it stays visible", async ({ page }) => {
  await gotoReadyShell(page);
  await pushAdmission(page, {
    gate: { kind: "checking" },
    readiness: { kind: "loading", source: "cache", generation: 4 }
  });
  await expect(sidebarStatus(page)).toHaveText(PROVISIONAL_COPY);

  for (const [catalogLocale, expected] of Object.entries(PROVISIONAL_COPY_BY_LOCALE)) {
    await page.evaluate((locale) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            settings: {
              ...snapshot.state.domain.settings,
              values: {
                ...snapshot.state.domain.settings.values,
                locale: { language_tag: locale, text_direction: "auto" }
              }
            },
            locale_profile: {
              ...snapshot.state.domain.locale_profile,
              lang: locale,
              dir: "ltr",
              catalog_locale: locale
            }
          }
        }
      });
      window.__harness.pushStateUpdate();
    }, catalogLocale);

    // The notice is not stale copy captured before the switch.
    await expect(sidebarStatus(page)).toHaveText(expected);
    await expect(page.locator("html")).toHaveAttribute("lang", catalogLocale);
    // The separate send-blocked reason is translated too, not left in English.
    await expect(composerNotice(page)).not.toHaveText(SEND_BLOCKED_COPY);
    await expect(composerNotice(page)).toHaveText(
      catalogLocale === "ja"
        ? "安全なバックアップの準備ができるまで送信できません。"
        : catalogLocale === "ko"
          ? "보안 백업이 준비될 때까지 보낼 수 없습니다."
          : catalogLocale === "zh-Hans"
            ? "安全备份就绪前无法发送。"
            : "安全備份就緒前無法傳送。"
    );
  }
});
