# Hidden-state ACL headless scenario (#1117)

## Scope

#1110 promised a disposable-homeserver scenario with one ordinary message
followed by repeated `m.room.server_acl` updates. The fix landed with unit
probes only. This change adds the headless scenario `hidden_state_acl`. Runtime
behavior is unchanged.

## Scenario

- B creates an unencrypted room, invites A, and sends one message.
- A disposable second SDK device of B (the room creator) writes 125 distinct
  ACL updates. Each update denies a different `example.invalid` host, because
  homeservers may drop a state write that changes nothing.
- A must receive every observed ACL update as a hidden item, and the scenario
  fails if any received one is exported visibly. Lag on the reader's event
  stream is not skipped: the item waiter re-observes the Core-held timeline
  through a fresh subscription under the same absolute deadline, feeding items
  that arrive while that snapshot is awaited to the same observer.
- The hidden updates must leave the room list alone: A requires an explicit
  `RoomListUpdated` publication newer than the burst (a general state delta does
  not count, and the item wait counts publications instead of discarding them),
  then asserts the summary holds `latest_event` = the message with
  `unread_count >= 1`.
- A then subscribes again. The Core-held timeline is replayed through the
  live-edge window. The replay held 136 raw items in the passing run, and it
  must still contain the visible message.
- A second ordinary message after the burst must appear in both the timeline
  and the room list.
- A reports a viewport showing only the newest message. Navigation must
  converge to `Synced`, with that message as the server-confirmed read
  boundary, and the room's unread count must reach zero.

The scenario is dispatched separately (Safety + `HiddenStateAcl`) and is left
out of `all`. Its success token is `hidden_state_acl=ok`.

## Finding: fresh subscription after unsubscribe (tracked, not fixed here)

A first draft did `Unsubscribe` and then `Subscribe`, which builds a fresh SDK
timeline. That replay held 120 items, and every one was a hidden ACL update.
The message was absent.

The cause is the `should_hydrate_empty_initial_room_timeline` hydration. It
runs once with `INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT` (100) events. When
more hidden events than that follow the message, the fresh snapshot still has
no displayed row.

Whether the renderer's viewport-fill pagination recovers the message in the GUI
was not checked. The scenario covers the Core-held replay path only.

This gap is tracked as #1125 and is kept as an executable RED stage:
`crates/koushi-core/src/timeline/actor/fresh_room_hydration_tests.rs` builds a
fresh room subscription whose live-edge window is entirely hidden state events
and is `#[ignore]`d because it fails today. Run it with:

```bash
cargo test -p koushi-core --lib -- --ignored fresh_room_subscription
```

Observed RED output: `items=120, hidden=120, window=120` — the fresh
subscription's initial window holds no displayed row, so the visible message is
unreachable. The `hidden_state_acl` scenario keeps covering the live-actor
replay path only, and its module doc says so.

## Verification

- `npm --prefix apps/desktop run qa:headless-local -- --run --server=tuwunel --scenario=hidden_state_acl --core --timeout-ms=240000`
  printed `hidden_state_acl=ok` against a local Tuwunel 1.7.1, including the
  room-list assertions and the post-burst follow-up message.
- The registry unit tests, clippy (`koushi-qa`, `qa-bin`), `cargo fmt --check`,
  and the docs and structure checkers passed.
