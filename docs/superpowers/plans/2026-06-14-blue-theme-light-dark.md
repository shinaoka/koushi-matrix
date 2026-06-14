# Blue Token System + Light/Dark Theming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the blue identity + light/dark theming half of Issue #11 by turning `apps/desktop/src/styles.css` into a complete semantic-token system (blue light set + dark set), following the OS theme via `prefers-color-scheme`, with a `data-theme` hook ready for a future user toggle.

**Architecture:** Presentation-only. Every color becomes a CSS custom property defined once on `:root` (light) and overridden in a dark block applied two ways — `@media (prefers-color-scheme: dark)` (OS follow) and `:root[data-theme="dark"]` (explicit). No React or Rust state is added: theme *selection* (a persisted `system|light|dark` choice) is deferred to the Rust `SettingsState` in Issue #6; until then React never sets `data-theme`, so dark mode is driven entirely by the OS. No reducer transitions change, so no state-machine diagram edits are required.

**Tech Stack:** CSS custom properties; Vitest source-contract tests (`renderToStaticMarkup` + `readFileSync`); Playwright (`emulateMedia({colorScheme})`) for real computed-color verification.

---

## Scope (read before starting)

**In scope (Issue #11 Phase B — theming):**
- Replace the purple ramp (`--rail #34123d`, `--brand #5b236a`, green `--accent #2f9d68`) with the blue semantic token set from `docs/design/ui-design-language.md` §2.
- Light + dark token sets; `color-scheme`; OS-follow via `prefers-color-scheme`; `data-theme` override hook.
- Tokenize every hardcoded hex across the stylesheet so dark mode actually applies to rail, sidebar, timeline, composer, message rows, dialogs, menus, thread pane, settings, auth, titlebar.
- Presentation states from existing Rust-owned data: selected row (`--brand-weak` + 3px `--brand` left bar), unread pill (`--unread`), mention dot (`--mention`), presence (`--success`).
- Tests: CSS source contract (vitest), light/dark computed color (Playwright).

**Deferred (do NOT build here; documented in Task 1):**
- **Theme selection UI / persisted choice** → Issue #6 (`SettingsState`). We ship OS-follow + the `data-theme` hook so #6 only has to set the attribute.
- **Favourites / Low-priority room sections** → needs Rust `m.tag` account-data modeling. `RoomSummary` has no tags today; categorizing rooms in React would invent product state and violate the canon. Filed/tracked separately; this plan keeps the existing section structure and only restyles it.
- **Inter font bundling** → font/emoji track (Issue #5).

---

## Authoritative token values (from `docs/design/ui-design-language.md` §2)

These are the exact values every task below references.

| token | light | dark |
| --- | --- | --- |
| `--brand` | `#2D6FEF` | `#5C8DF6` |
| `--brand-hover` | `#1F59D0` | `#79A2F8` |
| `--brand-weak` | `#E7F0FE` | `#1B2942` |
| `--brand-contrast` | `#FFFFFF` | `#0A111F` |
| `--rail` | `#16213E` | `#0A111F` |
| `--rail-item` | `#27324F` | `#1B2942` |
| `--sidebar` | `#F5F7FB` | `#111A2B` |
| `--surface` | `#FFFFFF` | `#0E1726` |
| `--surface-hover` | `#EEF2F8` | `#1A2740` |
| `--text` | `#0F1B2D` | `#E6ECF5` |
| `--text-muted` | `#5B6B82` | `#93A0B4` |
| `--text-faint` | `#93A0B4` | `#5B6B82` |
| `--line` | `#E3E8F0` | `#1E2A40` |
| `--unread` | `#2D6FEF` | `#5C8DF6` |
| `--danger` (= mention) | `#E5484D` | `#F2575C` |
| `--success` | `#1A9E6C` | `#34B988` |
| `--warning` | `#C98A1B` | `#E0A53A` |

Theme-independent (light block only): `--r-sm:6px; --r-md:8px; --r-lg:12px; --r-pill:999px;`

## Token rename map (old → new)

The current stylesheet uses these tokens; rename file-wide as part of Task 4:

| old token | new token | notes |
| --- | --- | --- |
| `--muted` | `--text-muted` | |
| `--faint` | `--text-faint` | |
| `--brand-2` | `--brand` | blue secondary collapses into brand |
| `--accent` (presence/online uses) | `--success` | presence dots, `.user-presence` ring |
| `--accent` (action uses) | `--brand` | `.send-button.ready`, `.dialog-button.is-primary` |
| `--rail-strong` | `--rail-item` | |
| `--sidebar-line` | `--line` | |
| `--surface-muted` | `--brand-weak` | tinted/selected surfaces become brand-weak |

## Literal → token map (apply file-wide in Task 4)

Every remaining hardcoded hex maps to a semantic token. Grouped by role:

- **White surfaces** `#ffffff`, `#fff` (cards/inputs/panels) → `var(--surface)`.
- **Body backdrop** `#121016` → `var(--rail)`.
- **Neutral text** `#1f1f26`, `#2d2931`, `#211a28`, `#221628`, `#32475d`, `#3b3440`, `#213047`, `#234c72`, `#355269`, `#285047` → `var(--text)`; secondary greys `#6e6473`, `#5b7390`, `#5f4a68`, `#4d3656`, `#4b4350`, `#4e3b57`, `#7a7280`, `#6d5176`, `#4d4352` → `var(--text-muted)`; faint `#9a90a1`, `#d7c9dc` → `var(--text-faint)`.
- **Borders/dividers** `#e5e1e8`, `#ded0e3`, `#d8e3ef`, `#e7eef6`, `#d8d1dc`, `#d9d1df`, `#d9d2dc`, `#d5d0d8`, `#d9e1ea`, `#e2d6c1`, `#e4d8c4`, `#eadfc9`, `#eadcf0`, `#bcd2e7`, `#bba9c2`, `#d8e0d2` → `var(--line)`.
- **Brand-tinted fills** (selected/hover/info chips) `#f7fbff`, `#edf6ff`, `#f3eef6`, `#f4ecf6`, `#f8f3f9`, `#f0e7f4`, `#f5f8fb`, `#f1f7f2`, `#e5f0e7`, `#f6f1f8`, `#f7f2... ` → `var(--brand-weak)`.
- **Rail text on dark rail** `#f6ecfa`, `#eadcf0`, `#f7eefb`, `#f7eefb` → `var(--brand-contrast)`; rail inactive tiles `rgb(255 255 255 / 16%)` stay as-is (translucent over rail) OR `var(--rail-item)`.
- **Brand actions/hover** purple hovers `#6a3590`, `#7e5a8f`, `#884bb3`(rgb) → `var(--brand-hover)`.
- **Presence / success / online** `#2f9d68`, `#64c98e`, `#66d08f`, `#4bb96b`, `#3c7d7b`, `#1e6a49`, `#dff3e8` → `var(--success)`.
- **Danger / mention / error** `#e5484d`(new), `#d14545`, `#f15c5c`, `#f1c7c7`, `#8f2d2d`, `#fff3f3`, `#b42318`, `#8f1f15`, `#fff0ee`, `#d55654` → `var(--danger)` (text vs bg chosen by context).
- **Warning / amber** `#f4c247`, `#fff1c7`, `#7d5c18`, `#fff0cc`, `#7c4d17`, `#ffe58a`, `#fff2cd`, `#73521b` → `var(--warning)`.
- **Neutral chip/keys** `#f8f7f8`, `#f0edf2`, `#ece8ef`, `#f5f2f6` → `var(--surface-hover)`.

Shadows/overlays: replace ad-hoc `rgb(25 17 31 / X%)` overlays with `rgb(15 27 45 / 45%)` (design §6 overlay) and dialog/menu shadow with the elevation token `0 12px 40px rgb(15 27 45 / 18%)` (light) — acceptable to keep a single light shadow; dark refinement optional.

---

### Task 1: Phase A — record theming-ownership decision in canon

**Files:**
- Modify: `docs/architecture/state-machine.md` (append a short "Appearance / theme ownership" note near the existing presentation-vs-product-state guidance)

- [ ] **Step 1: Add the ownership note**

Append a subsection stating exactly:

```markdown
### Appearance / theme ownership

Theme *appearance* is split deliberately:

- **OS-follow theming is presentation-only.** The dark token set is applied by
  `@media (prefers-color-scheme: dark)` in `styles.css`. No React or Rust state
  participates; nothing is dispatched, nothing is stored.
- **An explicit user theme choice (`system | light | dark`) is product state**
  and is therefore Rust-owned. It is deferred to `SettingsState` (Issue #6).
  When it lands, React applies it by setting `data-theme` / `color-scheme` on
  the root element; the CSS `:root[data-theme="dark"]` block already exists for
  this. React must not store the chosen theme as its own product state.

Selection, unread, reply, thread, search, and right-panel modes remain
Rust-owned (`AppState.navigation`, `rooms[].unread_count`/`highlight_count`,
`timeline.composer.mode`, `thread`, `search`, right-panel mode). The theming
work changes presentation only and adds no reducer transitions.
```

- [ ] **Step 2: Commit**

```bash
git add docs/architecture/state-machine.md
git commit -m "docs(state-machine): record appearance/theme ownership split (Issue #11 Phase A)"
```

---

### Task 2: CSS contract test for the blue token system (RED)

**Files:**
- Create: `apps/desktop/src/styles.contract.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

describe("styles.css token system", () => {
  test("defines the blue brand token in light and dark", () => {
    expect(css).toContain("--brand: #2D6FEF;");
    expect(css).toMatch(/:root\[data-theme="dark"\]/);
    expect(css).toContain("--brand: #5C8DF6;");
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
      "--text",
      "--text-muted",
      "--text-faint",
      "--line",
      "--unread",
      "--danger",
      "--success",
      "--warning"
    ]) {
      expect(css).toContain(`${token}:`);
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
      expect(css).not.toContain(legacy);
    }
  });
});
```

- [ ] **Step 2: Run and verify it fails**

Run: `npm --prefix apps/desktop run test -- styles.contract`
Expected: FAIL — current `:root` has `--brand: #5b236a;`, `--accent`, no dark block.

---

### Task 3: Define the token blocks (root + dark) — greens tests 1–3

**Files:**
- Modify: `apps/desktop/src/styles.css:1-20` (the `:root` block) and add dark blocks immediately after.

- [ ] **Step 1: Replace the `:root` block**

Replace lines 1–20 (`:root { … }`) with:

```css
:root {
  color-scheme: light dark;

  --brand: #2D6FEF;
  --brand-hover: #1F59D0;
  --brand-weak: #E7F0FE;
  --brand-contrast: #FFFFFF;
  --rail: #16213E;
  --rail-item: #27324F;
  --sidebar: #F5F7FB;
  --surface: #FFFFFF;
  --surface-hover: #EEF2F8;
  --text: #0F1B2D;
  --text-muted: #5B6B82;
  --text-faint: #93A0B4;
  --line: #E3E8F0;
  --unread: #2D6FEF;
  --danger: #E5484D;
  --success: #1A9E6C;
  --warning: #C98A1B;

  --r-sm: 6px;
  --r-md: 8px;
  --r-lg: 12px;
  --r-pill: 999px;

  font-family:
    Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI",
    sans-serif;
}

/* Dark token set: shared by OS-follow and explicit data-theme="dark". */
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --brand: #5C8DF6;
    --brand-hover: #79A2F8;
    --brand-weak: #1B2942;
    --brand-contrast: #0A111F;
    --rail: #0A111F;
    --rail-item: #1B2942;
    --sidebar: #111A2B;
    --surface: #0E1726;
    --surface-hover: #1A2740;
    --text: #E6ECF5;
    --text-muted: #93A0B4;
    --text-faint: #5B6B82;
    --line: #1E2A40;
    --unread: #5C8DF6;
    --danger: #F2575C;
    --success: #34B988;
    --warning: #E0A53A;
  }
}

:root[data-theme="dark"] {
  color-scheme: dark;
  --brand: #5C8DF6;
  --brand-hover: #79A2F8;
  --brand-weak: #1B2942;
  --brand-contrast: #0A111F;
  --rail: #0A111F;
  --rail-item: #1B2942;
  --sidebar: #111A2B;
  --surface: #0E1726;
  --surface-hover: #1A2740;
  --text: #E6ECF5;
  --text-muted: #93A0B4;
  --text-faint: #5B6B82;
  --line: #1E2A40;
  --unread: #5C8DF6;
  --danger: #F2575C;
  --success: #34B988;
  --warning: #E0A53A;
}

:root[data-theme="light"] {
  color-scheme: light;
}
```

- [ ] **Step 2: Run contract test**

Run: `npm --prefix apps/desktop run test -- styles.contract`
Expected: tests 1–3 PASS; test 4 ("no legacy literals") still FAILS (component bodies still use `--accent`, `#5b236a` hovers, etc.).

---

### Task 4: Tokenize every component — greens test 4

**Files:**
- Modify: `apps/desktop/src/styles.css` (body, all component rules below the token blocks)

Apply the **token rename map** and **literal → token map** above to the entire stylesheet. Work top-to-bottom in clusters so nothing is missed:

- [ ] **Step 1: Shell** — `body` (`#121016`→`var(--rail)`), `.boot-screen`, `.auth-*` (the `var(--rail)` gradient keeps `--rail`; `#ffffff`→`var(--surface)`; `--muted`→`--text-muted`; `--faint`→`--text-faint`; auth-error reds → `var(--danger)`), `.titlebar`/`.top-search`/`.scope-select`/`.sync-*` (rail text `#f6ecfa`→`var(--brand-contrast)`, sync dots → `--success`/`--warning`/`--danger`/`--text-faint`).
- [ ] **Step 2: Rail** — `.workspace-rail`, `.workspace-button` (`.is-active` keeps white-on-rail; `data-count` badge bg `#d14545`→`var(--danger)`, border stays `var(--rail)`), `.user-presence` ring `var(--accent)`→`var(--success)`.
- [ ] **Step 3: Sidebar + rows** — `.sidebar` (`--sidebar-line`→`--line`), `.section-title` (`#5f4a68`→`var(--text-muted)`), `.nav-item`/`.room-item` (text `#4d3656`→`var(--text-muted)`; hover `rgb(255 255 255 / 60%)`→`var(--surface-hover)`; `.is-active` bg `var(--brand)` + add `color: var(--brand-contrast)`), `.presence-dot` `var(--accent)`→`var(--success)`, `.room-count` add `font-variant-numeric: tabular-nums;`.
- [ ] **Step 4: Timeline + messages** — `.channel-*`, `.member-pill`, `.tabs`/`.tab.is-active`, `.message:hover`→`var(--surface-hover)`, `.avatar` `var(--accent)`→`var(--brand)` (and `.avatar.bot` `#d55654`→`var(--danger)`), `.message-body` `#2d2931`→`var(--text)`, `.message-edited`/`.message-send-state` amber → `var(--warning)` family, `.message-action:hover` → `var(--brand)` + `var(--brand-contrast)`, `.message-redacted` → `var(--text-muted)`.
- [ ] **Step 5: Reactions + thread chips** — `.reaction-pill` (neutral `#f8f3f9`→`var(--surface-hover)`, `[data-reacted-by-me="true"]` → `var(--brand-weak)` + `var(--brand)`), `.reaction-pill-count`→`var(--brand)`, `.reaction-picker*` → `var(--surface)`/`var(--line)`/`var(--brand)`, `.thread-summary-chip` green → neutral `var(--surface-hover)`/`var(--text-muted)` with `:hover` `var(--brand-weak)`, `.reply-link` `var(--brand-2)`→`var(--brand)`, focus outlines `var(--brand-2)`→`var(--brand)`.
- [ ] **Step 6: Composer + dialogs + menus** — `.composer*`/`.thread-composer*` (`#ffffff`→`var(--surface)`, tools bg `#f8f7f8`→`var(--surface-hover)`), `.send-button.ready` `var(--accent)`→`var(--brand)` + `color: var(--brand-contrast)`, `.composer-reply-banner` `#f3eef6`→`var(--brand-weak)`, `.composer-reply-label`→`var(--brand)`, `.dialog-*` (overlay → `rgb(15 27 45 / 45%)`, `.is-primary` `var(--accent)`→`var(--brand)`), `.context-menu*` (`#ffffff`→`var(--surface)`, hover → `var(--surface-hover)`, `.destructive` → `var(--danger)`).
- [ ] **Step 7: Thread pane + settings + account switcher** — `.thread-pane` `var(--surface-muted)`→`var(--surface)` (or `var(--sidebar)`), thread borders → `var(--line)`, all `.settings-*`/`.shortcut-*`/`.account-switcher-*` `#ffffff`→`var(--surface)`, greys → `--text`/`--text-muted`, status chips → `--success`/`--warning`/`--surface-hover`, avatar `#3c7d7b`→`var(--brand)`.

- [ ] **Step 8: Run contract test**

Run: `npm --prefix apps/desktop run test -- styles.contract`
Expected: ALL 4 tests PASS (no `--accent`, `--brand-2`, `--muted:`, `--faint:`, or legacy purple/green hex remain).

- [ ] **Step 9: Commit**

```bash
git add apps/desktop/src/styles.css apps/desktop/src/styles.contract.test.ts
git commit -m "feat(desktop): blue semantic token system with light/dark theming (Issue #11)"
```

---

### Task 5: Selected / unread / mention presentation states

**Files:**
- Modify: `apps/desktop/src/styles.css` (`.room-item.is-active`, add `.room-item` unread/mention affordances)
- Modify: `apps/desktop/src/styles.contract.test.ts`

- [ ] **Step 1: Extend the contract test (RED)**

Add to `styles.contract.test.ts`:

```ts
test("selected room row has a brand left bar", () => {
  expect(css).toMatch(/\.room-item\.is-active[^}]*box-shadow|\.room-item\.is-active[^}]*border-inline-start/);
});
```

Run: `npm --prefix apps/desktop run test -- styles.contract` → new test FAILS.

- [ ] **Step 2: Implement selected-row left bar**

In `.nav-item.is-active, .room-item.is-active`, replace the solid brand fill with the design's selected treatment:

```css
.nav-item.is-active,
.room-item.is-active {
  color: var(--brand-hover);
  background: var(--brand-weak);
  box-shadow: inset 3px 0 0 0 var(--brand);
}
```

- [ ] **Step 3: Run and verify GREEN**

Run: `npm --prefix apps/desktop run test -- styles.contract`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/styles.css apps/desktop/src/styles.contract.test.ts
git commit -m "feat(desktop): selected/unread/mention row affordances per design spec"
```

---

### Task 6: Playwright light/dark computed-color test

**Files:**
- Modify: `apps/desktop/src/test/harnessMain.tsx` (add the stylesheet import so the harness renders real CSS)
- Create: `apps/desktop/e2e/theme.spec.ts`

- [ ] **Step 1: Import styles into the harness**

Add near the top of `src/test/harnessMain.tsx` (after existing imports):

```ts
import "../styles.css";
```

- [ ] **Step 2: Write the e2e test**

```ts
import { expect, test } from "@playwright/test";

async function railBackground(page: import("@playwright/test").Page): Promise<string> {
  return page.evaluate(() => {
    const rail = document.querySelector(".workspace-rail");
    return rail ? getComputedStyle(rail).backgroundColor : "";
  });
}

test("the space rail follows the OS color scheme", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  await expect(page.getByRole("navigation", { name: "Workspaces" })).toBeVisible();
  const light = await railBackground(page);

  await page.emulateMedia({ colorScheme: "dark" });
  const dark = await railBackground(page);

  // #16213E light rail vs #0A111F dark rail.
  expect(light).toBe("rgb(22, 33, 62)");
  expect(dark).toBe("rgb(10, 17, 31)");
  expect(light).not.toBe(dark);
});
```

- [ ] **Step 3: Run the e2e test**

Run: `npm --prefix apps/desktop run test:e2e -- theme`
Expected: PASS (rail background differs and matches the light/dark token values).

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/test/harnessMain.tsx apps/desktop/e2e/theme.spec.ts
git commit -m "test(desktop): verify OS light/dark theming in headless Chromium"
```

---

### Task 7: Full gate sweep + issue sync

- [ ] **Step 1: Run the frontend gates**

```bash
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run build
npm --prefix apps/desktop run test:e2e
```

Expected: all green. (The pre-existing `basic-operations.spec.ts` full-run flake documented in `AGENTS.md` is unrelated; confirm in isolation if it appears.)

- [ ] **Step 2: Check Issue #11 boxes that are now done**

Tick in Issue #11: blue palette, `data-theme`/`color-scheme` light/dark, component alignment (token-driven), light/dark + selected/unread/mention coverage, "no Element/green," "first viewport shows the app shell." Leave Favourites/Low-priority section IA, theme toggle, and Inter font unticked with a note pointing to #6 / the tags follow-up.

```bash
gh issue comment 11 --body "Theming half landed: blue token system + OS light/dark + presentation states + Playwright color test. Deferred per canon: theme-selection persistence → #6 (SettingsState); Favourites/Low-priority section IA → needs Rust m.tag modeling; Inter font → #5."
```

- [ ] **Step 3: Final commit if anything is uncommitted**

```bash
git status
```

---

## Self-Review

1. **Spec coverage (#11 Phase B):** blue palette → Task 3/4; `data-theme`+`color-scheme` light/dark → Task 3; component alignment → Task 4 (token-driven) + Task 5; new visible labels via i18n → N/A (no new labels; section relabel deferred with the tags work); logical CSS → existing file already uses `margin-inline`/`padding-inline`; light/dark + selected/unread/mention + long-label → Task 5 (CSS) + Task 6 (Playwright). Acceptance: no Element/green → Task 2 test 4 enforces it; first viewport is the shell → harness already mounts it (a11y spec proves landmarks). Gaps: Favourites/Low-priority IA and theme toggle are *intentionally* deferred (Task 1 documents why).
2. **Placeholder scan:** token blocks, mapping table, and test code are all literal; "apply mapping file-wide" is an exhaustive transformation, not a TODO.
3. **Type/name consistency:** token names match `docs/design/ui-design-language.md` §2 exactly; rename map removes every old token so no dangling `var(--accent)` etc. remains (Task 2 test 4 guards this).
