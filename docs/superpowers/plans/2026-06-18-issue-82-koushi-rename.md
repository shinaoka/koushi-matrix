# Issue #82 — Koushi Rename and Lattice-Light Logo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` or inline execution. Steps use checkbox syntax for tracking.

**Goal:** Rename all user-facing product branding from Kagome to Koushi, add a lattice-light logo asset family, and document the migration decision for internal identifiers.

**Architecture:** User-facing strings live in the React i18n catalog, Tauri bundle metadata, README/design docs, and QA tests. Internal identifiers are migrated from Kagome to Koushi (`chat.koushi.desktop`, `koushi-desktop`, `koushi-desktop` credential service, `app.koushi.local_aliases`) with read-old-write-new migration for persisted keychain and Matrix account-data entries. Logo source SVGs live under `assets/branding/` and are rasterized to the Tauri `icons/` directory.

**Tech Stack:** TypeScript/React i18n, Tauri v2 JSON config, SVG + ImageMagick, Node/Vitest, Cargo.

---

## Task 1: Update React i18n catalog

**Files:**
- Modify: `apps/desktop/src/i18n/messages.ts`
- Test: `apps/desktop/src/i18n/messages.test.ts`

- [x] **Step 1: Rename English user-facing strings**
  - `"app.about"` → `"About Koushi"`
  - `"app.title"` → `"Koushi"`
  - `"auth.matrixDesktop"` → `"Koushi"`
  - `"window.title"` → `"Koushi"`

- [x] **Step 2: Rename Japanese user-facing strings**
  - `"app.about"` → `"Koushi（光子・格子）について"`
  - `"app.title"` → `"Koushi（光子・格子）"`
  - `"auth.matrixDesktop"` → `"Koushi（光子・格子）"`
  - `"window.title"` → `"Koushi（光子・格子）"`

- [x] **Step 3: Add explicit branding test**
  Added `product branding uses Koushi in English and Japanese` to
  `apps/desktop/src/i18n/messages.test.ts` asserting `app.title`, `window.title`,
  and `auth.matrixDesktop` values.

- [x] **Step 4: Run i18n tests**
  Run: `npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts`
  Expected: PASS

---

## Task 2: Update Tauri bundle metadata

**Files:**
- Modify: `apps/desktop/src-tauri/tauri.conf.json`

- [x] **Step 1: Replace user-facing metadata**
  - `productName` → `"Koushi"`
  - `app.windows[0].title` → `"Koushi"`
  - `bundle.publisher` → `"Koushi contributors"`
  - `bundle.copyright` → `"Copyright 2026 Koushi contributors"`
  - `bundle.shortDescription` → `"Koushi — a Matrix desktop client."`
  - `bundle.longDescription` → text using `"Koushi"`
  - Populate `bundle.icon` with generated icon paths.

- [x] **Step 2: Validate Tauri config**
  Run: `npm --prefix apps/desktop run typecheck`
  Expected: PASS (TypeScript check; Tauri JSON schema validated by CLI on build)

---

## Task 3: Update package metadata and static shell fallback

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop-shell/app.js`

- [x] **Step 1: Update package description**
  - `description` → `"Koushi — a desktop Matrix client."`
  - `name` → `koushi-desktop` (repository/package identifier).

- [x] **Step 2: Update desktop-shell fallback**
  - Change default active space name fallback from `"Kagome"` to `"Koushi"`.

---

## Task 4: Update docs and canon rules

**Files:**
- Modify: `README.md`
- Modify: `docs/design/ui-design-language.md`
- Modify: `docs/design/palette.svg`
- Modify: `docs/qa/integration-edge-cases.md`
- Modify: `docs/policies/engineering-rules.md`
- Modify: `REPOSITORY_RULES.md`
- Modify: `AGENTS.md`

- [x] **Step 1: README rename**
  - Title `# Kagome (籠目)` → `# Koushi (光子・格子)`
  - Body text uses Koushi and explains 光子/格子 wordplay.

- [x] **Step 2: Design docs rename**
  - `docs/design/ui-design-language.md` title and body.
  - `docs/design/palette.svg` title text.

- [x] **Step 3: Canon docs**
  - `docs/qa/integration-edge-cases.md` update naming reference.
  - `docs/policies/engineering-rules.md` update product title reference.
  - `REPOSITORY_RULES.md` change "Kagome-specific JSON..." to "product-specific JSON...".
  - `AGENTS.md` add a note that `app.kagome.local_aliases` remains the persisted account-data key for migration compatibility despite the Koushi rename.

---

## Task 5: Update tests and QA strings

**Files:**
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`
- Modify: `apps/desktop/src/domain/timelineStore.test.ts`
- Modify: `apps/desktop/src/domain/desktopModel.test.ts`
- Modify: `apps/desktop/src/scripts/releaseScripts.test.ts`

- [x] **Step 1: Replace Kagome test strings**
  - Device names, export paths, smoke messages, and model test labels to use Koushi.

- [x] **Step 2: Run focused tests**
  Run:
  - `npm --prefix apps/desktop run test -- --run src/scripts/releaseScripts.test.ts src/domain/desktopModel.test.ts src/domain/timelineStore.test.ts`
  - `npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts`
  Expected: PASS

---

## Task 6: Generate Koushi lattice-light logo assets

**Files:**
- Create: `apps/desktop/src-tauri/icons/icon.svg` (source)
- Create: `apps/desktop/src-tauri/icons/icon.png` (512x512, replacing placeholder)
- Create: `apps/desktop/src-tauri/icons/32x32.png`
- Create: `apps/desktop/src-tauri/icons/128x128.png`
- Create: `apps/desktop/src-tauri/icons/128x128@2x.png`
- Create: `apps/desktop/src-tauri/icons/icon.ico`
- Create: `apps/desktop/src-tauri/icons/icon.icns`
- Create: `scripts/generate-koushi-icons.sh`

- [x] **Step 1: Write SVG source**
  Dark blue rounded-square background, light-blue grid lines, diagonal photon beams, central glow.

- [x] **Step 2: Rasterize to PNG/ICO/ICNS**
  Use ImageMagick and a small Python ICNS packer (no external deps). Commit the generator script so assets are reproducible.

- [x] **Step 3: Wire icons in Tauri config**
  Set `bundle.icon` to the generated paths.

---

## Task 7: Verification and close

- [x] **Step 1: Text-artifact scan**
  Run: `rg "Kagome|kagome" README.md REPOSITORY_RULES.md AGENTS.md docs/architecture/state-machine.md apps/desktop --max-depth 10`
  Expected: Only internal identifiers (`kagome-desktop-app`, `chat.kagome.desktop`, `app.kagome.local_aliases`) remain. No user-facing Kagome strings.

- [x] **Step 2: Codex diff review**
  Ran `codex review -` against the staged diff. Initial review found three
  findings; fixes applied:
  - Promoted the `app.kagome.local_aliases` retention rule from `AGENTS.md` to
    a new durable `REPOSITORY_RULES.md` "Product Identity And Migration" section.
  - Fixed ICNS type codes in `scripts/lib/generate-icns.py` (256px now maps to
    `ic13`, the 128x128@2x slot, instead of `ic11`).
  - Kept the 256x256 frame in `icon.ico` by removing `-delete 0` from the
    ImageMagick command in `scripts/generate-koushi-icons.sh`.
  Re-review: no discrete correctness, security, or contract issues.

- [x] **Step 3: Full validation suite**
  Run:
  - `cargo fmt --check` — PASS
  - `git diff --check` — PASS
  - `npm --prefix apps/desktop run typecheck` — PASS
  - `npm --prefix apps/desktop run test` — PASS (343 tests)
  - `npm --prefix apps/desktop run build` — PASS
  - `cargo test -p koushi-key credential_backend` — PASS
  - `npm --prefix apps/desktop run test:ipc-contract` — PASS

- [x] **Step 4: Close #82**
  Summarize changes on the issue, noting retained internal identifiers and migration rationale.
