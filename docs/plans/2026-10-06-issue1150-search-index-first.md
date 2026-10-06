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

## Remaining

1. **Shrink `SearchDocumentStore` to attachment metadata.** Delete
   `scan_candidates`, `verify_candidate`, `search_with_candidates*`,
   `SearchScanStats`/`SearchWithCandidatesStats` and `edit_aliases`; keep only
   what the Files view needs (room, event, sender, timestamp, filename,
   attachment, `is_edited`). Do not add an LRU around the dead API.
   - The Files view reads `attachment.filename`, not `attachment_filename`, and
     filename-only edits currently update only the latter — fix while shrinking.
   - `None` in an edit currently means "leave unchanged"; attachment removal
     needs explicit replacement semantics.
   - Crawled attachments start `is_edited = false`, unlike timeline projections.
   - Migrate the scenarios that `crates/koushi-search/tests/search_adapter.rs`,
     `crates/koushi-search/tests/unicode_search.rs` and
     `crates/koushi-core/src/search/tests.rs` cover (matching semantics to the
     pure verifier, candidate/resolution and scope ordering to index+cache+Core).
     Scan-stat assertions can go with the scan. Keep the Debug-redaction and
     maintenance tests.
   - Remove now-unused exports and stale module docs
     (`crates/koushi-search/src/lib.rs`, the header comment of
     `crates/koushi-core/src/search.rs`).
2. **Bounded filtered refill.** Core fetches one candidate page per query
   variant before verification, so a heavily filtered query can under-report
   older matches. Page until enough results pass verification or the pager is
   exhausted, with a bounded total candidate budget.
3. **QA false-green.** The headless QA scenario accepts the first `Results`
   event, which can be Core's initial empty emission, so absence checks can pass
   vacuously. Require a settled generation before asserting absence.
4. **M3** durable, generation-tagged index-commit checkpoints for a resumable
   bounded crawl, and typed missing/failure outcomes for cache-only offline
   reads.
5. **M4** warm set on the existing encrypted navigation persistence bringing the
   SDK display window up before timeline construction, with a startup-latency
   RED gate.

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
