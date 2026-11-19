# Media-gallery Debug redaction (#1972)

`TimelineMediaGalleryItem`'s hand-written `Debug` redacted `room_id`, `sender`,
`sender_label`, and `timestamp_ms` but printed `event_id` raw, so formatting an
`AppState` or `TimelinePaneState` with a selected room's gallery copied real media
event IDs into test output, CI logs, QA artifacts, or issue comments. That is the
media/file surface engineering rules "Secrets and Private Data" rule 13 requires to
be metadata-minimized, and rule 3 requires the `EventId(..)` placeholder for an
artifact-facing `Debug`. Same class as closed #1878 (`TimelineItem`/`ReactionGroup`).

`crates/koushi-state/src/state/timeline.rs:651` now prints `&"EventId(..)"`, which is
what the other 16 `EventId(..)` sites in `koushi-state` (`ScheduledSendItem`'s
`thread_root_event_id`, the files view, live signals, activity, native attention)
already do. `Serialize` and `PartialEq` are untouched, so
the projection, the snapshot DTO, and every comparison behave as before.

The existing test in `crates/koushi-state/tests/state/upload_staging_state.rs`
asserted the leak (`debug.contains("$media")`), which pinned it instead of catching
it; it now asserts `EventId(..)` and the absence of the raw ID. RED first: with only
the test changed, it failed on the raw
`event_id: "$media"` in the rendered struct.

The QA private-data gate in
`crates/koushi-qa/src/bin/headless_core_qa/scenarios/timeline.rs`
(`assert_upload_ux_state_contract`) listed the room ID, the filename, the caption,
and the MXC URI but not the gallery event ID; the gallery's own event ID is now in
that list, so the same leak would fail the `media` lane.

## Other raw `event_id` in `Debug` (found, out of scope here)

Repository search for `field("event_id"` leaves two sites that print the value
instead of a placeholder:

- `crates/koushi-protocol/src/command/timeline.rs:614` — `Command::EditText`. Every
  neighbouring arm in the same `Debug` impl (`LoadMessageSource`, `RequestRoomKey`,
  `DownloadMedia`, `Redact`, `SendReaction`, `RedactReaction`, `ForwardMessage`)
  prints `EventId(..)`, so this one is an outlier in a type rule 3 names directly.
  Same class as #1972, different surface; tracked in #1979 so the leak is not left
  documented only here.
- `crates/koushi-core/src/search.rs:256-274` — `SearchIndexMessage`, an actor mailbox
  message, prints `room_id`, `event_id`, `edit_event_id`, and `target_event_id` raw.
  Rule 3 lists `koushi-protocol` commands, not `koushi-core` actors, so whether this
  is in the rule's scope is a maintainer call.

Neither is changed here: #1972 is scoped to the media-gallery item, and both are
their own review.

## Verification

- `cargo test -p koushi-state --profile ci` — 133 + 1223 + 3 passed.
- `cargo test -p koushi-qa --profile ci --features qa-bin --bin headless-core-qa` —
  155 passed, so the changed gate compiles and its unit tests still hold.
- `cargo clippy --profile ci -p koushi-state --all-targets --locked -- -D warnings`
  — clean.
- `cargo fmt --check`, `node scripts/check-rust-test-structure.mjs`,
  `node scripts/check-agents-docs.mjs` — clean.
- `cargo check -p koushi-qa --profile ci --bin headless-core-qa --features qa-bin`
  — compiles with the new assertion. The `media` lane itself was not run: it needs a
  disposable local homeserver and `--core`, so the assertion is compile-verified,
  not execution-verified.
