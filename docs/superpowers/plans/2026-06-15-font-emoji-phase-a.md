# Font And Emoji Substrate Phase A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete Issue #5 Phase A by exposing a Rust-owned font/emoji display profile, documenting the asset/fallback policy, and keeping frontend snapshots React-independent.

**Architecture:** `matrix-desktop-state` owns typography settings and resolves a pure `TypographyDisplayProfile` from those settings plus a platform profile. Tauri serializes that profile into the frontend snapshot; React will later consume it as data for root attributes and CSS tokens, without choosing font or emoji semantics locally.

**Tech Stack:** Rust reducer/state crate tests first; Tauri DTO serialization tests; TypeScript domain/fake snapshots kept in sync; docs updates in architecture/policy/AGENTS.

---

## Scope

In scope for Phase A:

- Add a pure `resolve_typography_display_profile()` helper in `matrix-desktop-state`.
- Expose `typography_profile` in the Tauri frontend snapshot next to `locale_profile`.
- Mirror the profile in TypeScript domain types, browser fake snapshots, app harness snapshots, and the Tauri IPC mock.
- Document asset policy and cross-platform fallback behavior.

Out of scope for Phase A:

- Installing or bundling font packages.
- CSS `@font-face` declarations and root dataset wiring.
- Settings panel controls for typography.
- `document.fonts.check()` browser-headless assertions. Those are Phase B.

## Data Contract

Add this Rust-owned profile shape:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypographyDisplayProfile {
    pub font: FontPreference,
    pub emoji: EmojiPreference,
    pub platform: DisplayPlatform,
    pub font_asset: TypographyAssetStatus,
    pub emoji_asset: TypographyAssetStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypographyAssetStatus {
    SystemFallback,
    BundledPreferred,
}
```

Mapping:

- `font = System` -> `font_asset = SystemFallback`
- `font = Inter` -> `font_asset = BundledPreferred`
- `emoji = System` -> `emoji_asset = SystemFallback`
- `emoji = TwemojiColr` -> `emoji_asset = BundledPreferred`

The profile must never include Matrix identifiers, homeserver URLs, device IDs, event IDs, message bodies, filenames, raw errors, credentials, or local paths.

---

### Task 1: Rust Typography Profile

**Files:**
- Create: `crates/matrix-desktop-state/src/typography_profile.rs`
- Create: `crates/matrix-desktop-state/tests/typography_display_profile.rs`
- Modify: `crates/matrix-desktop-state/src/lib.rs`

- [x] **Step 1: Write the failing profile tests**

Create `crates/matrix-desktop-state/tests/typography_display_profile.rs`:

```rust
use matrix_desktop_state::{
    DisplayPlatform, EmojiPreference, FontPreference, TypographyAssetStatus,
    TypographySettings, resolve_typography_display_profile,
};
use serde_json::json;

#[test]
fn default_typography_resolves_to_system_assets_on_each_platform() {
    for platform in [
        DisplayPlatform::Macos,
        DisplayPlatform::Windows,
        DisplayPlatform::Linux,
    ] {
        let profile =
            resolve_typography_display_profile(&TypographySettings::default(), platform);

        assert_eq!(profile.font, FontPreference::System);
        assert_eq!(profile.emoji, EmojiPreference::System);
        assert_eq!(profile.platform, platform);
        assert_eq!(profile.font_asset, TypographyAssetStatus::SystemFallback);
        assert_eq!(profile.emoji_asset, TypographyAssetStatus::SystemFallback);
    }
}

#[test]
fn bundled_preferences_request_bundled_assets_with_system_fallbacks() {
    let profile = resolve_typography_display_profile(
        &TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        },
        DisplayPlatform::Linux,
    );

    assert_eq!(profile.font, FontPreference::Inter);
    assert_eq!(profile.emoji, EmojiPreference::TwemojiColr);
    assert_eq!(profile.font_asset, TypographyAssetStatus::BundledPreferred);
    assert_eq!(profile.emoji_asset, TypographyAssetStatus::BundledPreferred);
}

#[test]
fn typography_profile_serializes_as_the_frontend_contract() {
    let profile = resolve_typography_display_profile(
        &TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        },
        DisplayPlatform::Windows,
    );

    assert_eq!(
        serde_json::to_value(profile).unwrap(),
        json!({
            "font": "inter",
            "emoji": "twemojiColr",
            "platform": "windows",
            "font_asset": "bundledPreferred",
            "emoji_asset": "bundledPreferred"
        })
    );
}
```

- [x] **Step 2: Run the focused test to verify RED**

Run:

```bash
cargo test -p matrix-desktop-state --test typography_display_profile
```

Expected: FAIL because `TypographyAssetStatus` and `resolve_typography_display_profile` do not exist.

- [x] **Step 3: Implement the minimal profile resolver**

Create `crates/matrix-desktop-state/src/typography_profile.rs` with the data contract above and:

```rust
pub fn resolve_typography_display_profile(
    settings: &TypographySettings,
    platform: DisplayPlatform,
) -> TypographyDisplayProfile {
    TypographyDisplayProfile {
        font: settings.font.clone(),
        emoji: settings.emoji.clone(),
        platform,
        font_asset: match settings.font {
            FontPreference::System => TypographyAssetStatus::SystemFallback,
            FontPreference::Inter => TypographyAssetStatus::BundledPreferred,
        },
        emoji_asset: match settings.emoji {
            EmojiPreference::System => TypographyAssetStatus::SystemFallback,
            EmojiPreference::TwemojiColr => TypographyAssetStatus::BundledPreferred,
        },
    }
}
```

Export the module and public types from `crates/matrix-desktop-state/src/lib.rs`.

- [x] **Step 4: Run the focused test to verify GREEN**

Run:

```bash
cargo test -p matrix-desktop-state --test typography_display_profile
```

Expected: PASS.

### Task 2: Snapshot DTO And TypeScript Contract

**Files:**
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`

- [x] **Step 1: Add failing DTO assertions**

In `apps/desktop/src-tauri/src/dto.rs`, update `frontend_snapshot_serializes_to_the_typescript_contract()` to assert default `typography_profile`, and add a focused test that changes `state.settings.values.typography` to Inter/Twemoji and expects bundled-preferred profile fields.

- [x] **Step 2: Run the focused DTO test to verify RED**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_snapshot_typography
```

Expected: FAIL because the snapshot does not include `typography_profile`.

- [x] **Step 3: Serialize the Rust-owned typography profile**

Import `TypographyDisplayProfile` and `resolve_typography_display_profile` into `dto.rs`, add `pub typography_profile: TypographyDisplayProfile` to `FrontendAppState`, and resolve it from `state.settings.values.typography` with `frontend_display_platform()`.

- [x] **Step 4: Mirror the contract in TypeScript fakes**

Add `typography_profile` to `AppState` in `types.ts`. Add helper/default profiles in `browserFakeApi.ts`, `appHarnessMain.tsx`, and `tauriIpcMock.ts`. When `updateSettings()` or the harness `update_settings` command receives a typography patch, recompute `typography_profile` from the patched settings.

- [x] **Step 5: Run focused Rust and TypeScript verification**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_snapshot_typography
npm --prefix apps/desktop run typecheck
```

Expected: both PASS.

### Task 3: Documentation And Phase A Gate

**Files:**
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/plans/2026-06-15-font-emoji-phase-a.md`

- [x] **Step 1: Document the policy**

Record that typography profile resolution is Rust-owned, carries only non-secret preference/profile data, and that bundled Inter/Twemoji assets are Phase B with system fallbacks.

- [x] **Step 2: Run Phase A gates**

Run:

```bash
cargo test -p matrix-desktop-state --test typography_display_profile
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_snapshot_typography
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run qa:secret-scan
git diff --check
```

Expected: all PASS.

- [ ] **Step 3: Commit, merge, and close #5 only after all acceptance checks are satisfied**

Commit the Phase A and Phase B work separately if Phase B follows immediately. Close #5 only after all issue acceptance criteria are met; otherwise leave #5 open with a Phase A completion comment and keep Phase B next.
