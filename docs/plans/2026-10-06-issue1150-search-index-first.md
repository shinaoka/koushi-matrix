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
4. **M4 warm set.** The app already has the cache-only primitive: the SDK fork's
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
