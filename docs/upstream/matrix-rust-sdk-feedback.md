# Matrix Rust SDK Feedback Packet

Date: 2026-06-15

This note separates SDK-upstreamable material from desktop-product decisions. Element Desktop/Web compatibility work in this repository is UX-only and is intentionally out of scope for the SDK feedback.

## Upstreamable Patch Material

- `matrix-sdk-search` now has a `SearchIndexConfig` surface with a validated ngram tokenizer configuration.
- Invalid ngram bounds are rejected before index construction.
- The tokenizer name includes the ngram bounds, so a future schema/version check can distinguish index layouts.
- `matrix-sdk` search index store selection can pass custom search config for in-memory, unencrypted directory, and encrypted directory stores.
- `SearchIndexStoreKind::encrypted_directory_ngram(path, password, min_gram, max_gram)` is a convenience constructor for encrypted ngram search.
- SDK tests cover default tokenizer behavior, invalid ngram config, schema tokenizer selection, Japanese substring search, encrypted directory open/reopen and wrong-passphrase failure, edit ordering, redaction handling, and `matrix-sdk` search index wiring for an in-memory ngram index.

- `SendHandle::transaction_id()` accessor (2026-06-13, headless core Phase 5):
  `matrix-sdk/src/send_queue/mod.rs` gains a public getter for the private
  `SendHandle.transaction_id` field. Why: `RoomSendQueue::send()` generates
  its own transaction id internally; a caller that must correlate a queued
  send with the later `RoomSendQueueUpdate::SentEvent { transaction_id, .. }`
  (e.g. to map a client-supplied request/txn id to the SDK's txn id) has no
  way to learn the id at enqueue time — `LocalEcho.transaction_id` is only
  observable on the update stream, racing the caller. Upstreaming intent:
  small, additive, no behavior change — good candidate for an upstream PR
  alongside (or independent of) the search-index patch.

- Committed per-room sync-response provenance (2026-07-17, issue #275):
  `EventCache` retains a private-safe `CommittedRoomTimelineObservation` for
  each joined room after timeline topology persistence. It distinguishes a
  response with no timeline mutation from one that inserted an exact opaque
  gap, and late subscribers receive the latest observation. Ancillary
  post-processing failures cannot erase already-committed provenance. Why:
  clients using legacy `/sync` need the same exact, generation-fenced
  live-catchup anchor that SyncService exposes through room-subscription
  checkpoints; otherwise a newly received live event can coexist with an
  unrepaired offline interval. Upstreaming intent: propose the retained
  backend-neutral observation API upstream after the #275 production proof,
  keeping room IDs, event IDs, pagination tokens, and raw errors out of Debug
  output.

- Committed sync-response fence (2026-07-17, issue #275): `EventCache` also
  retains one `CommittedRoomUpdatesResponse` only after all joined/left room
  topology work for that response has completed. Its monotonic response
  sequence and aggregate room counts let consumers distinguish an unchanged,
  omitted room from a response that has not committed yet. This closes the
  legacy `/sync` ambiguity without exposing room IDs, event IDs, pagination
  tokens, message bodies, or raw errors. The desktop adapter uses an omitted
  room only as a bounded signal to inspect and repair its newest persisted
  live-edge gap after restart.

Current SDK-only patch area:

- `vendor/matrix-rust-sdk/crates/matrix-sdk-search`
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/search_index`
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/send_queue/mod.rs`
  (`SendHandle::transaction_id()` accessor only)
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/mod.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk/tests/integration/event_cache/mod.rs`

## API Questions

- Should `SearchIndexStoreKind` grow config variants, or should search index config be passed separately from the store kind?
- Should encrypted search index config include tokenizer/schema metadata in the index directory and force an explicit rebuild when config changes?
- Should `SearchIndexStoreKind::EncryptedDirectory*` have an SDK-boundary test for wrong-secret open failure, in addition to the lower-level encrypted directory tests in `matrix-sdk-search`?
- Should the public SDK API expose ngram presets for CJK use cases rather than only raw `min_gram` / `max_gram` bounds?
- Should SDK search return candidate event IDs only, leaving snippet/highlight verification to apps, or should it expose a first-class verified-result mode?
- Should key-backup restore expose a public backup-wide room-key download API
  with private-data-free progress/counter semantics, or should apps continue to
  hydrate keys room-by-room for currently joined rooms?
- Should login discovery expose MAS / delegated-auth metadata, especially
  delegated registration and account-management URLs, through a stable public
  SDK DTO? The desktop app can parse Matrix login flows and delegated OIDC
  compatibility today, but keeps `DelegatedAuthLinks::default()` until the SDK
  has a reviewed public path for these non-secret capabilities.

## Desktop Integration Findings

- Ngram works well as a candidate generator for CJK substring search, but desktop UI still needs exact verification against canonical visible message text or attachment filename before showing a result.
- Redactions and replacement events must be reflected in both the visible timeline model and search index. The desktop backend now removes redacted SDK timeline events from the visible timeline and local search candidates.
- Late decryption still needs a durable SDK hook. The current desktop plan needs an event-cache or decryption-complete notification that can enqueue search reindex work without polling every room.
- Thread timeline stability still needs validation with `matrix-sdk-ui::Timeline` focused on thread roots before enabling deeper thread subscriptions.
- Recovery state timing is observable through the SDK recovery state stream, but the desktop flow still needs a clear contract for when `Unknown` should become actionable after sync/account-data observation.
- Unread counts are a server/SDK observation, not a command-success signal.
  Matrix Rust SDK issue
  [#6211](https://github.com/matrix-org/matrix-rust-sdk/issues/6211)
  described unread notification counts that could disagree with other clients
  or fail to update after another session marked a room read; upstream
  [#6406](https://github.com/matrix-org/matrix-rust-sdk/pull/6406)
  fixed one read-receipt convergence path. Koushi's vendored SDK currently
  includes that fix, but desktop mark-read flows still must wait for explicit
  RoomActor/SDK success before treating a local Activity action as persistent
  unread clearance.
- `matrix-sdk-ui::Timeline::send_multiple_receipts` can intentionally drop
  fully-read/read-receipt fields when its timeline metadata believes an older
  receipt is already covered. For desktop unread clearance, Koushi sends the
  combined fully-read marker and private read receipt through
  `Room::send_multiple_receipts` so the homeserver receives a fresh read-marker
  request even while the room-list unread snapshot is stale. This is a desktop
  integration choice, not an SDK patch request.

## Non-Upstream Desktop Decisions

- Tauri native menu accelerators, Element-like right-panel modes, settings placement, and keyboard shortcut parity are app-shell behavior only.
- Element Desktop/Web was used as a UX reference. No Element Web/Desktop source code, assets, or icons have been copied into this repository.
- Search results in the desktop app remain exact-verified before display; raw ngram candidates are not a user-facing result type.
- MVP key-backup restore in matrix-desktop uses public SDK APIs only: import the
  recovery secret, then hydrate currently joined rooms. The desktop app will not
  add a vendored SDK accessor for private backup-wide internals merely for
  convenience; its restore summary scope is `JoinedRooms`. Broader restore
  requires a public SDK API or a separately reviewed minimal upstreamable patch.

## Verified SDK Checks

- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search,sqlite,e2e-encryption`
- `git -C vendor/matrix-rust-sdk diff --check`

## Remaining Before Upstream PR

- Decide whether to add a `matrix-sdk` store-kind boundary test for encrypted index open failure with the wrong secret, or rely on the `matrix-sdk-search` encrypted directory coverage.
- Add an SDK late-decryption reindex hook or keep the current documented gap as an API feedback item.
- Prepare the upstream patch with only the remaining SDK search-index diff under `vendor/matrix-rust-sdk`.
