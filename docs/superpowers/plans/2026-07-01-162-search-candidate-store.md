# #162 Search candidate/store unification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make koushi's own `SearchDocumentStore` a first-class search candidate source so any message it holds (crawled history, live, CJK, short queries) is findable, with the SDK ngram index demoted to an accelerator.

**Architecture:** Add a `document_store` scan (same `exact_range` matcher as verification), union its candidates with SDK ngram-index candidates in `SearchActor::handle_query`, then scope + verify + project as today. Align the JS timeline highlighter normalization to the Rust rule so highlight/count agree.

**Tech Stack:** Rust (`koushi-search`, `koushi-core`), TypeScript/Vitest (highlighter), headless-core-qa.

## Global Constraints

- Search matching semantics are Rust-owned; React must not DOM-scan to count. (verbatim: AGENTS.md search rules)
- verify-first: add/extend a headless check that REPRODUCES the bug (RED) before the fix; fix is done only when it turns GREEN.
- Private-data-free evidence: no room/event/user ids, message bodies, pagination tokens, or raw SDK errors in tests/QA/logs. Use synthetic CJK fixtures.
- SDK ngram bounds are (2,4); do not change SDK index feeding or build a new index.

---

### Task 1: `SearchDocumentStore` candidate scan (RED→GREEN)

**Files:**
- Modify: `crates/koushi-search/src/document.rs`
- Modify: `crates/koushi-search/src/verify.rs` (expose `exact_range` to the crate, or add a `verify_event(&SearchableEvent, query)` helper reused by the scan)
- Test: `crates/koushi-search/tests/search_adapter.rs`

**Interfaces:**
- Produces: `SearchDocumentStore::scan_candidates(&self, query: &str, scope: &SearchScopeFilter, limit: usize) -> Vec<SearchResult>` where scope filtering matches the existing room/global scope semantics; results scored by `timestamp_ms` recency, capped at `limit`, deduped by `event_id`. Reuses `resolved_event` (so applied edits are honored) and the same `exact_range` matcher as `verify_candidate`.

- [ ] **Step 1: Write the failing test** — a store holding a message whose body is a synthetic 2-char CJK word returns exactly one candidate for that query via `scan_candidates` (and zero for an absent query). Also assert a short 1-char CJK query still matches (index-floor case).

```rust
#[test]
fn document_store_scan_finds_cjk_body_without_index_candidate() {
    let mut store = SearchDocumentStore::default();
    // synthetic CJK body ("ABCあいう" style — use a fixed 2-char CJK fixture)
    store.upsert_message(make_event("!r:test", "$e1", "検査しました"));
    let scope = SearchScopeFilter::Global;
    let hits = store.scan_candidates("検査", &scope, 50);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].event_id, "$e1");
    assert!(store.scan_candidates("該当なし", &scope, 50).is_empty());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p koushi-search --test search_adapter document_store_scan_finds_cjk_body_without_index_candidate`
Expected: FAIL (`scan_candidates` not found).

- [ ] **Step 3: Implement `scan_candidates`** — iterate `self.documents.values()`, `resolved_event`, scope-filter by room, run the shared `exact_range` over body then `attachment_filename`, build `SearchResult` (same shape as `verify::result`), sort by `timestamp_ms` desc, dedupe by `event_id`, truncate to `limit`. Introduce a `SearchScopeFilter` enum in `koushi-search` (Global | Room{room_id}) or reuse an existing scope type if present.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p koushi-search --test search_adapter`
Expected: PASS (all, including existing ngram/verify tests).

- [ ] **Step 5: Commit** — `test+feat(search): document_store candidate scan with shared exact_range matcher (#162)`

---

### Task 2: Union document_store candidates into `handle_query` (RED→GREEN)

**Files:**
- Modify: `crates/koushi-core/src/search.rs` (`handle_query`)
- Test: `crates/koushi-backend/tests/fake_backend.rs` (fake backend already controls SDK candidates — see `fake_backend_search_drops_ngram_false_positive`)

**Interfaces:**
- Consumes: `SearchDocumentStore::scan_candidates` (Task 1).
- Produces: `handle_query` result set = union (by `(room_id,event_id)`) of SDK-index verified candidates and `document_store` scan candidates; scope applied to both; ordering stable (score desc, then room/event id).

- [ ] **Step 1: Write the failing test** — drive a query through the search path where the SDK candidate source returns **empty** for the query but a matching message is present in `document_store` (simulating a crawled/backfilled message). Assert the emitted `SearchEvent::Results` count ≥ 1.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p koushi-backend --test fake_backend <new_test_name>`
Expected: FAIL (0 results — candidates only from SDK index today).

- [ ] **Step 3: Implement the union** — in `handle_query`, after collecting SDK candidates, also call `self.document_store.scan_candidates(query, &scope_filter, SEARCH_CANDIDATE_LIMIT)`; merge into `candidates_by_key`/results by `(room_id,event_id)`; keep the existing verify pass for SDK candidates (doc-store scan already yields verified `SearchResult`s). Ensure no duplicate results and stable ordering. Keep `SEARCH_CANDIDATE_LIMIT` cap; `log`-note if the scan hits the cap.

- [ ] **Step 4: Run to verify it passes** — the new test PASSES; existing search tests still pass.

Run: `cargo test -p koushi-backend --test fake_backend && cargo test -p koushi-core search`

- [ ] **Step 5: Commit** — `fix(search): union document_store candidates so crawled/visible messages are findable (#162)`

---

### Task 3: JS highlighter normalization alignment (RED→GREEN)

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx` (`renderQueryHighlight`)
- Test: a Vitest spec (co-located highlighter unit test)

**Interfaces:**
- Produces: `renderQueryHighlight(text, query)` locates the span after applying NFKC + case fold to both `text` and `query` (mapping the match back to original offsets), matching the Rust rule.

- [ ] **Step 1: Write the failing test** — full-width vs half-width and case-differing query highlights the substring (currently raw `indexOf` misses it).
- [ ] **Step 2: Run to verify it fails** — `npm --prefix apps/desktop run test -- --run <spec>` → FAIL.
- [ ] **Step 3: Implement** — normalize with `String.prototype.normalize("NFKC")` + `toLowerCase()` on both sides; find index in normalized space; map back to original indices for `<mark>` slicing.
- [ ] **Step 4: Run to verify it passes** + `npm --prefix apps/desktop run typecheck`.
- [ ] **Step 5: Commit** — `fix(search): align timeline highlighter normalization to Rust matcher (#162)`

---

### Task 4: Local headless proof (crawl → offline → search finds it)

**Files:**
- Extend the `headless-core-qa` search scenario (crate `koushi-core`, `--features qa-bin`).

- [ ] **Step 1:** Extend the search scenario: after a room's history is crawled into `document_store`, block live candidates (or use a message never live-synced), search a synthetic CJK term, assert found. Token-only output.
- [ ] **Step 2:** Run the local proof (search scenario) and confirm the new token is emitted.
- [ ] **Step 3: Commit** — `test(qa): crawled-history search proof (#162)`

## Self-Review

- Spec coverage: candidate-source unification (Tasks 1–2), highlighter unification (Task 3), verify-first RED (each task), headless proof (Task 4). ✓
- No placeholders except where a fixture id/name is intentionally synthetic and chosen at implementation time.
- Type consistency: `scan_candidates` signature is identical in Tasks 1 and 2.
