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
- A must receive every ACL update as a hidden item. Lag on the reader's event
  stream is not skipped: the item waiter re-observes the Core-held timeline
  through a fresh subscription, and the count of received ACL updates with any
  visible one is a failure.
- The hidden updates must leave the room list alone: A asserts that the room
  summary still shows the message as `latest_event` with `unread_count >= 1`.
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

This gap is tracked as #1125. It is not automated yet: the `hidden_state_acl`
scenario keeps the live-actor replay path green, and the scenario's module doc
states that the fresh-subscription path is not covered. The RED stage would be
run by repeating the burst with a `Unsubscribe` before the final `Subscribe`
and asserting the message is still visible, which currently fails.

## Verification

- `npm --prefix apps/desktop run qa:headless-local -- --run --server=tuwunel --scenario=hidden_state_acl --core --timeout-ms=240000`
  printed `hidden_state_acl=ok` against a local Tuwunel 1.7.1, including the
  room-list assertions and the post-burst follow-up message.
- The registry unit tests, clippy (`koushi-qa`, `qa-bin`), `cargo fmt --check`,
  and the docs and structure checkers passed.
