# Phase 15 Desktop Interactions Implementation Plan

Date: 2026-06-13
Status: in progress.

Canon/design:

- [../../architecture/overview.md](../../architecture/overview.md)
- [../../policies/engineering-rules.md](../../policies/engineering-rules.md)
- [../specs/2026-06-13-phase-15-desktop-interactions-design.md](../specs/2026-06-13-phase-15-desktop-interactions-design.md)

Execution model: implementation is delegated to mini subagents; the main agent
performs canon edits, review, and verification. Follow TDD: add or extend a
failing test before each production change.

## Task A — State/Core Notification Surface

Files to inspect first:

- `crates/matrix-desktop-state/src/lib.rs`
- `crates/matrix-desktop-state/tests/navigation_state.rs`
- `crates/matrix-desktop-sdk/src/lib.rs`
- `crates/matrix-desktop-sdk/tests/password_login.rs`
- `apps/desktop/src-tauri/src/commands.rs`

Steps:

1. Add unread notification/highlight metadata to the room summary snapshot, or
   an equivalent serializable DTO that preserves the canon payload limits.
2. Map SDK unread counts (`notification_count`, `highlight_count`, and existing
   unread message fallback) into that DTO.
3. Keep `matrix-desktop-state` pure and wasm-clean.
4. Add reducer/serialization tests that prove mention/DM/message precedence can
   be derived without message bodies or raw IDs in the attention payload.

Verification:

- `cargo test -p matrix-desktop-state`
- `cargo test -p matrix-desktop-sdk`
- `cargo check --target wasm32-unknown-unknown -p matrix-desktop-state`

## Task B — Frontend Attention Adapter

Files to inspect first:

- `apps/desktop/src/App.tsx`
- `apps/desktop/src/domain/qaTitle.ts`
- `apps/desktop/src/test/tauriIpcMock.ts`
- `apps/desktop/src-tauri/capabilities/default.json`

Steps:

1. Create a pure domain helper for desktop attention decisions:
   unread total, badge count, title hint, notification candidate, and QA title
   token.
2. Wire the helper into React state changes.
3. Use Tauri window APIs for badge count/title when available; browser tests
   must keep using mockable fallbacks.
4. Add capability permissions for any newly used Tauri window API.
5. Keep OS notification content redacted per canon.

Verification:

- `npm --prefix apps/desktop run typecheck`
- `npm --prefix apps/desktop run test`
- targeted frontend tests for the helper and App wiring

## Task C — OS Notification And Linux QA Assertion

Files to inspect first:

- `apps/desktop/package.json`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/tauri.conf.json`
- `scripts/desktop-linux-gui-qa.mjs`
- `docker/linux-gui.Dockerfile`

Steps:

1. Prefer the official Tauri notification plugin if the installed Tauri v2
   stack supports it cleanly; otherwise document why and use request-attention
   plus QA token as the temporary fallback.
2. Add a mockable notification adapter and tests proving no body/sender/raw ID
   reaches the notification call.
3. Extend the Linux GUI runner to assert a notification/badge/title QA token.
   Attempt a DBus notification assertion under Xvfb; if it is not reliable,
   record the gap in the roadmap and keep the deterministic QA token assertion.
4. Save any install/setup changes in `AGENTS.md` or the Docker recipe.

Verification:

- `npm --prefix apps/desktop run test:ui-headless`
- `npm --prefix apps/desktop run qa:linux-gui -- --artifact-dir=...`
- Docker recipe build/run if dependencies changed

## Task D — Window State Contract

Files to inspect first:

- `apps/desktop/src-tauri/src/lib.rs`
- `scripts/desktop-linux-gui-qa.mjs`

Steps:

1. Reuse the existing Tauri window-state persistence code.
2. Add or expose a Linux-lane assertion that the state path and persisted JSON
   contract remain valid.
3. Avoid adding product multi-window behavior.

Verification:

- `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`
- Linux GUI lane output includes a window-state token or the Tauri test evidence
  is referenced in the phase exit.

## Task E — Accessibility And Shortcut Evidence

Files to inspect first:

- `apps/desktop/src/App.tsx`
- `apps/desktop/e2e`
- `docs/qa/shortcut-parity.md`

Steps:

1. Ensure the three panes expose stable landmarks or labelled regions.
2. Add a Playwright keyboard-only walkthrough/focus-order spec.
3. Complete the shortcut parity table and document deviations.

Verification:

- `npm --prefix apps/desktop run test:ui-headless`
- `npm --prefix apps/desktop run test`

## Phase Exit Review

Reviewer-run gates:

```bash
cargo test -p matrix-desktop-core --lib
cargo test -p matrix-desktop-sdk -p matrix-desktop-state -p matrix-desktop-search -p matrix-desktop-key
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo check --target wasm32-unknown-unknown -p matrix-desktop-state -p matrix-desktop-search
npm --prefix apps/desktop run typecheck && npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run test:ipc-contract
npm --prefix apps/desktop run qa:secret-scan
node scripts/desktop-release-gate-check.mjs --no-compile
docker build -f docker/linux-gui.Dockerfile -t matrix-desktop-linux-gui:phase15 .
npm --prefix apps/desktop run qa:linux-gui -- --artifact-dir=/work/artifacts/linux-gui-phase15 --timeout-ms=180000
```

Update
[2026-06-13-roadmap-phases-10-18.md](2026-06-13-roadmap-phases-10-18.md)
only after the reviewer has evidence for each Phase 15 checkbox.
