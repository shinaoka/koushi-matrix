# #1150 search: index-first candidates and bounded memory (M2)

Status: in progress on `feat/1150-index-first-search` (app) and
`feat/1150-literal-bounded-search` (SDK fork, PR #19).

## Goal

Make the persistent, encrypted ngram index the only search candidate source and
stop retaining every message body and edit in RAM, while keeping
`koushi-search`'s matcher synchronous and pure and keeping verified-result
semantics (UTF-16 highlights, bidirectional width folding, voiced kana, case
folding, filename-field attribution, false-positive rejection).

## Landed

- SDK fork (`matrix-rust-sdk-work`, PR #19): literal, offset-free,
  newest-first paging (`Room::search_literal_page`, `SearchCursor`), a
  normalized body field with per-grapheme folding, and a cache-only resolved
  reader that resolves edits/redactions and splits a media caption from its
  filename. `MatrixLiteralSearchPager` pages a whole scope in `koushi-sdk`.
- `koushi-core`: candidates come only from the pager; each candidate is
  verified by resolving its current content from the encrypted event cache
  (`koushi_sdk::resolve_cached_message` -> `SearchableEvent` ->
  `koushi_search::verify_candidate`). The document-store scan and store-based
  candidate verification are no longer called; the verify diagnostic now
  reports `candidates_in_scope`, `cache_resolved`, `verified`.
- `SearchDocumentStore` retains attachment metadata only: messages without an
  attachment are not stored, edit text is never retained, a filename edit lands
  on the attachment (which is what the Files view reads) and marks the row
  edited, and redaction removes the row. The store-level matching tests moved to
  the pure verifier (`koushi-search/tests/`), and the actor's store maintenance
  tests now assert the attachment contract.
- Bounded filtered refill: the SDK task verifies each index page and keeps paging
  until it has enough verified results or the candidate scan budget (`500`) is
  spent, with `50` candidates per page. The pre-SDK local emission is gone, so a
  query emits exactly one `Results`; an SDK failure emits
  `AppAction::SearchFailed` plus `CoreEvent::OperationFailed`.
- **M3 durable crawl commitments**: an encrypted per-account record
  (`store/search_crawl.rs`) holds, per crawled room, the boundary event id and
  the counters the room row reports. Startup seeds the completed-room set from
  it, so a restart no longer re-crawls committed history; a completed or removed
  room updates it. The record is generation-tagged
  (`SEARCH_CRAWL_BACKEND_VERSION`): commitments from another version are ignored
  and dropped, making an index or extraction change a migration. Only
  identifiers and counters are stored; a missing file is an empty commit set and
  an unreadable one is a typed `StoreUnavailable` that means "crawl again".

## Remaining

1. **Typed failure on an SDK query failure** (done here): the actor previously
   relied on the pre-SDK local emission to settle the UI, so an SDK failure
   would have left it waiting. It now emits `AppAction::SearchFailed` and
   `CoreEvent::OperationFailed { CoreFailure::SearchFailed }`.
2. **QA false-green** is addressed by removing the pre-SDK emission: each
   accepted query now emits exactly one `Results`, so the QA helper that accepts
   the first `Results` can no longer read an empty placeholder as settled.
3. **M4** warm set on the existing encrypted navigation persistence bringing the
   SDK display window up before timeline construction, with a startup-latency
   RED gate (see the implementation pointers in Remaining).
4. **M4 warm set.** Landed: `NavigationState` now persists a bounded, deduplicated
   most-recent-first list of opened search results (identifiers only) via
   `AppAction::SearchResultOpened`, and the account actor loads each target's
   disk chunks cache-only (2 chunks / 200 events per target) once the session is
   up, so the first navigation reuses a warm display window. **Remaining: the
   startup-latency RED gate** proving the improvement, which the documented lane
   cannot supply without maintainer GO and real-homeserver credentials; the
   alternatives are a behavioural headless gate (network-blocked navigation to a
   warm target succeeds from the cache, and fails without the warm set) or an
   instrumented headless measurement of the primed phase at 1 thread with the
   effective thread count recorded.

The app already has the cache-only primitive: the SDK fork's
   patch surface exposes `RoomPagination::run_backwards_cache_only`
   (`CacheOnlyBackOutcome { anchor_present, .. }`), and
   `crates/koushi-core/src/timeline/navigation.rs` uses `anchor_present` to load
   backwards until an anchor is resident without network. The warm set is the
   persisted active room/anchor plus recent search room/event pairs run through
   that same load before timeline construction, so the first navigation does not
   wait on `/context` or `/messages`. Work needed: (a) persist a bounded,
   deduped recent search-target list (identifiers only) alongside the existing
   `NavigationState` fields (`active_room_id`, `main_timeline_anchor`,
   `room_scroll_anchors`), (b) prime at startup before the timeline is built
   (`timeline/actor.rs` subscribes the event cache and traces `cache` vs
   `network` origin; `timeline/focused_build.rs` owns the focused build that can
   wait on a remote `/context`), (c) a RED startup-latency gate. The documented
   startup-latency lane needs maintainer GO and real-homeserver credentials, so
   the local gate should be an instrumented headless measurement of the primed
   phase at 1 thread, recorded with the effective thread count.
5. **Evidence still outstanding**: a synthetic history-scale measurement showing
   zero retained body/edit bytes as indexed history grows (with index/disk size
   reported separately), the real-homeserver QA lane, and the SDK PR merge
   decision (its red checks are fork-wide pre-existing failures).

## Measured memory budget

`crates/koushi-search/tests/search_memory.rs` is the #1150 memory probe:
50,000 synthetic indexed messages carrying ~220-byte bodies leave
`resident_body_bytes() == 0` and `document_count() == 0` (before this change the
store retained one `SearchableEvent` per message, i.e. roughly 11 MB of bodies
plus per-event map overhead), and 5,000 attachment messages retain 5,000 Files
rows with `resident_body_bytes() == 0`. Index/disk residency and process RSS are
reported by the QA lanes, not by this probe.

## Open review findings (PR #1157, blocking merge)

An independent post-implementation review of the finished diff found twelve issues; two are
verified against source (content-policy bypass, `SearchResultOpened` bypassing the navigation
persistence diff) and the rest are on the PR for triage. The full list and severities are in the
PR #1157 comment. Fix order: content policy and index-commit acknowledgement first (both
blocking), then the durable-record invalidation/attachment-rebuild pairing, the edit-order and
sticker regressions, the supersede/settlement gap, and finally the warm-task ownership and
window-priming corrections.

### Fix round 1 (in review)

A second independent pre-implementation review (GPT-6.1 Sol) of the fix plan confirmed all twelve
root causes against the source and changed several of the planned fixes. Its verdicts:

- **#1, #2 (blocking)** - the planned fixes hold. #2 is implemented as "index the page, then report
  it": the crawler writes the page through the index guard (which commits synchronously) and a page
  whose write fails is reported as failed, so `Success` always carries an acknowledgement. No new
  result field; the existing failure outcome is reused.
- **#3** - remove-then-add rewriting is not a general migration and does not repair schema or
  tokenizer changes. Implemented instead by naming the index directory after the index contract
  version (`search-index.v{N}`), so a bump opens a fresh index before the client is built. The same
  version keys the durable crawl commitments, so an extraction change re-crawls and re-extracts
  together.
- **#4** - implemented, plus two corrections the review added: the durable record now also carries
  the content policy it was produced under (so a settings change that could not be saved is still
  detected on the next start), and membership pruning happens before the paused-crawler early
  return. An unreadable record no longer disables persistence for the session.
- **#5** - the planned cache-only attachment refresh is rejected as tail-only: `event_cache.events()`
  returns the loaded linked chunk, not the persisted history. The fix must rebuild attachment rows
  from persisted SDK events (identifier-only record plus SDK resolution) instead.
- **#6** - the planned timestamp watermark would break a real producer: the canonical timeline path
  records the original item timestamp for both Upsert and Edit (`timeline/item_projection.rs`),
  while the crawler records the edit event's timestamp. Version information must be normalised
  across both producers first.
- **#7** - fix sticker resolution; do **not** extend the resolver to polls while the index ignores
  poll replacements (stale poll text would pass verification as canonical). Removing unsupported
  poll indexing is the alternative.
- **#8** - carry both the pager timestamp and the indexed primary event id, and dedupe by resolved
  identity before counting toward the 50-result quota.
- **#9** - settle supersession where the search state actually changes (AppActor), not in
  `SearchActor`, because a new query is not the only invalidator (edit, too-short, close).
- **#10-#12** - the review recommends deleting the M4 warm set rather than tuning it: search-result
  navigation goes through the SDK's event-focused `/context` build (`EventFocusedCache` is in-memory
  and network-backed), which warming the room's live cache cannot accelerate, and a larger cache-only
  budget would spend itself before discovering that the target is out of reach. Pending a decision.

### Fix round 2 (post-implementation review of round 1)

The round-1 fixes were reviewed on the diff. Every fix was present; the review
found one privacy blocker and nine further defects. Fixed in `5cba5be7`:

- a content-policy change while a query runs now invalidates that query's result
  (re-verified under the current policy) instead of publishing an opted-out snippet;
- the index contract moves to version 2, because version 1 named a different index
  directory and had no content policy in the record, so a version-1 commitment
  describes an index this build never opens;
- attachment edits order by the edit's own time with `canonical` only as the
  tiebreak, so a newer crawl edit is not pinned by an older canonical observation;
  an edit rollback arrives as the redaction of the applied edit, which retires it
  (and any pending copy);
- the Files rebuild marks a room done only after a successful store read, clears
  its markers when the document store is cleared, bounds its projection to the
  newest events, and refuses a replacement from a different sender;
- the room-list notification (content policy + commitment pruning) is emitted at
  every speed and for an empty room list, so a restart with crawling paused no
  longer keeps the restrictive placeholder forever;
- a completed search request is no longer reported as superseded.

Still open from round 2: the once-per-session Files marker can hold rows a room's
later sync mutation should change until a catch-up crawl or timeline message
arrives (finding: currentness), and the whole-store read holds the event-cache
store lock for its duration (bounded per room, not per page). The M4 decision is
still pending.

### Fix round 3 (post-implementation review of round 2)

Round 3 accepted the round-2 fixes and found three merge blockers plus four
important defects. Fixed in `7db251eb`:

- the canonical upsert now carries the identity of the edit that produced its
  content, so the content and edit halves of one observation are a single guarded
  update (an upsert without an edit means the row is not edited, i.e. a rollback);
- redacting an edit retires it (per row) and drops the metadata it produced, a
  replay of that edit is refused, and the room's Files rows are rebuilt from the
  store again (the once-per-session marker is invalidated);
- the Files rebuild validates a replacement with the SDK's own replacement rules
  instead of a sender-only approximation;
- a content-policy change closes the search view, so results verified under the
  old policy can no longer be displayed.

Still open from round 3 (both need a design round, not a patch):

- the policy fence is only actor-local for the *transport* `SearchEvent::Results`:
  a notification deferred behind a saturated mailbox can leave the actor on the
  previous policy while the state has moved on. The UI is state-driven (the
  desktop renders `searchResults` from the snapshot), and the state closes the
  view, so no opted-out text reaches the UI; the residual exposure is a
  harness-level event in that window.
- the Files rebuild still reads a whole room's persisted events in one SDK store
  call while holding the event-cache store lock, then sorts and truncates. The
  bounded fix is paged chunk reads that release the guard between pages.
- the Files rebuild is still once per session per room: a mutation that does not
  change the room's latest event (an edit or redaction of a non-latest event) is
  not picked up until a catch-up crawl or timeline message arrives.

### Fix round 4 (post-implementation review of round 3)

Round 4 confirmed fixes 5, 6, 7, 8, 9, 10 and found five new defects, all in the
round-3 additions. Fixed in `559cb72e` and `b34b8f7a`:

- a redacted edit is retired for both halves of its upsert/edit pair, and a row
  remembers several retired ids, so retiring a newer edit does not forget an
  older one;
- a keyless observation no longer replaces a row that holds an edit (it can be a
  queued observation from before the edit), so a crawl rename cannot be erased;
- the Files rebuild projects its bounded collection as one set, so a rename whose
  original sits in an older page is not discarded, and a partial rebuild no
  longer deletes rows outside the range it read;
- the search actor no longer publishes `SearchEvent::Results` itself: the state
  admits a result and the AppActor publishes it, so publication is fenced by the
  authoritative policy and accepted query identity. An admitted result is the
  query's single settlement (a later result no longer replaces the answer);
- the retired local-first supplement wording is gone from the canon.

Round-4 item closed in the same round: a submitted query now carries the
account's content policy (`AppEffect::SearchMessages { content_policy }` ->
`AccountMessage::SearchQuery` -> `SearchActorMessage::Query`), and the actor
adopts it before capturing the generation its result is checked against. Query
verification therefore uses the same policy the state accepted the query under,
whether or not the crawler notification has been delivered yet.

Also raised: migrate commitments already written without an acknowledgement, make the "rebuild
search database" action actually rebuild the persistent index (the user help promises it), cover
attachment edit rollback and mixed producers, measure pending-edit residency in the memory probe,
and update the stale canon in `docs/architecture/state-machine.md`.

## Required evidence

RED-then-GREEN for literal completeness (operators/fields inert, raw+normalized,
one-scalar normalization), Japanese short/substring/normalized queries,
attachment filename search, edits/redaction/rollback/restart, bounded filtered
refill, paginated filtered results, old indexed messages findable after
eviction, restart, network-blocked startup and navigation to the result,
clear/account isolation and empty-store behavior; a synthetic history-scale
measurement showing zero retained body/edit bytes with index/disk size reported
separately; and the repository-local gates plus
`node scripts/check-sdk-submodule.mjs`.

## Review

Pre-implementation design review (independent model) returned "A with changes":
delete the unused search surface rather than bound it, keep the pure verifier
and its matching semantics, migrate the listed scenarios, and treat the refill
gap and the QA false-green as blockers to close before pushing M2.
