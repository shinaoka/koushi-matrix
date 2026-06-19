# Post Headless Core Follow-ups

Date: 2026-06-13
Status: completed cleanup spec.

This spec captures work intentionally deferred when
`2026-06-12-headless-core-runtime-implementation.md` was marked completed.
The Phase 8 real-homeserver gate is green; these items were cleanup or
product-surface follow-ups, not blockers for the completed headless runtime
plan. They were completed before Phase 10 GUI design work.

## SDK Adapter Rename

Completed: `matrix-desktop-auth` was renamed to `koushi-sdk`.

Scope:

- crate directory and package name
- workspace membership and Cargo dependencies
- Tauri lockfile updates
- imports in `koushi-core`, `koushi-backend`, and Tauri
- docs that describe crate responsibilities

Constraint satisfied: no behavior changes in the rename patch.

## Runtime IPC Type Generation

Completed: the handwritten TypeScript CoreEvent/AppState wire-contract mirror
is mechanically checked by
`apps/desktop/src/domain/coreEvents.generated.json` and the Rust
`core_event_wire_format_matches_checked_in_contract_artifact` test.

Requirements:

- the Rust contract test pins serialized wire shapes
- the frontend import surface remains `coreEvents.ts`
- local verification fails if representative Rust payloads drift from the
  checked-in JSON artifact
- secret-bearing payloads remain absent from CoreEvent and redacted in
  debug/log paths

## Room Lifecycle Commands

Completed: explicit room cleanup commands exist:

- `LeaveRoom`
- `ForgetRoom`
- real-homeserver QA cleanup uses these commands and reports
  `leave_room=ok forget_room=ok`

Constraints:

- no silent cleanup failures in real-account QA
- failures are classified, not raw SDK error text
- only synthetic QA rooms are cleaned up by automation

## Optional App Commands

Decision: these transport/UI shims do not become first-class core commands yet.

- `discover_login_methods` remains a compatibility shim. `LoginPassword`
  performs discovery inside core and the frontend reads homeserver text from
  local form state.
- `open_thread` / `close_thread` remain frontend navigation shims. A later
  product design may add native cross-window thread commands, but the current
  core runtime does not need them.

The shim commands are documented in `apps/desktop/src-tauri/src/commands.rs`.
