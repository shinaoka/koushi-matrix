# Post Headless Core Follow-ups

Date: 2026-06-13
Status: proposed follow-up spec.

This spec captures work intentionally deferred when
`2026-06-12-headless-core-runtime-implementation.md` was marked completed.
The Phase 8 real-homeserver gate is green; these items are cleanup or
product-surface follow-ups, not blockers for the completed headless runtime
plan.

## SDK Adapter Rename

Rename `matrix-desktop-auth` to `matrix-desktop-sdk` in a dedicated change.

Scope:

- crate directory and package name
- workspace membership and Cargo dependencies
- Tauri lockfile updates
- imports in `matrix-desktop-core`, `matrix-desktop-backend`, and Tauri
- docs that describe crate responsibilities

Constraint: no behavior changes in the rename patch.

## Runtime IPC Type Generation

Replace the handwritten TypeScript CoreEvent/AppState wire-contract mirror
with generated or mechanically checked types.

Requirements:

- preserve the Rust contract test that pins serialized wire shapes
- keep the frontend import surface stable during migration
- fail locally if Rust event/snapshot payloads and TypeScript types diverge
- keep secret-bearing payloads redacted in debug/log paths

## Room Lifecycle Commands

Add explicit room cleanup commands once product behavior is defined:

- `LeaveRoom`
- `ForgetRoom` or equivalent post-leave cleanup
- real-homeserver QA cleanup should use these commands instead of reporting
  `leave_room=not_available`

Constraints:

- no silent cleanup failures in real-account QA
- failures must be classified, not raw SDK error text
- only synthetic QA rooms may be cleaned up by automation

## Optional App Commands

Decide whether these transport/UI shims need first-class core commands:

- dedicated login discovery command before password submit
- thread open/close navigation commands

If they remain frontend-local UI state, document that explicitly and remove
the shim commands later.
