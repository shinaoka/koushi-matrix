import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { gotoReadyShell } from "./support/basicOperations";

/*
 * #2045 (the acceptance #2038 asked for): confirming a copy must not change the
 * Diagnostics dialog's geometry. The report came from a Japanese macOS profile
 * in a window short enough that the dialog only just fits, so this spec opens the
 * real dialog on the real stylesheet, seeds a long synthetic report through the
 * app's own diagnostics snapshot, switches to that locale and the macOS platform
 * (44px of native title bar is reserved above the WebView, which tightens the
 * dialog's height cap), constrains the viewport, and compares the outer scroll
 * range, the modal bounds, the scrollable regions and the footer before Copy and
 * after each confirmation.
 *
 * jsdom cannot see any of this (it has no layout), and the CSS-string contract
 * test next to it only reads declarations, so this is the guard that fails when
 * the dialog grid, the status slot or the preview sizing regress. The save path's
 * confirmation is the longest status string, so it is measured too.
 */

const JAPANESE_MACOS = {
  lang: "ja",
  catalog_locale: "ja",
  pseudo_locale: "none",
  platform: "macos"
} as const;
const LABELS = {
  open: t("diagnostics.open", {}, "ja"),
  title: t("diagnostics.title", {}, "ja"),
  copy: t("diagnostics.copy", {}, "ja"),
  save: t("diagnostics.save", {}, "ja"),
  copied: t("diagnostics.copied", {}, "ja"),
  saved: t("diagnostics.saved", {}, "ja")
};

/* A Japanese macOS profile produces long CJK log lines, and the report is long
   enough to fill the preview at every height below: 320 lines is far past the
   preview's 520px cap, so the preview itself always scrolls. */
const REPORT_ENTRIES = 320;
const REPORT_LAST_LINE = `診断ログ ${REPORT_ENTRIES}:`;

/*
 * Heights where the bug is reachable: the dialog overflows once its natural
 * height reaches the cap, which happens when the preview's 56vh cap plus the
 * fixed chrome comes to just under `100dvh - 32px - 44px` on macOS. Three heights
 * bracket that crossing, so the check does not rest on one magic number.
 */
const CONSTRAINED_HEIGHTS = [520, 560, 640] as const;

/** Geometry and bounds of one state of the dialog; compared across states. */
type DiagnosticsGeometry = {
  /** Outer scroll range of the modal box: > 1 means a second scrollbar. */
  dialogScrollRange: number;
  /** The box's own cap, from the computed style, so macOS's safe area counts. */
  dialogMaxBlockSize: number;
  dialog: { top: number; left: number; width: number; height: number };
  /** Visible height of the preview, which the status row must not take from. */
  previewClientHeight: number;
  previewScrollRange: number;
  previewScrollTop: number;
  /**
   * Regions inside the dialog that scroll vertically, by class. The dialog box
   * and the footer cannot scroll, so a second entry means a new scrollbar
   * appeared somewhere between the title and the actions.
   */
  scrollableRegions: string[];
  /** The live region that announces the action, and what it says. */
  statusSlot: { mounted: boolean; text: string };
  footer: { saveTop: number; copyTop: number; visible: boolean };
  /** What the last Copy put on the clipboard ("" before the first click). */
  clipboardTail: string;
};

/**
 * Stubs clipboard completion so the confirmation state is reachable without a
 * permission prompt, and keeps what was written to prove the copy happened.
 */
async function stubClipboard(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const writes: string[] = [];
    Object.defineProperty(window, "__clipboardWrites", { value: writes, configurable: true });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          writes.push(text);
        }
      }
    });
  });
}

/** Seeds the long synthetic Japanese report the app renders in its own dialog. */
async function seedLongReport(page: Page): Promise<void> {
  await page.evaluate(
    ({ entries, profile }) => {
      const snapshot = window.__harness.currentSnapshot();
      window.__harness.setSnapshot({
        ...snapshot,
        state: {
          ...snapshot.state,
          domain: {
            ...snapshot.state.domain,
            locale_profile: { ...snapshot.state.domain.locale_profile, ...profile }
          }
        }
      });
      // The store drops a snapshot that does not advance its generation (#759).
      window.__harness.pushStateUpdate();
      window.__harness.setCommandResponse("get_diagnostic_snapshot", () => ({
        entries: Array.from({ length: entries }, (_unused, index) => ({
          timestampMs: 1_700_000_000_000 + index * 997,
          source: index % 3 === 0 ? "renderer" : index % 3 === 1 ? "snapshot" : "timeline",
          message:
            `診断ログ ${index + 1}: 表示プロファイル=日本語 (ja_JP) ピクセル比=2.0 ` +
            `ウィンドウ=1440x812 スクロール位置=${index * 7} ` +
            `タイムライン更新=受信済み 描画完了=真 キュー滞留=${index % 11}`
        })),
        droppedEntries: 0
      }));
    },
    { entries: REPORT_ENTRIES, profile: JAPANESE_MACOS }
  );
}

async function readGeometry(
  page: Page,
  labels: { save: string; copy: string }
): Promise<DiagnosticsGeometry> {
  return page.evaluate((expected) => {
    const dialog = document.querySelector<HTMLElement>(".diagnostics-dialog");
    if (!dialog) {
      throw new Error("the Diagnostics dialog is not open");
    }
    const round = (value: number) => Math.round(value * 10) / 10;
    const box = dialog.getBoundingClientRect();
    const preview = dialog.querySelector<HTMLElement>(".diagnostics-output");
    if (!preview) {
      throw new Error("the Diagnostics preview is missing");
    }
    const buttons = [...dialog.querySelectorAll<HTMLElement>(".dialog-actions button")];
    const footerButton = (label: string) => {
      const button = buttons.find((candidate) => candidate.getAttribute("aria-label") === label);
      if (!button) {
        // Missing is not the same failure as clipped: say which one happened.
        throw new Error(`the Diagnostics ${label} button is missing`);
      }
      return button;
    };
    const save = footerButton(expected.save);
    const copy = footerButton(expected.copy);
    // Reachable means: inside the box, on screen, and the topmost element at its
    // own centre (the footer cannot end up under the dialog's clipped overflow).
    const visible = buttons.every((button) => {
      const rect = button.getBoundingClientRect();
      const style = getComputedStyle(button);
      const hit = document.elementFromPoint(
        rect.left + rect.width / 2,
        rect.top + rect.height / 2
      );
      return (
        style.visibility !== "hidden" &&
        style.display !== "none" &&
        rect.width > 0 &&
        rect.height > 0 &&
        rect.top >= box.top - 0.5 &&
        rect.bottom <= box.top + box.height + 0.5 &&
        rect.top >= 0 &&
        rect.bottom <= window.innerHeight + 0.5 &&
        hit !== null &&
        (hit === button || button.contains(hit))
      );
    });
    const scrolls = (element: Element): boolean => {
      const style = getComputedStyle(element);
      return (
        (style.overflowY === "auto" || style.overflowY === "scroll") &&
        element.scrollHeight - element.clientHeight > 1
      );
    };
    const writes = (window as unknown as { __clipboardWrites?: string[] }).__clipboardWrites ?? [];
    const status = dialog.querySelector('[role="status"]');
    return {
      dialogScrollRange: dialog.scrollHeight - dialog.clientHeight,
      dialogMaxBlockSize: Number.parseFloat(getComputedStyle(dialog).maxBlockSize),
      dialog: {
        top: round(box.top),
        left: round(box.left),
        width: round(box.width),
        height: round(box.height)
      },
      previewClientHeight: preview.clientHeight,
      previewScrollRange: preview.scrollHeight - preview.clientHeight,
      previewScrollTop: preview.scrollTop,
      scrollableRegions: [...dialog.querySelectorAll("*")]
        .filter(scrolls)
        .map((element) => element.className || element.tagName.toLowerCase()),
      statusSlot: { mounted: status !== null, text: status?.textContent ?? "" },
      footer: {
        saveTop: round(save.getBoundingClientRect().top),
        copyTop: round(copy.getBoundingClientRect().top),
        visible
      },
      clipboardTail: writes.at(-1) ?? ""
    };
  }, labels);
}

/** The state-independent part of the geometry: one shape for all comparisons. */
function comparable(geometry: DiagnosticsGeometry) {
  return {
    dialog: geometry.dialog,
    previewClientHeight: geometry.previewClientHeight,
    scrollableRegions: geometry.scrollableRegions,
    saveTop: geometry.footer.saveTop,
    copyTop: geometry.footer.copyTop
  };
}

async function openDiagnostics(page: Page, height: number): Promise<void> {
  await page.setViewportSize({ width: 900, height });
  await gotoReadyShell(page);
  await seedLongReport(page);
  // The UI is in Japanese before the dialog is opened, so the labels and the
  // confirmations are the ones a Japanese macOS profile shows.
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.catalogLocale))
    .toBe("ja");
  await expect
    .poll(() => page.evaluate(() => document.documentElement.dataset.platform))
    .toBe("macos");
  // The web fonts decide the privacy note's wrap width and therefore the dialog's
  // height, so the baseline is only taken once they are loaded.
  await page.evaluate(() => document.fonts.ready);
  await page.getByRole("button", { name: LABELS.open }).first().click();
  await expect(page.getByRole("dialog", { name: LABELS.title })).toBeVisible();
  await expect(page.locator(".diagnostics-output")).toContainText("診断ログ 1:");
}

for (const height of CONSTRAINED_HEIGHTS) {
  test(`copying a long report leaves the dialog geometry unchanged at ${height}px`, async ({
    page
  }) => {
    await stubClipboard(page);
    await openDiagnostics(page, height);
    const dialog = page.getByRole("dialog", { name: LABELS.title });

    const before = await readGeometry(page, LABELS);

    // Scroll the preview away from its top, so a re-layout that resets it shows up.
    const scrolledTop = await page.evaluate(() => {
      const element = document.querySelector<HTMLElement>(".diagnostics-output")!;
      element.scrollTop = Math.floor((element.scrollHeight - element.clientHeight) / 2);
      return element.scrollTop;
    });
    expect(scrolledTop, "the preview does not scroll on its own").toBeGreaterThan(0);

    await dialog.getByRole("button", { name: LABELS.copy }).click();
    await expect(dialog.getByRole("status")).toHaveText(LABELS.copied);
    const afterCopy = await readGeometry(page, LABELS);

    // The save confirmation is the longest status string, so it is measured too.
    await dialog.getByRole("button", { name: LABELS.save }).click();
    await expect(dialog.getByRole("status")).toHaveText(LABELS.saved);
    const afterSave = await readGeometry(page, LABELS);

    const describe = (geometry: DiagnosticsGeometry) => JSON.stringify(comparable(geometry));

    // The copy reached the clipboard with this report, so the confirmations below
    // are the real ones and not a no-op.
    expect(afterCopy.clipboardTail, "the copy never reached the clipboard").toContain(
      REPORT_LAST_LINE
    );
    expect(afterCopy.clipboardTail.length).toBeGreaterThan(REPORT_ENTRIES * 20);

    // 1. No outer scrollbar in any state: the dialog box itself never scrolls, so
    //    the footer cannot be pushed into its hidden overflow. One pixel covers
    //    a fractional row sum; a second scrollbar is tens of pixels.
    const outerScroll = {
      idle: before.dialogScrollRange,
      copy: afterCopy.dialogScrollRange,
      save: afterSave.dialogScrollRange
    };
    const scrollMessage = `outer scroll at ${height}px: ${JSON.stringify(outerScroll)}; idle ${describe(before)}`;
    expect(outerScroll.idle, scrollMessage).toBeLessThanOrEqual(1);
    expect(outerScroll.copy, scrollMessage).toBeLessThanOrEqual(1);
    expect(outerScroll.save, scrollMessage).toBeLessThanOrEqual(1);

    // 2. The modal stays inside its own cap: the 100dvh cap minus the 32px
    //    overlay inset and macOS's 44px title bar, read from the computed style.
    expect(afterSave.dialogMaxBlockSize).toBeGreaterThan(0);
    for (const state of [before, afterCopy, afterSave]) {
      expect(state.dialog.height).toBeLessThanOrEqual(state.dialogMaxBlockSize + 0.5);
    }
    expect(afterSave.dialog.top).toBeGreaterThanOrEqual(0);
    expect(afterSave.dialog.top + afterSave.dialog.height).toBeLessThanOrEqual(height + 0.5);

    // 3. One scrollable region, and it is the preview, in every state.
    expect(before.previewScrollRange, "the preview does not overflow").toBeGreaterThan(0);
    for (const state of [before, afterCopy, afterSave]) {
      expect(state.scrollableRegions).toEqual(["diagnostics-output"]);
    }

    // 4. Save and Copy stay fully visible and hit-testable inside the dialog.
    expect(
      { idle: before.footer.visible, copy: afterCopy.footer.visible, save: afterSave.footer.visible },
      `footer clipped at ${height}px: ${describe(before)}`
    ).toEqual({ idle: true, copy: true, save: true });

    // 5. The confirmation takes nothing from the preview: it keeps its visible
    //    height and its own scroll position.
    expect(afterCopy.previewScrollTop, `the preview lost its scroll at ${height}px`).toBe(scrolledTop);
    expect(afterSave.previewScrollTop).toBe(scrolledTop);

    // 6. The whole geometry is unchanged by either confirmation.
    expect(comparable(afterCopy), `geometry moved after copy at ${height}px`).toEqual(
      comparable(before)
    );
    expect(comparable(afterSave), `geometry moved after save at ${height}px`).toEqual(
      comparable(before)
    );

    // 7. The confirmation is announced through a live region that is mounted in
    //    every state: a region added only when it has something to say is exactly
    //    what used to move the dialog.
    expect(before.statusSlot).toEqual({ mounted: true, text: "" });
    expect(afterCopy.statusSlot).toEqual({ mounted: true, text: LABELS.copied });
    expect(afterSave.statusSlot).toEqual({ mounted: true, text: LABELS.saved });
  });
}
