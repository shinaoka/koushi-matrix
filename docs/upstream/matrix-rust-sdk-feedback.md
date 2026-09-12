# Matrix Rust SDK Feedback Packet

Date: 2026-08-03

This note separates SDK-upstreamable material from desktop-product decisions. Element Desktop/Web compatibility work in this repository is UX-only and is intentionally out of scope for the SDK feedback.

## Fork Maintenance Snapshot

As of 2026-09-12, the checked-in SDK gitlink follows the maintained
`shinaoka/matrix-rust-sdk-work` fork at commit `a3754f6be` on `main`, which is
upstream `matrix-org/matrix-rust-sdk` `6602de58e` (2026-09-11) merged into the
previous pin `a04792c7a` plus the retained Koushi customizations. The
2026-07-27 snapshot below described the state before that upgrade. The fork is expected to be managed and
maintained for a while; local SDK patches should therefore stay as small topic
commits with clear upstream intent instead of being squashed into an opaque
vendor snapshot.

The current Koushi-required SDK topic stack is:

- `feat(event-cache): publish committed response fence`
- `fix(event-cache): classify committed room membership`
- `test(event-cache): exercise joined update failure fence`
- `fix(event-cache): refresh authoritative room live tail`
- `fix(event-cache): retain targeted persisted gap work`
- `fix(event-cache): reconcile live tail from older anchor`
- `test(event-cache): require stable live-tail anchors`
- `fix(timeline): publish gap barrier in visible suffix`
- `fix(crypto): ignore replayed SAS starts`
- `fix(room-list): expand own-member state key`
- `feat(room-list): expose committed all-rooms response`
- `feat(room-list): expose authoritative all-rooms readiness`
- `feat(room-list): correlate response checkpoints`
- `fix(crypto): harden async delivery ownership`
- `fix: avoid identity query Olm lock deadlock`
- `Handle stale order tracker readers`

These are retained because Koushi currently depends on their public or
behavioral contracts for verification delivery, restricted verification sync,
legacy room/timeline catch-up, and room-list request compatibility. The first
upstreaming unit should be the smallest self-contained crypto verification
patches; event-cache/live-tail work can remain fork-maintained until the
desktop production evidence is easier to summarize.

Matrix Rust SDK PR #6753 (`sliding_sync: eagerly send verification responses
after a sync response`) was still open when this snapshot was taken and is not
part of the pinned SDK revision. Koushi therefore keeps its own wait-state
diagnostics around `to_device_delivery`, `sas_start`, `mac`, and
`normal_sync_resume` so a verification stall can still be assigned to a product
or SDK boundary without logging private Matrix payloads.

## Upstreamable Patch Material

- Element X Megolm send parity cleanup (issue #795, 2026-09-05) removes the
  Koushi-only readiness fence, repeated/duplicate pre-share, initial-share
  repair, manual force-new/discard/share-index-0/resend-index-0 APIs, and their
  original-recipient ledger. The retained send flow is the stock sequence:
  member sync, dirty/untracked key query, one `preshare_room_key`, then encrypt.
  Receive-side key requests, gossip, backup lookup, decrypt retry, member-reload
  rotation, and read-only diagnostics remain. Upstream intent: no new upstream
  feature; this shrinks the fork back toward upstream and leaves only separately
  justified diagnostic deltas.

- Persisted outbound Megolm rotation attribution (issue #794, 2026-09-05)
  stores one versioned, 128-entry exact room/session-to-closed-reason ledger in
  the existing encrypted crypto-store custom-value mechanism. The crypto
  machine restores it on construction, updates it only after successful new
  outbound-session creation, replaces exact duplicate keys without growth, and
  evicts oldest-first. Missing, malformed, unknown-version, oversized, or
  unwritable data fails closed to unavailable without blocking encryption or
  sending. Raw identifiers remain internal store keys and never enter exported
  diagnostics or public return values. Upstreaming intent: propose the generic
  bounded crypto-store attribution primitive and exact closed-reason accessor;
  keep Koushi UI wording and product diagnostic projection outside the SDK.

- Issue #541 manual current-session index-0 recovery (SDK topic commit
  `cb164845`, 2026-08-17) adds an immutable initial-share proof ledger,
  request ownership tags with legacy-pickle fail-closed migration, and the
  standard `m.forwarded_room_key` transport path for a bounded one-shot resend.
  Upstream intent: propose the smallest generic persisted initial-share
  ledger/request-ownership and forwarded-key/session-persistence primitives;
  keep Koushi's dangerous UI, actor fence, diagnostics, and temporary manual
  enablement out of the SDK. No periodic or activity-triggered replay is
  included. Verification evidence: 21 focused resend tests pass at
  `cb164845`; the full crypto suite passes with 580 tests and 1 ignored at
  that exact revision. The local encrypted-room QA lane is currently blocked
  before this operation by its existing A2 SAS proof-method prerequisite.

- Initial outbound Megolm Olm-claim repair (issue #523, 2026-08-14) remains
  upstreamable patch material, retained in the vendored SDK behind an
  independent default-off builder option. The vendored crypto layer retains
  only the exact still-eligible devices that failed the initial index-0 share;
  the matrix-sdk layer serializes one targeted `/keys/claim` through the
  existing claim lock, and one sync-driven wake is fenced by the existing short
  first-event deadline. The repair reuses standard signed one-time/fallback-key
  verification and the normal encrypted `m.room_key` path; homeserver
  acceptance still does not imply recipient decryption. Closed diagnostics
  expose only runtime aliases, buckets, counts, and elapsed time. Upstreaming
  intent: propose the smallest generic targeted claim/retry API and keep the
  product-specific fence/diagnostic projection in Koushi; no custom Matrix event
  or persistence should be upstreamed. Production enablement requires new
  measured evidence and an explicit canon/product decision; passing tests alone
  is not sufficient.

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

- Historical/superseded committed per-room sync-response provenance
  (2026-07-17, issue #275):
  `EventCache` retains a private-safe `CommittedRoomTimelineObservation` for
  each joined room after timeline topology persistence. It distinguishes a
  response with no timeline mutation from one that inserted an exact opaque
  gap, and late subscribers receive the latest observation. Ancillary
  post-processing failures cannot erase already-committed provenance. Why:
  this was introduced so clients using legacy `/sync` could obtain the same
  exact, generation-fenced live-catchup anchor that SyncService exposes through room-subscription
  checkpoints; otherwise a newly received live event can coexist with an
  unrepaired offline interval. Upstreaming intent: propose the retained
  backend-neutral observation API upstream after the #275 production proof,
  keeping room IDs, event IDs, pagination tokens, and raw errors out of Debug
  output. Issue #412 removed the desktop Legacy Sync adapter and no production
  Koushi path consumes this API now.

- Historical/superseded committed sync-response fence (2026-07-17, issue
  #275): `EventCache` also
  retains one `CommittedRoomUpdatesResponse` only after all joined/left room
  topology work for that response has completed. Its monotonic response
  sequence and aggregate room counts let consumers distinguish an unchanged,
  omitted room from a response that has not committed yet. This closes the
  legacy `/sync` ambiguity without exposing room IDs, event IDs, pagination
  tokens, message bodies, or raw errors. The former desktop adapter used an
  omitted room only as a bounded signal to inspect and repair its newest
  persisted live-edge gap after restart. Issue #412 removed that adapter.
  Current omission repair uses the exact response-correlated
  `RoomListService` room checkpoint plus its matching global committed
  all-rooms sequence.

- Idempotent remote SAS-start replay (2026-07-20, issue #285 hardening): a
  repeated `m.key.verification.start` from the same peer, device, and flow no
  longer replaces the already-adopted responder SAS continuation. Replacement
  previously discarded accepted state and could end a valid exchange with a
  commitment/key mismatch when overlapping sync delivery replayed the start.
  Locally initiated simultaneous starts and QR-to-SAS transitions retain their
  existing origin-specific tie-break paths. SDK tests cover exact remote replay
  through successful identical emoji/key completion and separately preserve
  simultaneous-start behavior. Upstreaming intent: submit this minimal crypto
  state-machine patch with the replay regression after the desktop live E2EE
  proof; keep protocol identifiers and raw cancellation text out of evidence.

- Exact own-member required-state key (2026-07-20, issue #285 hardening):
  `RoomListService` expands the MSC4186 `m.room.member` `$ME` placeholder to the
  authenticated user's exact state key when building the all-rooms list and
  room subscriptions, while retaining `$ME` when no authenticated user exists.
  Other placeholders and event types are unchanged. This improves compatibility
  with servers that advertise MSC4186 but do not expand `$ME`; it is not treated
  as proof that their invite-list semantics are otherwise complete. Unit and
  integration requests assert the exact expansion. Upstreaming intent: submit
  the helper and request-shape regressions independently of Koushi's backend
  capability preflight.

- Element X all-rooms request parity guard (2026-08-03, issue #412 PR1):
  a direct source comparison to the Issue #412 and Element X 26.07.28 SDK pin
  `ccd225e58eb900e321411397d1c13c2d9b312bb6` found the same request contract:
  the `room-list` connection, sole `all_rooms` list, unset invite filter,
  timeline limit `1`, ordered
  `DEFAULT_REQUIRED_STATE`, and enabled account-data, all-subscribed receipts,
  typing, and capability-gated thread subscriptions with limit `10`. Koushi's
  existing narrow own-member patch expands only `$ME` to
  `@example:localhost` in the authenticated test request. Local SDK commit
  `1e70c6661c6f14fe8760c76cb8022fa12bc43861` adds
  `all_rooms_request_matches_element_x_26_07_28`, which drives the real first
  `RoomListService` request through `MatrixMockServer`/wiremock and asserts the
  serialized URL, query, connection ID, list, required state, filter, timeline,
  and extension contract. For durable TDD RED evidence, the real request-capture
  test was first run with an intentionally wrong sentinel endpoint expectation
  using
  `(cd vendor/matrix-rust-sdk && cargo test -p matrix-sdk-ui all_rooms_request_matches_element_x_26_07_28)`;
  the single test failed on the endpoint mismatch with exit `101`. After the
  expectation was changed to the authoritative Element X endpoint, the same
  command passed `1/1` with exit `0`. No request-builder production change was
  necessary, and this test-only guard does not justify a wholesale SDK rebase
  or upgrade.

- Committed all-rooms response and projection readiness (2026-08-04, issue #412
  runtime):
  `RoomListService` exposes a read-only latest-value observable that advances
  only after a successful `all_rooms` Sliding Sync response has completed
  client processing, including the event-cache commit. Its public payload is
  limited to a process-local monotonic sequence, `pos_present`, and coarse
  complete-range readiness; it contains no room IDs and never exposes the
  position value. The SDK separately retains the top-level response room IDs
  behind `RoomList`, excluding extension-only updates, so its public
  `current_entries_snapshot()` can correlate filtered entries with the same
  response sequence without exposing the ID set. Active-room subscription
  checkpoints carry that exact response sequence as token-free provenance,
  including responses with no timeline update, and are published before the
  matching global latest value. This lets callers distinguish an included room
  from a room omitted by that exact incremental response without treating
  omission as leave. Before the first response of
  a sync/recovery cycle the snapshot remains provisional cache data; afterwards
  dynamic entries reset to the observed response set, so a cache-only omitted
  room cannot survive an authoritative full-range projection. Failed requests
  leave the committed value unchanged, while a later successful reconnect
  advances it. Why: callers need to distinguish `SyncService::State::Running`
  from a complete response and must reconcile that exact SDK-owned projection
  before declaring connectivity. Upstreaming intent: propose the lifecycle
  observable, range readiness, and response-correlated snapshot as additive
  room-list APIs independently of Koushi product state; they add no second sync
  loop or application policy to the SDK.

- Shared encryption-sync permit injection (2026-08-04, issue #412 runtime):
  `EncryptionSyncPermit` has a production constructor and `SyncServiceBuilder`
  accepts an application-owned permit. Koushi uses one permit across the
  provisional verification owner and the normal `SyncService`, stopping and
  joining the former before starting the latter. Why: the previous public API
  could not express a lifecycle handoff without using a test-only constructor
  or creating unrelated permits. Upstreaming intent: propose the additive
  constructor and builder injection as an explicit single-owner contract.

- Non-blocking own-user identity query (2026-07-30, issue #375):
  `Encryption::request_user_identity` clones the current `OlmMachine` and
  releases the client's read guard before awaiting `/keys/query`. Previously
  the response path reacquired the same Tokio `RwLock`; if Olm regeneration
  queued a writer while the request was in flight, the original read guard
  blocked the writer and writer preference blocked the nested read forever.
  The regression delays the key query, queues regeneration, and requires both
  operations to settle. Why: Koushi's authoritative current-device trust
  recheck exposed this as an intermittent login stall in
  `Provisional { RecheckingTrust }`. Upstreaming intent: submit the minimal
  lock-scope change and deterministic concurrency regression independently;
  it changes no identity or trust policy.

- Deferred unknown-device verification request (2026-07-20, issue #285
  hardening): a valid to-device `m.key.verification.request` is retained when
  sender `DeviceData` has not arrived yet, instead of being irreversibly
  discarded. The queue is FIFO-bounded, timestamp-gated, and deduplicated by
  sender/transaction flow. It uses the existing device-key query manager and
  replays only after matching key data has been committed, re-running normal
  timestamp/self/device validation and materializing one stable cached handle. Tests
  cover recovery, duplicate/query coalescing, a still-missing response followed
  by a later successful response, expiry, and the capacity boundary. The
  pending slot is preserved across scheduling/store errors and still-missing
  responses; duplicates retry only a previously failed schedule. Because the
  recovered handle can be created after the original raw-event callback, so
  normal and recovered materialization publish into one typed incoming-request
  lease stream. One owner lock contains pending entries, stable publications,
  subscriber generation, and active head claim, with a combined maximum of 32.
  Replay converts its pending slot to a publication under that lock. An active
  lease retains its slot; commit pops it and drop releases the claim in place.
  Subscriber generation check and claim are one linearization point. An absent
  subscriber does not fail key-response processing, and a post-commit replay or
  cache/reschedule failure returns the already-applied key changes while leaving
  retry state schedulable. Capacity is strict FIFO and never evicts an existing
  obligation to admit a newcomer. At capacity a new materialized request is
  explicitly terminally cancelled and queues an outgoing cancel rather than
  being silently lost after sync cursor advancement; a newest unknown-device
  request is not retained and does not schedule a query. Cache insertion returns an atomic
  existing-versus-inserted result and never upgrades unrelated same-flow cached
  provenance. Query ownership is an explicit state machine rather than one
  scheduling boolean. A response RAII claim is acquired before identity-manager
  processing and spans durable commit plus its later awaits; cancellation or
  error returns claimed entries to `NeedsQuery`, while normal still-missing
  completion enters `WaitingForExternalUpdate`. Both the crypto delivery and client wrapper use
  constant redacted `Debug` output rather than delegating to request/store/client
  internals.
  Generic raw handlers are deliberately independent compatibility fanout; a
  partially cancelled handler set can repeat on redelivery. The typed stream is
  the product-delivery API, while transport semantics remain at-least-once with
  stable sender/flow identity. This preserves downstream exhaustive matches by
  leaving `ProcessedToDeviceEvent` unchanged.
  Replay failure after device keys are applied is isolated from that successful
  key response and leaves the pending request retryable.
  Element X and the current FFI raw-event
  observation shape were useful comparisons, but do not yet provide this
  no-loss notification. Upstreaming intent: propose the bounded crypto recovery
  and unified typed incoming-request handle stream together; neither proposal
  contains desktop UI policy.

- Non-persisting sync token option (2026-07-20, issue #285 hardening):
  `SyncSettings::save_sync_token(false)` processes and persists ordinary sync,
  crypto, device, account-data, and to-device changes and calls application
  handlers, but does not replace the client's global persisted sync token with
  that response's `next_batch`. The default remains `true`. Koushi uses the
  option with `NoToken` only for its verification-only room-suppressed sync, so
  a restored canonical room cursor survives process exit/account switching and
  a fresh store remains tokenless. Upstreaming intent: expose this as a generic
  opt-in for purpose-filtered one-shot sync consumers, with SQLite-reopen and
  handler-delivery tests.

Current SDK-only patch area:

- `vendor/matrix-rust-sdk/crates/matrix-sdk-search`
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/search_index`
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/send_queue/mod.rs`
  (`SendHandle::transaction_id()` accessor only)
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/mod.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk/tests/integration/event_cache/mod.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/verification/requests.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/verification/machine.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/verification/mod.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/machine/mod.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/identities/manager.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/src/machine/tests/interactive_verification.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/encryption/mod.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/room_list_service/mod.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/room_list_service/all_rooms.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/encryption_sync_service.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/sync_service.rs`
- `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/tests/integration/room_list_service.rs`

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

- Local-only member profile adapter (2026-08-01, diagnostics/privacy milestone):
  `crates/koushi-sdk` exposes `room_member_profiles_no_sync`, which validates
  requested user IDs and reads only already-populated room-store entries through
  `get_member_no_sync`; it performs no member sync or homeserver request.
  `TimelineActor` now uses that adapter when building live and authoritative
  receipt-observation actions, and sends the resulting profile/receipt actions
  through the existing actor-generation fence so late observations cannot
  update a replaced timeline. This local-only, generation-fenced integration is
  complete; it is an application adapter, not a vendored SDK patch.
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
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/Cargo.toml test_replayed_sas_start_keeps_adopted_responder_sas`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-crypto/Cargo.toml test_simultaneous_sas_starts_keep_lexicographically_smaller_start`
- `cargo test -p matrix-sdk --lib test_request_user_identity_does_not_deadlock_with_olm_regeneration`
- `cargo test -p matrix-sdk --lib`
- `cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-ui/Cargo.toml room_list_service`
- `(cd vendor/matrix-rust-sdk && cargo test -p matrix-sdk-ui all_rooms_request_matches_element_x_26_07_28)`
- `(cd vendor/matrix-rust-sdk && cargo test -p matrix-sdk-ui test_all_rooms_are_declared)`
- `(cd vendor/matrix-rust-sdk && cargo test -p matrix-sdk-ui committed_all_rooms_response_observable)`
- `git -C vendor/matrix-rust-sdk diff --check`

## Remaining Before Upstream PR

- Decide whether to add a `matrix-sdk` store-kind boundary test for encrypted index open failure with the wrong secret, or rely on the `matrix-sdk-search` encrypted directory coverage.
- Add an SDK late-decryption reindex hook or keep the current documented gap as an API feedback item.
- Prepare the upstream patch with only the remaining SDK search-index diff under `vendor/matrix-rust-sdk`.

## 2026-08-10 (#460): withheld-code feedback accessors

Added two doc-hidden `Encryption` accessors for the GUI phase of room-key
request feedback (issue #460):

- `room_keys_withheld_received_stream()` — surfaces the crypto store's
  withheld stream. Only `blacklisted`, `unverified`, `unauthorised`, and
  `unavailable` codes are retained by `add_withheld_info`; `no_olm`,
  `history_not_shared`, and custom codes are not correlatable from this stream.
- `room_key_withheld_codes(room_id)` — maps stored withheld entries to
  `(session, closed code)` via `get_withheld_sessions_by_room_id`.

## 2026-08-14: full-member-reload rotation correlation diagnostics

Koushi needs to distinguish normal first-use Megolm creation from replacement
sessions caused by a full member-list reload. The vendored SDK therefore adds a
minimal observation-only diagnostic seam across `matrix-sdk`,
`matrix-sdk-base`, and `matrix-sdk-crypto`:

- process-local member-invalidation provenance is retained until the next
  successful full `/members` reload;
- the reload emits a typed event containing a closed reason, bounded count
  buckets, request/processing timing, and the discard outcome;
- the reload and later rotation reuse the existing anonymous room alias; and
- an explicit discard retains its timestamp so replacement-session creation
  can report `discard_elapsed_ms`.

No Matrix identifiers, device identifiers, session identifiers, keys, event
content, URLs, or raw errors cross the observer boundary. The patch does not
change when member lists are fetched or when outbound Megolm sessions are
discarded. Upstreaming intent: propose the typed observer as an optional debug
contract if other SDK consumers need to diagnose unexpected rotation churn;
otherwise keep it as a narrow downstream patch.

## 2026-08-21 (#591): exact retained rotation-reason lookup

Local Encryption details need to correlate an event's full Megolm session
identity with the already-classified rotation reason without exposing that
identity through exported diagnostics. The vendored SDK therefore retains the
128 most recent successfully created `(room, session) -> closed reason`
boundaries inside the owning `RoomKeyDiagnosticHub` and adds one additive query
through `OlmMachine` / `Encryption`:

- callers provide the full room/session only inside the existing trusted Rust
  crypto boundary;
- the result is only `Option<RoomKeyRotationReason>`;
- failed creation adds no attribution, exact duplicate keys update in place,
  and oldest entries are evicted deterministically; and
- storage is process-local and resets with the crypto machine.

The accessor returns no room/session value, alias, key material, content, or raw
error and does not change rotation, sharing, recipient, retry, or persistence
behavior. Upstreaming intent: propose this bounded diagnostic lookup alongside
the typed rotation observer; remove the downstream accessor if upstream offers
an equivalent closed correlation API.

## 2026-08-21 (#577): generation-scoped encryption readiness

A tracked, locally clean user can gain a new device before the sender processes
`device_lists.changed`. Standard pre-share then sees the stale local device set.
The downstream SDK patch adds an opt-in first-event readiness contract while
leaving upstream defaults unchanged:

- `EncryptionSyncService` publishes monotonic `Pending`, `Received`, `Failed`,
  and `Cancelled` generations through a client-owned watch; stale guards cannot
  settle a replacement generation;
- a bounded 128-entry exact outbound-session registry retains
  `Unfenced|Fencing|Ready`, so cancellation/failure and registry eviction cannot
  turn a resident index-0 session into a false ready bypass;
- an unfenced index-0 session waits under one 10-second deadline for the current
  encryption generation, performs an out-of-band full active-member
  `/keys/query`, commits it through the standard crypto path, and repeats normal
  pre-share before event encryption;
- the send queue treats the closed `EncryptionReadinessError` as recoverable and
  leaves the local echo pending; and
- the existing room-key diagnostic observer receives only anonymous aliases,
  closed states/outcomes, count/index buckets, generation/eviction counts, and a
  retryable flag.

The option defaults off and does not enable #510 or #523, alter #541's original
recipient ledger, introduce a new Matrix event, query before unchanged ready
sessions, fall back to plaintext, or infer historical entitlement for a device
first visible after the authoritative response. Underlying HTTP/crypto failures,
identifiers, sync positions, key material, URLs, content, and raw errors do not
cross the typed failure or diagnostic boundary. Upstreaming intent: propose the
generation watch and retryable new-session fence as separate opt-in APIs; retain
only the smallest downstream layer if upstream adopts equivalent lifecycle and
full-query/pre-share primitives.

## 2026-09-11: removed the superseded committed room-updates fence

Fork branch `chore/drop-superseded-committed-fence`, merged to the fork's
`main` as `a04792c7a` (based on the previously pinned `e85bc9e76`; the gitlink
now tracks fork `main`).

The committed room-updates response fence was introduced for the legacy `/sync`
adapter that issue #412 removed. It had no production consumer left:

- Koushi reads the `RoomListService` room-subscription checkpoint plus the
  committed all-rooms sequence, not `EventCache`'s committed response fence.
- The event cache only needs its own `latest_sync_observation`, which is
  retained.

Removed from the vendored SDK: `CommittedRoomUpdatesResponse`,
`CommittedRoomTimelineObservation`, `CommittedRoomUpdateMembership`,
`EventCache::subscribe_to_committed_room_updates_responses`,
`EventCache::subscribe_to_committed_room_timeline_observations`,
`Client::latest_room_updates_response_sequence`, the
`SequencedRoomUpdates`/`RoomUpdatesPublicationSequence` wrapper, and the
retained observation map with its senders and publish block. The event cache is
fed again by the upstream `Client::subscribe_to_all_room_updates` broadcast, and
`handle_room_updates` no longer takes an unused `response_sequence`.
`Room::reshare_room_key`, an unused wrapper left over from the removed manual
index-0 resend work, is also dropped. `SyncSettings::save_sync_token` and its
non-persisting sync plumbing are dropped as well: issue #412 removed the
verification-only filtered sync that used them, and no Koushi path sets the
option.

Net diff against the pinned revision: 10 files, +32 / -713 (of which 169 lines
are fork tests that only exercised the removed fence, plus three
`save_sync_token` tests). Upstreaming intent: none; this is removal of
downstream-only APIs, not a new upstream proposal.

Verified: `cargo check -p matrix-sdk -p matrix-sdk-ui --all-targets` passes;
`matrix-sdk-ui` `committed_all_rooms_response` tests pass; the
`event_cache::live_tail_refresh` integration tests pass (7/7). The pre-existing
flaky `event_cache::threads::test_multiple_valid_edits_update_thread_summary`
fails nondeterministically both with and without this change (1/6 failures at
the pinned revision), so it is not a regression from this removal.

Not yet done in this pass: the remaining test-only fork API (`repair_timeline_gap`
has no production caller) and the 825-commit upstream rebase. A trial merge of
`upstream/main` into the pinned revision conflicts in 24 files, concentrated in
`event_cache`, `room_list_service`, `timeline`, and `matrix-sdk-base`/crypto
identity handling.

## 2026-09-12: upstream SDK upgrade to 6602de58e

The gitlink moved from the 2026-06-10 base `a04792c7a` to upstream
`6602de58e` (825 upstream commits). The fork merge is
`shinaoka/matrix-rust-sdk-work` PR #8 (merge commit `fc780574d`), followed by
`a3754f6be` for test-target adaptations.

### Retained customizations (ported onto the new upstream structures)

- **ngram / CJK search** (explicitly excluded from removal):
  `SearchIndexStoreKind::{UnencryptedDirectoryWithConfig,
  EncryptedDirectoryWithConfig, InMemoryWithConfig}`, `encrypted_directory_ngram`,
  the ngram tokenizer registration in `search_index`, and the CJK search
  candidates API. Regression tests: `test_search_index_store_kind_can_configure_ngram_tokenizer`
  (matrix-sdk), `test_ngram_search_matches_japanese_substring` and
  `ngram_schema_uses_named_body_tokenizer` (matrix-sdk-search), plus the
  Koushi `search_crawler` and `edit_redact_search` QA scenarios.
- **Event cache**: persisted gap inspection/repair (`RoomTimelineGapHandle`,
  `RoomTimelineGapRepair*`, `inspect_timeline_gaps`), cache-only back
  pagination (`run_backwards_cache_only`), live-tail refresh
  (`RoomLiveTailRefresh*`) and `RoomTimelineSyncObservation`. Ported onto
  upstream's `StateLockReadGuard`/`StateLockWriteGuard` and
  `states::selectors`; `latest_sync_observation` feeds the room-subscription
  checkpoints. Regression tests: the `event_cache` integration targets plus
  Koushi's `timeline`, `timeline_nav` and gap-repair tests. At the old fork
  head the committed per-room response fence was already removed as unused;
  the sync observation itself is still consumed by Koushi's
  `MatrixCommittedRoomTimelineCheckpoint`.
- **Redaction replay**: `pending_redactions` re-applies a persisted redaction
  when its target only arrives later (or is delivered again). Regression test:
  `test_search_index_redaction_preserves_edit_aware_cache_hit`,
  `test_search_index_redaction_removes_redacted_event_when_cache_misses`.
- **Room subscriptions**: `RoomListService::subscribe_to_rooms_with_generation`
  / `reconcile_room_subscriptions_with_generation` and the per-room
  `RoomSubscriptionCheckpoint`, now implemented on top of the standard
  `SlidingSync::set_room_subscriptions` plus `subscribed_rooms()`. Generation
  tracking stays application-level. Regression tests:
  `koushi-core-testkit` room-subscription residency plus the Koushi
  `room_space`/`invites_dm` QA scenarios.
- **Encryption sync readiness**: `EncryptionSyncGenerationGuard` and the
  `begin_encryption_sync_generation` wiring in `run_iterations`/`sync`,
  adapted to upstream's stream API.
- **Crypto**: key-query response leases (`acquire_key_query_response_lease`,
  `covered_users`, multi-in-flight request metadata), the deferred
  unknown-device verification replay, rotation-reason diagnostics and the
  X.509 signature-upload field. Regression tests:
  `machine::tests::interactive_verification` (32 tests),
  `room_key_receive_diagnostics` (5), `persisted_rotation_reason` (4),
  `sas_start` replay (2).
- **Timeline**: lazy-reveal live pagination
  (`live_lazy_paginate_backwards_with_reveal`) and gap-repair projection
  settlement (`complete_gap_repair_projection`,
  `wait_for_gap_repair_projection`, `GapRepairProjectionSettlement`).

### Removed (upstream supersedes them, or they were unused)

- `RoomEventCacheSubscriber` (upstream's generic
  `event_cache::Subscriber`), and the fork's `room/subscriber.rs`.
- Search-index redaction/replacement hardening in `matrix-sdk`: upstream's
  `IndexableEvent`, media caption/filename and poll indexing, and
  `check_validity_of_replacement_events` cross-sender validation are strictly
  stronger. Replacement: upstream's `handle_room_message`/`handle_room_redaction`
  and media/poll handlers.
- `Room::subscribe_to_rooms` and the fork's private
  `SlidingSync::subscribe_to_rooms` helper → `SlidingSync::set_room_subscriptions`
  (+ `RoomListService::set_room_subscriptions`).
- `refresh_event_focused_cache` / `get_or_create_event_focused_cache` on
  `RoomEventCache` → `EventCache::event_focused` (upstream now owns the
  focused-cache lifecycle). Koushi had no production caller.
- Encrypted-room reply/thread-root extraction in the timeline controller
  metadata → upstream's `extract_reply_and_thread_root`.
- `subscribe_to_thread` / `subscribe_to_pinned_events` / `thread_pagination`
  wrappers on `RoomEventCache` → `EventCache::thread`/`pinned_events`. Koushi
  had no caller.
- `SlidingSync::reconcile_subscriptions` and `SlidingSyncSubscriptionDelta`
  (the fork's differential reconciliation from #518): with room subscriptions on
  the standard `set_room_subscriptions` plus `subscribed_rooms()` they had no
  production caller left, so PR #11 removed them and drove the remaining
  sliding-sync cache tests through the standard API.

### Follow-up: behaviors restored after running the SDK suites (PR #9)

Running the SDK's own suites against the merged revision exposed fork behaviors
that the merge had silently dropped and one regression introduced by the port.
All are fixed in `shinaoka/matrix-rust-sdk-work` PRs #9 (`5ba0c4790`), #10
(`f622e82db`) and #11 (`bec5f680b`):

- Room-subscription settings expand the `$ME` member placeholder again (the fork
  hardening from issue #285), with the upstream request-shape expectations
  updated. Regression: `room_list_service` integration tests plus
  `all_rooms_request_matches_element_x_26_07_28`.
- The targeted gap repair flushes linked-chunk updates to the store before
  post-processing; without it a joined gap was reported as `Progress` instead of
  `BoundariesJoined`. Regression: the five `event_cache::test_*gap*`
  integration tests.
- `RoomEventCache::clear()` (test-only) is back for the persisted gap-repair
  tests.
- The classic-sync token guard only drops tokens shaped like `s<stream>_...`
  instead of every non-numeric token, which had discarded valid opaque Sliding
  Sync tokens and broken the to-device token reload.
- `ReadReceiptSnapshot::changes_since` compares receipts explicitly; imbl 7's
  `OrdMap::diff` skips shared subtrees before comparing values, so a
  timestamp-only receipt update was reported as unchanged.
- `RoomEventCacheState::new` no longer performs a store write while rebuilding
  the pending-redaction map unless the replay actually changed an in-memory
  event, which is what made the SDK lib suite hang.

### Fixed in the same pass (PR #10)

- `cargo test -p matrix-sdk --lib` used to hang in the fork-added
  `event_cache::redecryptor::tests::test_event_is_redecrypted_even_if_key_arrives_while_event_processing`.
  `RoomEventCacheState::new` drained and flushed the linked chunk's pending store
  updates unconditionally after rebuilding the pending-redaction map, so creating
  a cache performed a store write that upstream's `new` never does; with a
  delayable store (the test's `DelayingStore`) that blocked cache creation.
  The flush now runs only when the replay actually replaced an in-memory event.
  The SDK lib suite completes again (663 passed).

### Known gaps discovered in the same pass

- Upstream serializes every cache kind (room, thread, pinned, event-focused) on
  one event-cache state lock (`states::StateLock`), so an in-flight
  event-focused pagination blocks room-cache reads for its whole network
  request. The fork's per-cache locking is gone; evaluate this for Stage 3.
- A redaction replayed from `pending_redactions` reaches only the room cache's
  copy of an event. Upstream keeps room and thread copies in separate linked
  chunks, so the thread copy stays unredacted and a thread aggregate can count
  it after a store reopen
  (`timeline::thread_list_service::tests::test_relation_aggregate_matches_after_persistent_reopen`,
  ignored with that reason). Stage 3 owns verifying/replacing the thread
  aggregate behavior.

### Still open for stages 2-4

- Simplify the room-list readiness checks and the custom checkpoints tied to
  sync responses (Stage 2's remaining bullet).
- Timeline gap-repair/history simplification around the standard focus and
  pagination APIs (Stage 3).
- Re-evaluate the crypto deferred-replay and readiness fences, and add
  private-data-free activation counters (Stage 4).
- Real-world validation by the user (startup, login, send/receive, Japanese
  ngram search, room switching, invites, reconnect, restore) has not been
  performed for this upgrade.
