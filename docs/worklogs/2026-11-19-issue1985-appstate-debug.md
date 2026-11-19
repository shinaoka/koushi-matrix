# #1985: the `AppState` diagnostic dump renders shape, not identities

## What was open

#1979 (worklog: [issue1972-1979](2026-11-19-issue1972-1979-boundary-debug-redaction.md))
redacted the `koushi-protocol` `Debug` impls and the DTOs they print, and left one
class open: types whose `Debug` dumps a large part of `AppState`, plus the
`koushi-state` types only reachable through them.

- `VersionedAppStateSnapshot` derived `Debug` over the whole `AppState`, so
  `format!("{snapshot:?}")` printed account, room and event ids, message bodies,
  drafts and previews.
- `StateDeltaChangedSlices` embedded `SessionState` -> `SessionInfo`, whose
  `Debug` printed `homeserver`, `user_id` and `device_id` raw, beside 78 id-keyed
  collection fields.
- `ThreadsListItem` printed its two events, two senders and two body previews when
  formatted directly.

## Canon consulted

- `docs/policies/engineering-rules.md`, "Logging and Diagnostics" rule 3: a
  boundary `Debug` renders `RoomId(..)` / `EventId(..)` / `UserId(..)`
  placeholders and never a body preview.
- `REPOSITORY_RULES.md`, "Working rules": a security-adjacent contract change
  leaves a worklog naming the canon, the files and the verification.
- #1979's deferral note, which required retargeting the assertions that prove
  secret absence *through* the dump's verbosity before making the dump opaque.

## What changed

| File | Change |
| --- | --- |
| `crates/koushi-state/src/state/mod.rs` | `AppState` loses `derive(Debug)`; a hand-written impl renders the session, the sub-states whose own `Debug` is already a summary, and a count for every identity-keyed collection. |
| `crates/koushi-state/src/state/session.rs` | `SessionInfo` redacts homeserver/user/device behind `Homeserver(..)` / `UserId(..)` / `DeviceId(..)`; `SessionState` is hand-written so `Authenticating { homeserver }` cannot print the homeserver the reader typed. |
| `crates/koushi-state/src/state/sync.rs` | `SyncState`'s `Failed`/`Reconnecting` reason is transport text; it renders `Reason(..)`. |
| `crates/koushi-state/src/state/timeline.rs` | `ComposerState` renders a draft length instead of the draft, and the reply mode / pending send kind as their variant name instead of the event they reply to; `TimelinePaneState` renders its room as `RoomId(..)` with counts for scheduled sends, staged uploads, media and thumbnails. |
| `crates/koushi-state/src/state/thread.rs` | `ThreadPaneState` renders both ids as placeholders and the open intent as its variant name (`PinnedReply` carries an event id), and `ThreadsListItem` renders its events, senders, labels and previews as placeholders. |
| `crates/koushi-state/src/state/basic_operation.rs`, `files_view.rs` | Both were already hand-written but printed a user-typed room name, two room ids and a free-form failure message; they now render `RoomName(..)` / `RoomId(..)` / `has_message`, and `FilesViewScope` redacts the room it names. |
| `crates/koushi-protocol/src/state_update.rs` | `StateDeltaChangedSlices` names the changed slices and nothing else. An exhaustive `let Self { .. } = self` makes a new slice a build error instead of a silent gap in the dump. |
| `crates/koushi-core/src/runtime/notification_delivery_tests.rs` | The "the send key never reaches the snapshot" assertion is retargeted at the delivery settings and the delivery runtime slice, which is where a leaked key would appear now that the whole-state dump is a summary. |

Tests: `crates/koushi-protocol/src/state_update/tests.rs` (snapshot and delta
dumps carry none of the fixture's identities or text, and the delta names exactly
the slices it changed) and `crates/koushi-state/tests/state/appstate_debug.rs`
(`AppState`, `SessionInfo`, `SessionState`, `SyncState`, `ComposerState` including
a reply, both panes including a pinned reply, `BasicOperationState`,
`FilesViewState`, `ThreadsListItem`).

## Review

Two post-implementation rounds with an independent reviewer (GLM via
`shinaoka-advisor/scripts/pi_review.sh`). Round 1 found three live paths the first
version still printed — `SessionState::Authenticating`'s typed homeserver, the
`SyncState` failure reason, and a summary that had dropped both panes — plus a
`changed_slice_names()` no test pinned and a `koushi-core` secret assertion the
summary had made vacuous. Round 2 found two more of the same class in the new
impls: the reply target behind `ComposerMode` / `PendingComposerSendKind`, and
`ThreadOpenIntent::PinnedReply`'s event id, both of which the summary reaches
through the composer and the thread pane. All were fixed with a test each, except
`changed_slice_names()`, which is pinned by the delta test and by the exhaustive
destructuring.

## Reachability sweep

After the change, `format!("{versioned_snapshot:?}")` and `format!("{delta:?}")`
reach only: redacted leaf `Debug` impls, count summaries, booleans and enums.
Thirty-eight state structs still derive `Debug` over raw `String` fields
(`InvitePreview`, `SearchResult`, `PinnedEvent`, `DeviceTrustSummary`,
`InviteTargetCandidate`, `AppError`, ...). None of them is reachable from those
two dumps any more, so they are latent: they matter only if something prints them
directly, which is the next class to take (#1979 and #1972/#1878 already redacted
the ones that are printed).

## Verification

```bash
cargo fmt --check                                                     # clean
cargo test -p koushi-protocol --profile ci                            # 53 + 19 passed
cargo test -p koushi-state --profile ci                               # 133 + 1244 + 3 passed
cargo test -p koushi-core --profile ci                                # 2011 passed, 11 ignored
cargo test -p koushi-sdk --profile ci                                 # 303 + 102 passed
cargo clippy --profile ci -p koushi-protocol -p koushi-state --all-targets --locked -- -D warnings
```

RED before the change: both new test files failed on the first run with the raw
homeserver, user, device, room and event ids, both previews, the `AppError`
message and the id-keyed `thread_unread` / `profile_*` map keys in the dump.

`cargo fmt --all -- --check` cannot be the gate in this repository: it rewrites
`vendor/matrix-rust-sdk`, so CI runs `cargo fmt --check`.
