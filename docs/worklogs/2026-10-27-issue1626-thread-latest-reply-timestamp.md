# #1626 — thread summary dated the latest reply at the root

## Symptom

For a session whose first sync already contained a thread's replies, the chip
under a thread root read `Latest reply <time>` with the **root's** time. A
session that received the replies live showed the right time. The thread pane
and the root row's position were correct: only the printed reply time was wrong.

## Cause (verified against the pinned SDK)

`TimelineEvent::bundled_latest_thread_event`
(`vendor/matrix-rust-sdk/crates/matrix-sdk-common/src/deserialized_responses.rs`)
derived the server-bundled latest reply with the **root's** `origin_server_ts`
as the anti-future clamp ceiling. A reply is always posted after its root, so
the derived copy always carried the root's time. The room event cache persists
that derived event
(`crates/matrix-sdk/src/event_cache/caches/room/state.rs`,
`post_process_upserted_events`), replacing the correctly dated copy of the same
reply. Every later read then carried the wrong value: the thread relation
aggregate (`matrix-sdk-ui/src/timeline/thread_list_service.rs`) and
`ThreadSummaryDto.latest_timestamp_ms`, which `crates/koushi-core/src/threads_list.rs`
copies from it into the chip.

The overwrite only reproduces when the bundled summary arrives **after** the
reply is already cached; a single sync batch that carries both writes the
reply's own copy last, which is why the live-receive case was correct.

## Fix (SDK fork)

One expression: the clamp ceiling is now `MilliSecondsSinceUnixEpoch::now()` —
the same guard `bundled_replacement` already uses. A forged future timestamp is
still capped; a real reply keeps its own time. Fork commit `3fec6d3bc`
(`shinaoka/matrix-rust-sdk-work` PR #30), recorded in
`docs/upstream/matrix-rust-sdk-feedback.md` with upstream intent and the
gitlink bumped to it.

## Verification

- RED before the fix, GREEN after, in the owning crate:
  `cargo test -p matrix-sdk-common --lib` (165 → 166 passed). The new
  `test_bundled_latest_thread_event_keeps_the_reply_timestamp` failed with
  `left: 1000, right: 2000` (root time vs reply time);
  `test_bundled_latest_thread_event_still_caps_a_future_timestamp` keeps the
  forgery guard honest.
- Workspace regression test through the production path (mock sync → room event
  cache → persisted copy → thread relation aggregate):
  `crates/koushi-core-testkit/tests/thread_bundled_reply_timestamp.rs`
  (`cargo test -p koushi-core-testkit --test thread_bundled_reply_timestamp`),
  RED against the previous gitlink and GREEN against `3fec6d3bc`. It lives in the
  testkit because `crates/koushi-core/tests` is a closed set enforced by
  `scripts/check-leaf-crate-boundaries.mjs`; that target list is updated in the
  same change.
- `node scripts/check-sdk-submodule.mjs` green; `cargo fmt --check` clean.

## Limits

No local-server or GUI run was needed: the value is produced before any
rendering and is covered at the layer that produced it. The chip's rendering of
`latest_timestamp_ms` is unchanged and already covered by the existing thread
summary tests.
