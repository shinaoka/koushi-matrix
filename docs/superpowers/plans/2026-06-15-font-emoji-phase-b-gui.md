# Font And Emoji Substrate Phase B GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete Issue #5 by loading licensed Inter/Twemoji COLR assets, applying Rust-owned typography profile tokens through root CSS, exposing settings controls, and proving the behavior in browser-headless tests.

**Architecture:** React consumes `snapshot.state.typography_profile` and applies root dataset tokens only. CSS owns font stacks through root variables; components inherit those variables and do not branch on OS or font choices. The settings panel dispatches typed `update_settings` patches and renders returned snapshots.

**Tech Stack:** npm package assets through Vite, React/TypeScript, Playwright headless harness, Vitest component rendering, release notices.

---

## Tasks

### Task 1: Browser-Headless RED

**Files:**
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

- [x] Add a Playwright test that pushes a snapshot with `typography_profile.font = "inter"` and `typography_profile.emoji = "twemojiColr"`, then expects:
  - `document.documentElement.dataset.uiFont === "inter"`
  - `document.documentElement.dataset.emojiFont === "twemojiColr"`
  - computed `--font-ui` contains `Inter`
  - computed `--font-emoji` contains `Twemoji`
  - `document.fonts.check('14px "Inter"')` is true
  - `document.fonts.check('14px "Twemoji"')` is true after font loading

Run:

```bash
cd apps/desktop && npx playwright test e2e/basic-operations.spec.ts --grep "typography profile" --workers=1
```

Expected: FAIL because root dataset tokens and font assets are not wired.

### Task 2: Asset Loading And Root Tokens

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop/package-lock.json`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/styles.css`

- [x] Install exact package versions:

```bash
npm --prefix apps/desktop install @fontsource/inter@5.2.8 twemoji-colr-font@15.0.3 --save-exact
```

- [x] Import font CSS once from `styles.css`, which is shared by production and the Playwright app harness:

```css
@import "@fontsource/inter/400.css";
@import "@fontsource/inter/500.css";
@import "@fontsource/inter/600.css";
@import "@fontsource/inter/700.css";
@import "twemoji-colr-font/twemoji.css";
```

- [x] Add an App effect that maps `snapshot.state.typography_profile` to root datasets:

```ts
document.documentElement.dataset.uiFont = profile.font;
document.documentElement.dataset.emojiFont = profile.emoji;
document.documentElement.dataset.fontAsset = profile.font_asset;
document.documentElement.dataset.emojiAsset = profile.emoji_asset;
```

- [x] Change root CSS to variables:

```css
--font-system-ui: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
--font-system-emoji: "Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji", emoji;
--font-ui: var(--font-system-ui);
--font-emoji: var(--font-system-emoji);
--font-message: var(--font-ui);
font-family: var(--font-ui);
```

and select bundled preferences with:

```css
:root[data-ui-font="inter"] {
  --font-ui: Inter, var(--font-system-ui);
}

:root[data-emoji-font="twemojiColr"] {
  --font-emoji: "Twemoji", var(--font-system-emoji);
}
```

### Task 3: Settings Controls

**Files:**
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Modify: `apps/desktop/src/components/UserSettingsPanel.test.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`

- [x] Add message IDs for font and emoji settings labels/options.
- [x] Add segmented controls in the Appearance section for UI font and emoji font.
- [x] Dispatch only typed `SettingsPatch` values:

```ts
onUpdateSettings({ typography: { ...settings.values.typography, font: value } });
onUpdateSettings({ typography: { ...settings.values.typography, emoji: value } });
```

- [x] Update component tests to assert labels and selected states.

### Task 4: Attribution And Gates

**Files:**
- Modify: `THIRD_PARTY_NOTICES.md`
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/plans/2026-06-15-font-emoji-phase-b-gui.md`

- [x] Add notices for `@fontsource/inter@5.2.8` and `twemoji-colr-font@15.0.3`, both `OFL-1.1`, with local path `apps/desktop/node_modules/...` and Vite-bundled release assets.
- [x] Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run src/components/UserSettingsPanel.test.tsx
cd apps/desktop && npx playwright test e2e/basic-operations.spec.ts --grep "typography profile" --workers=1
npm --prefix apps/desktop run build
npm --prefix apps/desktop run qa:secret-scan
git diff --check
```

Expected: all PASS before closing #5.
