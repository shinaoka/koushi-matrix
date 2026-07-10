# Diagnostics stderr-elimination design

- Date: 2026-07-10
- Status: Approved for implementation
- Scope: `matrix-desktop` application runtime only

## Goal

Make the structured Diagnostics collector the only application-owned debug
diagnostic output. The desktop app must not write diagnostic mirrors to stderr
under any `KOUSHI_*TRACE` or `KOUSHI_DEBUG_SDK_ERROR` setting.

## Decision

Remove the optional `eprintln!` branches and their trace-enable helpers from
the Rust/Tauri runtime. Their structured collector calls remain unconditional,
bounded, and private-data-free. Remove the corresponding environment-variable
exports from application and QA launch paths when they are used only for those
mirrors.

Record a schema-version mismatch from React as one sanitized frontend
Diagnostics entry (`source=snapshot`, fixed mismatch message) before showing
the existing recovery UI. Do not retain the received schema version or any IPC
payload in the entry.

## Alternatives considered

1. Keep optional stderr mirrors: rejected because it leaves a second debug
   output path and environment-controlled behavior.
2. Redirect stderr into Diagnostics: rejected because it could capture raw
   dependency, SDK, or operating-system output outside the privacy contract.
3. Remove mirrors and retain the structured collector: selected because the
   collector already holds the needed signals with a bounded, reviewed schema.

## Included runtime families

- startup, subscribe, timeline, event-cache, unread, search, sync, account,
  room-actor, and Tauri command diagnostics;
- the debug/test credential-store notification;
- the frontend schema-contract mismatch console message.

## Exclusions

- CLI/CI/QA script success and failure output;
- test-only `println!` output;
- dependency, operating-system, and browser-runtime output not issued by
  application diagnostic helpers.

## Verification

- Add tests that inspect production source partitions and reject every listed
  diagnostic stderr mirror or environment gate.
- Add a frontend test proving a mismatched snapshot creates a sanitized
  Diagnostics entry without relying on `console.error`.
- Run focused Rust and frontend suites, then the full desktop test, typecheck,
  lint, release-gate, formatting, diff, and secret checks.
