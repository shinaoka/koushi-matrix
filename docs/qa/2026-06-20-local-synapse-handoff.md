# 2026-06-20 Local Synapse Handoff

## Current Commit

- `f70d0e5 qa: fix local Synapse timeline and DM coverage`
- Parent repository worktree after the commit is clean except for the existing dirty
  submodule state under `vendor/matrix-rust-sdk`.
- Submodule dirty file observed at handoff:
  `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/redecryptor.rs`

## What Was Fixed

- Local Synapse reproduction and QA coverage now exercise the real failure areas:
  timeline loading, search crawler backfill, send queue retry/cancel/restore,
  room/space member lists, DM creation from member rows, read receipts, media,
  link previews, scheduled send, E2EE trust, and restore cleanup.
- Room and Space info panels expose all loaded members and provide a `New DM`
  action for each member row.
- The room-list user-facing label is `DMs`/`DM`, not `People`; internal DOM
  section id remains `people` for compatibility with existing tests.
- Local Synapse config now allows public directory publication so directory
  query/join QA can run against Synapse instead of silently depending on a
  permissive server.
- `send_queue` QA no longer uses a raw TCP copier. The test proxy is HTTP-aware,
  forces `Connection: close`, strips proxy keep-alive headers, and preserves
  request bodies correctly.
- E2EE verification acceptance waits now drive bounded `SyncOnce` loops instead
  of assuming one sync delivers the to-device verification transition.
- Headless UI tests were updated for current product behavior:
  username localpart login field, settings hierarchy, right panel closed by
  default, room-list projection filtering, dialog-based room key import/export,
  and Rust-owned section state updates.

## Fresh Verification Evidence

Commands run successfully after the final changes:

```bash
npm --prefix apps/desktop run typecheck
cargo fmt --check
cargo test -p koushi-core --features qa-bin --bin headless-core-qa send_queue_proxy_forces_connection_close_per_request -- --nocapture
cd apps/desktop && npx playwright test
npm --prefix apps/desktop run qa:headless-local -- --run --server=synapse --scenario=all --core --core-backend=probed --timeout-ms=240000
KOUSHI_QA_STRESS_SPACES=5 KOUSHI_QA_STRESS_ROOMS_PER_SPACE=8 KOUSHI_QA_STRESS_MESSAGES_PER_ROOM=30 npm --prefix apps/desktop run qa:headless-local -- --run --server=synapse --scenario=timeline_stress --core --core-backend=probed --timeout-ms=180000
```

Important observed tokens:

```text
146 passed
server=synapse
sync_backend_a=SyncService
member_list=ok
dm_start=ok
timeline=ok
search=ok
crawl_backfill=ok
crawl_no_media_bytes=ok
crawl_throttle=ok
send_fail=ok
resend=ok
fifo=ok
cancel_send=ok
unsent_restart=ok
e2ee_trust=ok
restore_cleanup=ok
stress_counts=spaces=5 rooms=40 messages=1201
stress_space_scope=ok
stress_no_blank=ok
timeline_stress=ok
```

## Root Causes Captured

- Local Synapse directory QA failed because Synapse denied room publication by
  default. The local QA server config now explicitly permits publication.
- `send_queue` login failed behind the QA network proxy because a raw TCP proxy
  and later an HTTP half-close caused Synapse/Twisted to see a disconnected
  client. The HTTP-aware proxy avoids half-close and lets `Connection: close`
  terminate the response.
- A later proxy rewrite bug inserted `Connection: close` into the JSON request
  body; the regression test now verifies the header/body split.
- Full `scenario=all` then exposed a verification timing issue: A2 did not
  always observe acceptance after one manual sync. The wait now repeatedly
  drives bounded sync until the verification state is projected.
- Several Playwright failures were stale test assumptions, not product
  regressions: active-space room filtering, settings category hierarchy,
  dialog-based key import/export, and a closed-by-default right panel.

## Long-Term Design Note

Do not stop at the current backpressure/concurrency fix forever. Long-term,
LegacySync should also receive active-room intent from navigation and rebuild
its sync filter around that active room. The intended design is:

- Navigation state changes emit an effect carrying the active room id to
  `SyncActor`.
- `SyncActor` owns active-room sync priority for both backends.
- SyncService continues using its native room-list/timeline behavior where
  available.
- LegacySync rebuilds or refreshes its `/sync` filter so the selected room gets
  a larger or fresher timeline window than background rooms.
- The search crawler stays lower priority than active timeline pagination and
  should never starve active-room message loading.
- Add forced-Legacy local QA proving that active room changes alter the sync
  filter/behavior and that selected-room messages continue loading while the
  crawler is busy.

This is intentionally a future architecture change, not part of `f70d0e5`.

## Recommended Next Steps

- Run the real `matrix.org` account smoke separately before dogfooding a new DMG.
  Keep sends limited to the approved test recipient only.
- Decide whether to commit or discard the dirty submodule change in
  `vendor/matrix-rust-sdk`; it was not included in `f70d0e5`.
- If continuing with Legacy active-room filtering, first add a failing
  forced-Legacy local QA case before changing `SyncActor`.
