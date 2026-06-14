# Font And Emoji Substrate Design

## Goal

Implement cross-platform UI font and emoji behavior for issue #5 without
copying Element assets and without adding OS-specific product branches.

## Current State

Rust already owns non-secret typography settings:

- `TypographySettings { font, emoji }`
- `FontPreference::{System, Inter}`
- `EmojiPreference::{System, TwemojiColr}`

The TypeScript snapshot contract already mirrors those wire values as
`font: "system" | "inter"` and `emoji: "system" | "twemojiColr"`. React must
consume those values; it must not invent local font or emoji preferences.

## Asset And License Decisions

- Inter is the selected UI font when `font = Inter`. Use `@fontsource/inter`
  or locally vendored files derived from that package. The package reports
  `OFL-1.1`, and the upstream Inter project publishes Inter under the SIL Open
  Font License 1.1.
- Twemoji COLR is the selected emoji font when `emoji = TwemojiColr`. Use the
  `twemoji-colr-font` package or locally vendored files derived from that
  package. The npm package reports `OFL-1.1`.
- Do not copy Element, FluffyChat, Compound, or product-brand font/icon assets.
- When font assets are added to the repository, update
  `THIRD_PARTY_NOTICES.md` with project name, package/repository, version or
  commit, local path, license, and notes. Reference-only reading still does not
  require a notice entry.

## Architecture

Phase A extends the Rust/headless contract only where needed:

- Add a Rust-owned typography display profile derived from
  `SettingsValues.typography`.
- The profile exposes stable names the UI can place on `documentElement`
  datasets and CSS custom properties, such as `font_family`, `emoji_family`,
  and asset status. It contains no account identifiers, message content,
  file names, homeserver URLs, device IDs, raw errors, or secrets.
- Architecture and policy docs record fallback semantics:
  `System` uses platform/browser system UI and emoji fonts; `Inter` and
  `TwemojiColr` request bundled assets with system fallbacks.

Phase B consumes the contract:

- Load Inter and Twemoji COLR through application-level CSS and tokens, not
  per-component branches.
- `App.tsx` maps the Rust-owned typography profile or settings to root
  attributes. Components inherit via CSS.
- Settings UI may expose typography choices, but it must dispatch
  `update_settings` patches only. The reducer remains the source of truth.

## CSS Model

Use root-level variables:

- `--font-ui`: UI text stack.
- `--font-emoji`: emoji stack.
- `--font-message`: message text stack, usually `var(--font-ui)`.

The default stack remains platform-generic:

```css
system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif
```

When bundled Inter is selected, prepend the bundled Inter family. When bundled
Twemoji COLR is selected, prepend the Twemoji family to emoji-rendering
contexts and keep system emoji fonts as fallback.

## Verification

Phase A verification:

- Rust tests prove the typography profile defaults to system settings, maps
  `Inter` and `TwemojiColr`, serializes through the Tauri DTO, and contains no
  private data.
- Docs and `AGENTS.md` record asset policy and fallback rules.

Phase B verification:

- Browser-headless tests set Rust-owned typography settings in the harness
  snapshot and assert root attributes/CSS variables update from those values.
- `document.fonts.check()` proves bundled Inter and Twemoji COLR are loaded
  when selected.
- Browser-headless samples include multilingual text and emoji/reaction/SAS
  emoji glyphs on light and dark themes.
- `THIRD_PARTY_NOTICES.md` contains the font asset provenance.

## Non-Goals

- No custom emoji picker implementation in #5; composer emoji picker remains
  part of later composer/reaction work.
- No native GUI smoke is required for #5. Browser-headless font loading is the
  primary gate; native lanes remain final compatibility checks.
- No per-OS component branches. Platform differences are represented as data
  or CSS fallbacks only.

## Source Checks

- `npm view @fontsource/inter version license dist.tarball --json` reported
  version `5.2.8` and license `OFL-1.1`.
- `npm view twemoji-colr-font version license dist.tarball --json` reported
  version `15.0.3` and license `OFL-1.1`.
- Upstream Inter and SIL OFL references were checked for the font license.
- The historical Mozilla `twemoji-colr` project notes that COLR/CPAL color
  font support depends on the rendering engine. The app keeps system emoji
  fallback after bundled Twemoji.
