# Live Signals Phase A: Rust-Owned Receipts, Markers, Typing, Presence

Date: 2026-06-15
Status: active Phase A plan for issue #16.

## Goal

Implement read receipts, fully-read markers, typing notifications, and presence
as Rust-owned state-machine data before any GUI affordance owns the behavior.

## Scope

- `koushi-state` owns `AppState.live_signals` and reducer actions for
  full room signal replacement, partial receipt merge, fully-read marker update,
  typing user replacement, and presence update.
- `koushi-core` owns typed `CoreCommand` and `CoreEvent` surfaces for
  read receipt send, fully-read marker send, typing notice, and presence set.
- `TimelineActor` relays SDK receipt, fully-read, and typing signals into
  reducer actions. `AccountActor` owns the Phase A presence command/event/state
  projection.
- Tauri and TypeScript contracts are DTO clients only: snapshots and event
  contracts must be updated from Rust-owned types, with browser fakes and IPC
  mocks kept in sync.

## Non-Goals

- React-owned receipt, marker, typing, or presence state.
- GUI controls beyond the DTO/contract surface.
- Real-account presence compatibility claims.
- Full network presence propagation until the sync-backend API decision is
  recorded. The legacy SDK path exposes `SyncSettings::set_presence`; the
  current vendored `SyncService` builder does not expose a direct setter.

## Phase A Exit Gates

```bash
cargo test -p koushi-state live_signal -- --nocapture
cargo test -p koushi-core live_signal -- --nocapture
cargo test -p koushi-core --features qa-bin --bin headless-core-qa -- --nocapture
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run qa:secret-scan
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=both --scenario=live_signals --core --core-backend=both --timeout-ms=240000
```

Expected local QA tokens:

```text
read_receipt=ok
fully_read=ok
typing=ok
presence=ok
live_signals=ok
```

The QA output must stay private-data-free: no Matrix room IDs, event IDs, user
IDs, message bodies, raw SDK errors, credentials, or local paths.

For local SyncService homeserver legs, the typing assertion may perform one
bounded debug/test `SyncOnce` on the observer account after `SetTyping` is
acknowledged. The diagnostic evidence is that legacy sync receives the typing
notification continuously, while the local SyncService path can acknowledge the
sender command without waking the SDK room typing observer. This is QA delivery
control only; product sync policy remains in Rust actors and must not be
implemented in React.

## Phase B Handoff

GUI work renders `snapshot.state.live_signals` and dispatches typed Tauri
commands. It must not infer live-signal lifecycle locally. Headless browser
tests should drive real controls and assert command names/shapes plus returned
Rust snapshots. The Linux virtual-display lane is the real WebView behavior
check once controls exist.
