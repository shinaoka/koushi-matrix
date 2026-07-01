# Design: Search candidate/store unification (issue #162)

Status: proposed
Date: 2026-07-01
Issue: https://github.com/shinaoka/koushi-matrix/issues/162

## Goal

A message that is visible in the timeline (and highlighted for the current
query) must be counted as an exact search result when it satisfies the same
matching rules. Highlighting, exact-match counting, and result navigation must
use one consistent, Rust-owned matching/projection path.

## Context — confirmed structural root cause

Search uses two separate stores wired to different pipelines, and only one is
used to generate candidates:

- **Candidate source** = SDK ngram index (bounds 2..4), read via
  `koushi_sdk::search_message_candidates` → `client.search_messages()`. It is
  written **only** by the SDK `event_cache` ngram indexing task
  (`vendor/matrix-rust-sdk/.../event_cache/tasks.rs`), i.e. effectively live
  sync. `crates/koushi-core/src/search.rs::handle_query` uses it as the **sole**
  candidate source.
- **Verification corpus** = koushi `SearchDocumentStore` (`SearchableEvent`),
  written by timeline diffs **and** the history crawler
  (`crates/koushi-core/src/search_crawler.rs`, which paginates `room.messages()`
  = raw `/messages`, deserializes, and feeds `handle_index` →
  `document_store` only). Used **only** by `verify_candidate`.

The history crawler backfills old messages into `document_store` but **never**
into the SDK ngram index (koushi writes to the SDK index nowhere). So a
scrolled-up / backfilled past message is verifiable — `verify_candidate` via
`exact_range` (direct + NFKC-normalized) would match it — but it is **never
emitted as a candidate**, so verification is never invoked → 0 results, even
though the message is visible and the JS timeline highlighter (raw `indexOf`,
no normalization) marks it.

This is structural, not a timing/crawler-lag issue. CJK / reply / thread are red
herrings; the real axis is **backfilled history vs live-synced**. The user
confirmed the live symptom (visible messages containing a common word return 0)
and agreed the ngram index should be an accelerator, not the authority.

(Investigation recorded in agent memory `koushi-162-search-candidate-store-split`.)

## Decided approach — document_store as a first-class candidate source

Demote the SDK ngram index to an accelerator; make koushi's own corpus
authoritative for existence.

1. Add a candidate-scan method to `koushi-search` (`SearchDocumentStore`) that
   scans documents with the **same** `exact_range` matcher used by verification
   (direct + NFKC-normalized), scope-filtered, scored, and capped.
2. In `handle_query`, generate candidates from **both**:
   - the SDK ngram index (accelerator: recall beyond the in-process corpus), and
   - the `document_store` scan (authority: everything koushi has indexed —
     crawled history, live, CJK, short queries),
   then union/dedup by `(room_id, event_id)`, apply scope, verify/project, and
   emit results.
3. **Secondary (the "one matching path"):** align the JS timeline highlighter
   (`renderQueryHighlight`) matcher to the same rule as Rust — normalize both
   query and candidate text with NFKC + case fold before locating the span — so
   highlight presence and exact-match counting agree. (Rendering Rust-provided
   per-event highlight ranges is a heavier future option and is out of scope
   here.) The primary fix resolves the reported 0-count bug regardless; this
   closes the residual normalization divergence.

## Ownership

Search matching semantics remain Rust-owned. React renders results/highlights
and dispatches queries; it must not scan the DOM/rendered text to synthesize a
count, and must not make match existence depend on what happens to be rendered.

## Performance

The `document_store` scan is O(N) over in-memory `SearchableEvent`s. Cap results
(reuse `SEARCH_CANDIDATE_LIMIT`), score by recency (`timestamp_ms`) so the
union ordering is stable, and keep the SDK index as the primary recall path for
corpora larger than the loaded/crawled set. Note the bound explicitly; do not
silently truncate without a log/observability note if a cap is hit.

## Testing (verify-first — RED before fix)

- Rust unit/backend RED: insert a `SearchableEvent` into `document_store` (via
  `SearchIndexMessage::Upsert`, simulating crawler/timeline) that is **absent
  from the SDK candidate source**, run the query, assert count ≥ 1. Currently
  returns 0. Include a **synthetic** CJK 2-char case mirroring the reported
  shape. Use `crates/koushi-backend` / `koushi-core` search tests where the
  candidate source is controllable.
- headless-core-qa: crawl a room's history with the network blocked to live
  candidates, then search and assert the crawled message is found — proving the
  crawler's output is now searchable. Token-only, private-data-free (synthetic
  CJK fixture; no room/event/user ids, bodies, or raw SDK errors).
- Highlight/count consistency regression: a visible highlighted match and the
  Search panel exact-match count must agree under the unified rule.

## Scope / non-goals

- Do not change how the SDK ngram index is fed and do not build a new index.
- Do not change crawler timing.
- Fix is Rust-search-internal (`koushi-core/src/search.rs` + a `koushi-search`
  scan method) plus the small JS highlighter normalization alignment.

## Batch integration (#161–#163, single PR)

Rust-search-internal; barely touches the FE hot files, so it is parallel-safe
with #161/#163 on the single implementation branch.
