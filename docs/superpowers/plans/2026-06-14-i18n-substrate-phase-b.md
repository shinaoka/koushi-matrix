# 2026-06-14 i18n Substrate Phase B

Goal: wire the Rust-owned locale/display profile into the GUI without moving
locale decisions into React.

Canon consulted:

- `REPOSITORY_RULES.md`
- `docs/architecture/overview.md`
- `docs/architecture/i18n.md`
- `docs/policies/engineering-rules.md`
- `docs/superpowers/plans/2026-06-14-i18n-substrate-phase-a.md`

## Plan

1. Extend the Tauri snapshot DTO, TypeScript domain types, browser fake, IPC
   mocks, and app harness snapshots with `LocaleDisplayProfile`.
2. Apply the resolved profile at the DOM root and activate the catalog from
   `snapshot.state.locale_profile`.
3. Move visible React component prose behind catalog IDs while keeping remote
   room/user/message text as caller-owned data with `dir="auto"`.
4. Convert locale-sensitive shell CSS from physical left/right assumptions to
   logical properties.
5. Add headless browser checks for root `lang`/`dir`, pseudo RTL/CJK/combining
   samples, raw-string coverage, logical CSS, and DTO/profile serialization.

## Phase Boundary

Phase B owns GUI wiring and headless DOM proof only. Locale resolution,
direction fallback, pseudo-locale mode selection, and platform shortcut label
selection remain in Rust and were completed in Phase A.

## Implementation Notes

- React feature components consume `snapshot.state.locale_profile`; they do
  not parse persisted locale tags.
- Browser fakes and harnesses may mirror Rust resolution only to keep tests
  deterministic.
- In the full App Playwright harness, timeline rows are event-driven. Locale
  snapshot tests that also need a custom row should update the seeded row with
  `ItemsUpdated.Set` rather than relying on a second one-shot `InitialItems`
  during a snapshot refresh.

## Verification

Run at minimum:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml dto::tests
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts src/styles.contract.test.ts src/backend/browserFakeApi.test.ts src/components/KeyboardSettingsPanel.test.tsx src/components/UserSettingsPanel.test.tsx src/components/RoomInfoPanel.test.tsx src/components/SpaceInfoPanel.test.tsx src/App.test.tsx
npm --prefix apps/desktop run test:ui-headless -- --grep "Rust-owned locale profile applies root lang and dir|pseudo RTL profile"
```

Before merge, also run formatting, `qa:wasm-check`, secret scan, and
`git diff --check`.
