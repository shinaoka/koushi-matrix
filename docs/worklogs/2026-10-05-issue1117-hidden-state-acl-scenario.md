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
- A must receive every ACL update as a hidden item.
- A then subscribes again. The Core-held timeline is replayed through the
  live-edge window. The replay held 136 raw items in the passing run, and it
  must still contain the visible message.
- A reports a viewport showing only the message. Navigation must converge to
  `Synced`, with the message as the server-confirmed read boundary, and the
  room's unread count must reach zero.

The scenario is dispatched separately (Safety + `HiddenStateAcl`) and is left
out of `all`. Its success token is `hidden_state_acl=ok`.

## Finding: fresh subscription after unsubscribe (not fixed here)

A first draft did `Unsubscribe` and then `Subscribe`, which builds a fresh SDK
timeline. That replay held 120 items, and every one was a hidden ACL update.
The message was absent.

The cause is the `should_hydrate_empty_initial_room_timeline` hydration. It
runs once with `INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT` (100) events. When
more hidden events than that follow the message, the fresh snapshot still has
no displayed row.

Whether the renderer's viewport-fill pagination recovers the message in the GUI
was not checked. The scenario covers the Core-held replay path. The fresh-actor
path is reported as a follow-up candidate.

## Verification

- `npm --prefix apps/desktop run qa:headless-local -- --run --server=tuwunel --scenario=hidden_state_acl --core --timeout-ms=240000`
  printed `hidden_state_acl=ok` against a local Tuwunel 1.7.1.
- The registry unit tests, clippy (`koushi-qa`, `qa-bin`), `cargo fmt --check`,
  and the docs and structure checkers passed.
