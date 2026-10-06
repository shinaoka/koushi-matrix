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

## Remaining

1. **Bounded filtered refill.** Core fetches one candidate page per query
   variant before verification, so a heavily filtered query can under-report
   older matches. Until this lands, the branch also emits an empty first
   `Results` (the local store scan it used to precede is gone), which the QA
   absence check can mistake for a settled empty answer. Page until enough
   results pass verification or the pager is exhausted, with a bounded total
   candidate budget, and drop the now-vacuous first emission.
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
