import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

import {
  EMOJI_PICKER_BLOCK_SIZE_PX,
  EMOJI_PICKER_GRID_COLUMNS,
  EMOJI_PICKER_INLINE_SIZE_PX
} from "./components/EmojiPicker";

const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
const appSource = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const shellSource = readFileSync(new URL("./components/Shell.tsx", import.meta.url), "utf8");
const uiSharedSource = readFileSync(new URL("./app/uiShared.ts", import.meta.url), "utf8");

// Sources that render Lucide icons: App.tsx plus the per-component modules the #87
// Phase 2d split moved icon usages into. Fixed icon sizes must stay centralized in
// ICON_SIZE, so the contract scan must cover all of them, not just App.tsx.
const iconRenderingSources: ReadonlyArray<readonly [string, string]> = [
  ["App.tsx", appSource],
  ["app/uiShared.ts", uiSharedSource],
  ...["dialogs", "Shell", "panes", "auth", "mediaLists", "composer", "rightPanel"].map(
    (name) =>
      [
        `components/${name}.tsx`,
        readFileSync(new URL(`./components/${name}.tsx`, import.meta.url), "utf8")
      ] as const
  )
];

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function blockFor(pattern: RegExp, label: string): string {
  const match = css.match(pattern);
  expect(match, `Expected CSS block for ${label}`).not.toBeNull();
  return match?.groups?.body ?? match?.[1] ?? "";
}

function selectorBlock(selector: string): string {
  return blockFor(new RegExp(`${escapeRegExp(selector)}\\s*\\{(?<body>[^}]*)\\}`), selector);
}

function lastSelectorBlock(selector: string): string {
  const pattern = new RegExp(`${escapeRegExp(selector)}\\s*\\{(?<body>[^}]*)\\}`, "g");
  const matches = [...css.matchAll(pattern)];
  expect(matches.length, `Expected CSS block for ${selector}`).toBeGreaterThan(0);
  return matches.at(-1)?.groups?.body ?? "";
}

function groupedSelectorBlock(selectorPattern: RegExp, label: string): string {
  return blockFor(new RegExp(`${selectorPattern.source}\\s*\\{(?<body>[^}]*)\\}`), label);
}

function expectTokens(tokens: string[]) {
  for (const token of tokens) {
    expect(css).toMatch(new RegExp(`(?:^|\\n)\\s*${escapeRegExp(token)}\\s*:`));
  }
}

function expectBlockUses(block: string, tokens: string[]) {
  for (const token of tokens) {
    expect(block).toContain(`var(${token}`);
  }
}

describe("styles.css token system", () => {
  test("defines the blue brand token in light and dark", () => {
    expect(css).toContain("--brand: #2d6fef;");
    expect(css).toMatch(/:root\[data-theme="dark"\]/);
    expect(css).toContain("--brand: #5c8df6;");
  });

  test("supports OS dark mode and a color-scheme", () => {
    expect(css).toContain("@media (prefers-color-scheme: dark)");
    expect(css).toContain("color-scheme: light dark;");
  });

  test("ships the full semantic token set in :root", () => {
    for (const token of [
      "--brand-hover",
      "--brand-weak",
      "--brand-contrast",
      "--rail",
      "--rail-item",
      "--sidebar",
      "--surface",
      "--surface-hover",
      "--selected",
      "--text",
      "--text-muted",
      "--text-faint",
      "--line",
      "--unread",
      "--mention",
      "--danger",
      "--success",
      "--warning"
    ]) {
      expect(css).toContain(`${token}:`);
    }
  });

  test("uses a neutral light shell with blue reserved for brand accents", () => {
    expect(css).toContain("--rail: #f7f8fa;");
    expect(css).toContain("--rail-item: #e9ecf1;");
    expect(css).toContain("--sidebar: #fafbfc;");
    expect(css).toContain("--surface-hover: #f2f4f6;");
    expect(css).toContain("--selected: #e9ecf0;");
    expect(css).toContain("--text: #171b21;");
    expect(css).toContain("--line: #e5e8ec;");
  });

  test("defines the eight-color avatar palette and fallback classes", () => {
    for (const token of [
      "--avatar-1",
      "--avatar-2",
      "--avatar-3",
      "--avatar-4",
      "--avatar-5",
      "--avatar-6",
      "--avatar-7",
      "--avatar-8"
    ]) {
      expect(css).toContain(`${token}:`);
    }
    for (let index = 1; index <= 8; index += 1) {
      expect(selectorBlock(`.avatar-c${index}`)).toContain(`background: var(--avatar-${index});`);
    }
  });

  test("contains no legacy purple or green literals", () => {
    for (const legacy of [
      "#34123d",
      "#4a1858",
      "#5b236a",
      "#1f6fb2",
      "#2f9d68",
      "--accent",
      "--brand-2",
      "--muted:",
      "--faint:"
    ]) {
      if (legacy.startsWith("--") && !legacy.endsWith(":")) {
        const escapedLegacy = legacy.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        expect(css).not.toMatch(new RegExp(`${escapedLegacy}(?=\\s*[:),])`));
      } else {
        expect(css).not.toContain(legacy);
      }
    }
  });

  test("upload staging panel bounds to available height with a dedicated scroll owner (#515)", () => {
    const timeline = selectorBlock(".timeline-scroll");
    // A long virtual timeline must consume the remaining height instead of
    // shrinking composer siblings from its intrinsic scroll extent.
    expect(timeline).toMatch(/flex:\s*1\s+1\s+0/);

    const dialog = selectorBlock(".upload-staging-dialog");
    // Three-row layout: header / minmax(0, 1fr) scroll body / footer.
    expect(dialog).toMatch(/grid-template-rows:\s*auto\s+minmax\(0,\s*1fr\)\s+auto/);
    // Bounded to the available vertical space and never overflow-clipped
    // without a scroll owner (the panel itself must not scroll the page).
    expect(dialog).toMatch(/max-height:\s*min\(80vh,\s*640px\)/);
    expect(dialog).toMatch(/overflow:\s*hidden/);

    const list = selectorBlock(".upload-staging-list");
    // The list is the vertical scroll owner: it can collapse below content
    // size and scrolls, containing overscroll so the timeline never moves.
    expect(list).toMatch(/min-height:\s*0/);
    expect(list).toMatch(/overflow-y:\s*auto/);
    expect(list).toMatch(/overscroll-behavior:\s*contain/);

    const singleList = selectorBlock(".upload-staging-list.is-single");
    // A single attachment keeps its editor fixed; any vertical pressure is
    // absorbed by the bounded image preview instead of scrolling the caption.
    expect(singleList).toMatch(/overflow-y:\s*hidden/);

    const singlePreviewItem = selectorBlock(
      ".upload-staging-list.is-single .upload-staging-item.has-preview"
    );
    expect(singlePreviewItem).toMatch(
      /grid-template-rows:\s*auto\s+minmax\(0,\s*1fr\)\s+auto/
    );
    expect(singlePreviewItem).toMatch(/min-block-size:\s*0/);

    const singlePreview = selectorBlock(
      ".upload-staging-list.is-single .upload-staging-item.has-preview .upload-preview-viewport"
    );
    expect(singlePreview).toMatch(/block-size:\s*100%/);
    expect(singlePreview).toMatch(
      /min-block-size:\s*var\(--upload-preview-viewport-min-size\)/
    );

    const preview = selectorBlock(".upload-preview-viewport");
    // A prepared image pans inside its own bounded surface; controls outside
    // this viewport do not move when the user inspects a large image.
    expect(preview).toMatch(/overflow:\s*auto/);
    expect(preview).toMatch(/overscroll-behavior:\s*contain/);
    expect(preview).toMatch(/inline-size:\s*100%/);
    expect(preview).toMatch(/min-inline-size:\s*0/);

    const fittedPreview = selectorBlock(
      '.upload-preview-viewport[data-preview-mode="fit"] .upload-staging-preview'
    );
    expect(fittedPreview).toMatch(/max-inline-size:\s*100%/);
    expect(fittedPreview).toMatch(/max-block-size:\s*100%/);

    const actualPreview = selectorBlock(
      '.upload-preview-viewport[data-preview-mode="actual"] .upload-staging-preview'
    );
    expect(actualPreview).toMatch(/max-inline-size:\s*none/);
    expect(actualPreview).toMatch(/max-block-size:\s*none/);

    const compactCaptionEditor = selectorBlock(
      ".upload-staging-caption .composer-inline-editor"
    );
    expect(compactCaptionEditor).toMatch(/min-block-size:\s*48px/);
    expect(compactCaptionEditor).toMatch(/max-block-size:\s*112px/);

    const compactCaptionToolbar = selectorBlock(
      ".upload-staging-caption .composer-tools"
    );
    expect(compactCaptionToolbar).toMatch(/height:\s*30px/);

    const previewImage = selectorBlock(".upload-staging-preview");
    // Prepared output dimensions must remain visible: forcing a 100% minimum
    // would upscale resized variants and make them look unchanged but blurrier.
    expect(previewImage).toMatch(/min-inline-size:\s*0/);
    expect(previewImage).not.toMatch(/min-inline-size:\s*100%/);
  });

  test("selected room row uses a logical brand start bar", () => {
    expect(css).toMatch(/border-inline-start-color:\s*var\(--brand\)/);
    expect(css).not.toContain("box-shadow: inset 3px 0 0 0 var(--brand)");
  });

  test("macOS titlebar overlay reserves native traffic light space in the app top bar", () => {
    expect(css).toContain("--macos-traffic-light-inline-space: 96px;");
    expect(appSource).toContain("platform={snapshot.state.domain.locale_profile.platform}");
    const block = selectorBlock('.titlebar[data-platform="macos"]');
    expect(block).toContain("padding-inline-start: var(--macos-traffic-light-inline-space);");
  });

  test("titlebar owns window dragging without a covering overlay element", () => {
    const titlebarBlock = selectorBlock(".titlebar");
    expect(titlebarBlock).toContain("position: relative;");
    expect(css).not.toContain(".titlebar-drag-strip");
    expect(shellSource).toContain('data-tauri-drag-region=""');
    expect(shellSource).toContain("shouldStartTitlebarDrag(event)");
    expect(shellSource).toContain('target.closest("button, input, select, textarea, a, label")');
  });

  test("titlebar reserves visible room for Matrix connection status", () => {
    const titlebarBlock = selectorBlock(".titlebar");
    expect(titlebarBlock).toContain(
      "grid-template-columns: minmax(220px, 1fr) var(--search-scope-column) minmax(132px, max-content);"
    );
    expect(titlebarBlock).not.toContain(" 84px;");
    expect(css).not.toContain(".history");
    expect(shellSource).toContain('className="sync-status"');
    expect(shellSource).toContain('className="sync-status-label"');
    expect(shellSource).toContain('className="sync-status-detail"');
  });

  test("locale-sensitive layout uses logical properties instead of physical left/right declarations", () => {
    expect(css).not.toMatch(
      /\b(?:left|right|margin-left|margin-right|padding-left|padding-right|border-left|border-right|inset-left|inset-right)\s*:/
    );
    expect(css).not.toMatch(/text-align:\s*(?:left|right)\b/);
  });

  test("timeline message action menus open toward the upper inline-start side", () => {
    const baseBlock = selectorBlock(".message-action-menu");
    expect(baseBlock).toContain("inset-inline-end: 0;");
    const aboveBlock = selectorBlock(".message-action-menu.is-above");
    expect(aboveBlock).toContain("inset-block-end: 28px;");
    const belowBlock = selectorBlock(".message-action-menu.is-below");
    expect(belowBlock).toContain("inset-block-start: 28px;");
    expect(baseBlock).not.toContain("inset-block-end");
  });

  test("timeline message action menus can stack above sticky timeline navigation", () => {
    const block = selectorBlock(
      ".timeline-scroll:has(.message-action-menu, .message-forward-menu) > .message-list"
    );
    expect(block).toContain("position: relative;");
    expect(block).toContain("z-index: 12;");
    // The reaction emoji picker renders in the body-level floating layer, so it
    // must not need the message list lifted for it.
    expect(css).not.toContain(":has(.message-action-menu, .message-forward-menu, .timeline-reaction-emoji-picker)");
  });

  test("emoji picker floats over clipped panes with a compact fixed-format layout", () => {
    const pickerBlock = selectorBlock(".emoji-picker");
    // Viewport-fixed so an overflow-clipped pane cannot cut the panel off; the
    // exact rectangle is measured by resolveEmojiPickerPlacement.
    expect(pickerBlock).toContain("position: fixed;");
    expect(pickerBlock).toContain(
      `inline-size: min(${EMOJI_PICKER_INLINE_SIZE_PX}px, calc(100vw - 32px));`
    );
    expect(pickerBlock).toContain(
      `block-size: min(${EMOJI_PICKER_BLOCK_SIZE_PX}px, calc(100vh - 32px));`
    );
    expect(pickerBlock).not.toContain("--emoji-picker-max-block-size");

    // The grid column count has a single owner in EmojiPicker.tsx.
    const gridBlock = selectorBlock(".emoji-picker-grid");
    expect(gridBlock).toContain(
      `grid-template-columns: repeat(var(--emoji-picker-columns, ${EMOJI_PICKER_GRID_COLUMNS}), minmax(0, 1fr));`
    );

    const tabsBlock = selectorBlock(".emoji-picker-tabs");
    expect(tabsBlock).toContain("justify-content: space-between;");
    expect(tabsBlock).not.toContain("overflow-x");

    const bodyBlock = selectorBlock(".emoji-picker-body");
    expect(bodyBlock).toContain("overflow-x: hidden;");
  });

  test("timeline uses Koushi-owned event anchoring rather than browser scroll anchoring", () => {
    const timelineBlock = selectorBlock(".timeline-view");
    const spacerBlock = selectorBlock(".timeline-virtual-spacer");

    expect(timelineBlock).toContain("overflow-anchor: none;");
    expect(spacerBlock).toContain("overflow-anchor: none;");
  });

  test("defines fixed-format sizing tokens for shared GUI controls", () => {
    expectTokens([
      "--icon-button-size",
      "--icon-button-radius",
      "--activity-row-action-size",
      "--activity-row-action-radius",
      "--nav-badge-size",
      "--nav-badge-padding-inline",
      "--nav-badge-font-size",
      "--nav-dot-size",
      "--nav-dot-margin-inline-start",
      "--room-count-min-inline-size",
      "--room-count-block-size",
      "--room-count-padding-inline",
      "--room-count-font-size",
      "--room-avatar-size",
      "--room-avatar-font-size",
      "--message-avatar-column-inline-size",
      "--message-avatar-size",
      "--message-avatar-font-size",
      "--avatar-compact-size",
      "--avatar-compact-radius",
      "--avatar-compact-font-size",
      "--thread-reply-avatar-column-inline-size",
      "--directory-avatar-size",
      "--directory-avatar-font-size",
      "--directory-avatar-compact-size",
      "--directory-avatar-compact-font-size",
      "--receipt-row-gap",
      "--receipt-row-min-block-size",
      "--receipt-row-margin-block-start",
      "--receipt-row-font-size",
      "--receipt-focus-outline-width",
      "--receipt-focus-outline-offset",
      "--receipt-avatar-stack-min-inline-size",
      "--receipt-avatar-size",
      "--receipt-avatar-overlap",
      "--receipt-avatar-border-width",
      "--receipt-avatar-font-size",
      "--receipt-overflow-min-inline-size",
      "--receipt-overflow-padding-inline",
      "--receipt-tooltip-z-index",
      "--receipt-tooltip-offset-block",
      "--receipt-tooltip-gap",
      "--receipt-tooltip-min-inline-size",
      "--receipt-tooltip-max-inline-size",
      "--receipt-tooltip-max-viewport-inline-size",
      "--receipt-tooltip-padding-block",
      "--receipt-tooltip-padding-inline",
      "--receipt-tooltip-font-size",
      "--receipt-tooltip-translate-block",
      "--motion-tooltip-duration"
    ]);
  });

  test("sidebar counters and icon buttons use fixed-format tokens", () => {
    expectBlockUses(selectorBlock(".icon-button"), ["--icon-button-size", "--icon-button-radius"]);
    expectBlockUses(selectorBlock(".activity-row-action"), [
      "--activity-row-action-size",
      "--activity-row-action-radius"
    ]);
    expectBlockUses(selectorBlock(".workspace-button[data-count]::after"), ["--nav-badge-font-size"]);
    expectBlockUses(selectorBlock(".nav-item[data-count]::after"), [
      "--nav-badge-size",
      "--nav-badge-padding-inline",
      "--nav-badge-font-size"
    ]);
    expectBlockUses(
      groupedSelectorBlock(
        /\.nav-item\[data-mention-count\]\s+\.nav-label::after,\s*\.nav-item\[data-live-count\]\s+\.nav-label::before/,
        "nav notification dots"
      ),
      ["--nav-dot-size", "--nav-dot-margin-inline-start"]
    );
    expectBlockUses(selectorBlock(".room-mention-dot"), ["--nav-dot-size"]);
    expectBlockUses(selectorBlock(".room-count"), ["--room-count-min-inline-size", "--room-count-font-size"]);
    expectBlockUses(selectorBlock(".room-count:not(:empty)"), [
      "--room-count-min-inline-size",
      "--room-count-block-size",
      "--room-count-padding-inline"
    ]);
  });

  test("sidebar sort controls read as a compact secondary control row", () => {
    const categoryBlock = selectorBlock(".room-list-category");
    const sortBlock = selectorBlock(".room-list-sort");
    const sortLabelBlock = selectorBlock(".room-list-sort-label");
    const sortButtonBlock = selectorBlock(".room-list-sort-button");
    const selectedSortButtonBlock = selectorBlock(".room-list-sort-button.is-selected");

    expect(categoryBlock).toContain("grid-template-columns: repeat(2, minmax(0, 1fr));");
    expect(categoryBlock).toContain("padding: 3px;");
    expect(sortBlock).toContain("grid-template-columns: auto minmax(0, 1fr) minmax(0, 1fr);");
    expect(sortBlock).toContain("padding: 2px;");
    expect(sortBlock).toContain("background: transparent;");
    expect(sortLabelBlock).toContain("font-size: 11px;");
    expect(sortLabelBlock).toContain("font-weight: 800;");
    expect(sortButtonBlock).toContain("min-height: 24px;");
    expect(sortButtonBlock).toContain("font-size: 11px;");
    expect(sortButtonBlock).toContain("font-weight: 700;");
    expect(selectedSortButtonBlock).toContain("color: var(--brand);");
    expect(selectedSortButtonBlock).toContain("background: var(--brand-weak);");
    expect(selectedSortButtonBlock).not.toContain("var(--brand-contrast)");
  });

  test("formatted message lists use compact chat-message spacing", () => {
    const formattedBodyBlock = selectorBlock(".message-body.message-formatted-body");
    const listBlock = groupedSelectorBlock(
      /\.message-body\.message-formatted-body > ul,\s*\.message-body\.message-formatted-body > ol/,
      "formatted list block"
    );
    const listItemBlock = selectorBlock(".message-body.message-formatted-body li");

    expect(formattedBodyBlock).toContain("display: block;");
    expect(formattedBodyBlock).not.toContain("display: grid;");
    expect(formattedBodyBlock).not.toContain("gap:");
    expect(listBlock).toContain("margin-block: 2px;");
    expect(listBlock).toContain("padding-inline-start: 18px;");
    expect(listItemBlock).toContain("margin-block: 1px;");
    expect(listItemBlock).toContain("line-height: 1.42;");
  });

  test("read marker stays a compact timeline divider", () => {
    const markerBlock = selectorBlock(".read-marker");

    expect(markerBlock).toContain("min-height: 18px;");
    expect(markerBlock).toContain("margin: 4px 0;");
    expect(markerBlock).toContain("gap: 8px;");
    expect(markerBlock).toContain("font-size: 11px;");
  });

  test("compact continuation styling stays density-scoped and accessible", () => {
    const compactMessageBlock = selectorBlock('.desktop[data-density="compact"] .message');
    const continuationBlock = selectorBlock(
      '.desktop[data-density="compact"] .message.is-continuation'
    );
    const avatarBlock = selectorBlock(
      '.desktop[data-density="compact"] .message.is-continuation > .avatar'
    );
    const senderBlock = selectorBlock(
      '.desktop[data-density="compact"] .message.is-continuation .sender'
    );
    const srOnlyBlock = selectorBlock(".sr-only");

    expect(compactMessageBlock).toContain("padding-block: 6px;");
    expect(continuationBlock).toContain("padding-block-start: 2px;");
    expect(avatarBlock).toContain("visibility: hidden;");
    expect(senderBlock.replace(/\\s+/g, " ").trim()).toBe(
      srOnlyBlock.replace(/\\s+/g, " ").trim()
    );
    expect(css).not.toContain('.desktop[data-density="default"] .message.is-continuation');
    expect(css).not.toContain(
      '.desktop[data-density="comfortable"] .message.is-continuation'
    );
  });

  test("avatar and receipt fixed geometry use named tokens", () => {
    expectBlockUses(selectorBlock(".room-avatar"), ["--room-avatar-size", "--room-avatar-font-size"]);
    const activityAvatarImageBlock = selectorBlock(".activity-row-avatar img");
    expect(activityAvatarImageBlock).toContain("width: 100%");
    expect(activityAvatarImageBlock).toContain("height: 100%");
    expect(activityAvatarImageBlock).toContain("object-fit: cover");
    expectBlockUses(selectorBlock(".message"), ["--message-avatar-column-inline-size"]);
    expectBlockUses(selectorBlock(".avatar"), ["--message-avatar-size", "--message-avatar-font-size"]);
    expectBlockUses(selectorBlock(".directory-result"), ["--directory-avatar-size"]);
    expectBlockUses(selectorBlock(".directory-result-avatar"), [
      "--directory-avatar-size",
      "--directory-avatar-font-size"
    ]);
    expectBlockUses(selectorBlock(".thread-reply"), ["--thread-reply-avatar-column-inline-size"]);
    expectBlockUses(selectorBlock(".thread-reply .avatar"), [
      "--avatar-compact-size",
      "--avatar-compact-radius",
      "--avatar-compact-font-size"
    ]);
    expectBlockUses(
      blockFor(
        /@media\s+\(max-width:\s*760px\)\s*\{[\s\S]*?\.avatar\s*\{(?<body>[^}]*)\}/,
        "compact avatar"
      ),
      ["--avatar-compact-size", "--avatar-compact-radius", "--avatar-compact-font-size"]
    );
    expectBlockUses(
      blockFor(
        /@media\s+\(max-width:\s*760px\)\s*\{[\s\S]*?\.directory-result-avatar\s*\{(?<body>[^}]*)\}/,
        "compact directory avatar"
      ),
      ["--directory-avatar-compact-size", "--directory-avatar-compact-font-size"]
    );
  });

  test("receipt row and tooltip sizing use fixed-format tokens", () => {
    const timelineViewBlock = selectorBlock(".timeline-view");
    expect(timelineViewBlock).toContain("overflow-x: hidden");
    const receiptBlock = selectorBlock(".message-receipts");
    expectBlockUses(receiptBlock, [
      "--receipt-row-gap",
      "--receipt-row-min-block-size",
      "--receipt-row-font-size"
    ]);
    expect(receiptBlock).toContain("max-inline-size: 100%");
    expect(receiptBlock).toContain("inline-size: fit-content");
    expect(receiptBlock).toContain("margin-inline-start: auto");
    expectBlockUses(selectorBlock(".message-receipts:focus-visible"), [
      "--receipt-focus-outline-width",
      "--receipt-focus-outline-offset"
    ]);
    expectBlockUses(selectorBlock(".receipt-avatars"), ["--receipt-avatar-stack-min-inline-size"]);
    expectBlockUses(
      groupedSelectorBlock(/\.receipt-reader-avatar,\s*\.receipt-overflow/, "receipt avatars"),
      [
        "--receipt-avatar-size",
        "--receipt-avatar-overlap",
        "--receipt-avatar-border-width",
        "--receipt-avatar-font-size"
      ]
    );
    expectBlockUses(lastSelectorBlock(".receipt-overflow"), [
      "--receipt-overflow-min-inline-size",
      "--receipt-overflow-padding-inline"
    ]);
    // #314: the reader popup is a viewport-fixed floating panel. Its position
    // and size come from resolveFloatingPlacement, so offset, size, and the
    // hover transition tokens no longer belong to this block. The tokens stay
    // defined because `.reaction-tooltip` is still a row-local popup.
    const readerPopupBlock = selectorBlock(".receipt-tooltip");
    expect(readerPopupBlock).toContain("position: fixed");
    expectBlockUses(readerPopupBlock, [
      "--receipt-tooltip-z-index",
      "--receipt-tooltip-gap",
      "--receipt-tooltip-padding-block",
      "--receipt-tooltip-padding-inline",
      "--receipt-tooltip-font-size"
    ]);
    for (const rowLocalOnly of [
      "--receipt-tooltip-offset-block",
      "--receipt-tooltip-translate-block",
      "--motion-tooltip-duration"
    ]) {
      expect(readerPopupBlock).not.toContain(rowLocalOnly);
    }
    expectBlockUses(selectorBlock(".reaction-tooltip"), [
      "--receipt-tooltip-offset-block",
      "--receipt-tooltip-min-inline-size"
    ]);
  });

  test("hover actions float over the row instead of the timestamp flow", () => {
    const block = selectorBlock(".message > .message-actions-floating");
    expect(block).toContain("position: absolute");
    expectBlockUses(block, [
      "--message-actions-z-index",
      "--message-actions-inset-block",
      "--message-actions-inset-inline",
      "--message-actions-gap",
      "--message-actions-padding"
    ]);
    // The row is the positioning context, so the pill stays inside the pane.
    expect(selectorBlock(".message")).toContain("position: relative");
  });

  test("message source dialog stays above other overlays and exposes labeled copy buttons", () => {
    const dialogBlock = selectorBlock(".message-source-dialog");
    expect(dialogBlock).toContain("position: fixed");
    expect(dialogBlock).toContain("z-index: 120");
    expect(selectorBlock(".message-source-copy")).toContain("inline-size: auto");
  });

  test("timeline media viewer uses its own top-level lightbox layer", () => {
    const overlayBlock = selectorBlock(".timeline-media-viewer-overlay");
    expect(overlayBlock).toContain("position: fixed");
    expect(overlayBlock).toContain("z-index: 140");
    expect(selectorBlock(".timeline-media-viewer-toolbar")).toContain(
      "grid-template-columns: minmax(0, 1fr) auto;"
    );
    expect(
      groupedSelectorBlock(
        /\.timeline-media-viewer-menu,\s*\.timeline-media-viewer-forward-menu/,
        "timeline media viewer menus"
      )
    ).toContain("position: absolute");
    expect(selectorBlock(".timeline-media-viewer-menu-item")).toContain(
      "grid-template-columns: 22px minmax(0, 1fr);"
    );
    expect(css).toContain(".media-viewer-backdrop");
  });

  test("app grid exposes separate resize handles for the room list and right panel", () => {
    expect(selectorBlock(".app-grid")).toContain("--right-panel-width");
    expect(css).toContain(".app-grid-resizer");
    expect(css).toContain(".app-grid-right-resizer");
    expect(appSource).toContain("beginSidebarResize");
    expect(appSource).toContain("beginRightPanelResize");
    expect(appSource).toContain('aria-label={t("workspace.resizeRightPanel")}');
  });

  test("application root inherits the containing block instead of using a viewport height", () => {
    const root = selectorBlock(".desktop");
    expect(root).toContain("block-size: 100%;");
    expect(root).toContain("min-block-size: 0;");
    expect(root).not.toMatch(/(?:height|min-height|block-size):\s*100(?:d)?vh/);
  });

  test("the composer math switch never renders hover as the ON state", () => {
    // #453: hover, :focus-visible and the checked state all shared one brand-fill
    // rule, so pointing at an OFF switch made it look ON and the control read as
    // an insert-LaTeX action instead of a toggle. Keep the two treatments apart.
    const hoverBlock = selectorBlock(
      ".composer-math-toggle:hover,\n.composer-math-toggle:focus-visible"
    );
    const checkedBlock = selectorBlock('.composer-math-toggle[aria-checked="true"]');
    expect(hoverBlock).toBeTruthy();
    expect(checkedBlock).toBeTruthy();
    expect(hoverBlock).not.toEqual(checkedBlock);
    // Only the checked state may claim the brand fill.
    expect(hoverBlock).not.toContain("var(--brand-weak)");
    expect(checkedBlock).toContain("var(--brand-weak)");
    // The state must be visible at rest, not only through colour.
    expect(css).toContain(".composer-math-switch-track");
    expect(css).toContain(".composer-math-switch-thumb");
    expect(css).toContain(
      '.composer-math-toggle[aria-checked="true"] .composer-math-switch-thumb'
    );
  });

  test("fixed Lucide icon sizes stay centralized in ICON_SIZE", () => {
    expect(uiSharedSource).toContain("const ICON_SIZE");
    // No icon-rendering module may hard-code a numeric `size={N}`; all fixed sizes
    // must come from ICON_SIZE. After the Phase 2d split this scans the moved
    // component modules too, not only App.tsx.
    for (const [name, source] of iconRenderingSources) {
      expect(source, `${name} should not hard-code Lucide icon sizes`).not.toMatch(
        /size=\{\d+\}/
      );
    }
  });
});
