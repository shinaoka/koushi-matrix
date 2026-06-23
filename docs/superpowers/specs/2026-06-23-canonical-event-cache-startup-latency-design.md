# Design: Unify event loading around a canonical event cache

Status: design spec. Tracks GitHub issue #123
("Unify timeline and search event loading around a canonical event cache").
Date: 2026-06-23.

This spec decomposes #123 into three phases and fully specifies **Phase A
(observability)**. Phases B and C are scoped here as context and will each get
their own spec/plan before implementation. This document is subordinate to
`docs/architecture/overview.md`; if Phase C changes the architecture, amend the
overview first.

## 1. Symptom and problem

Felt symptom (reported by the maintainer): right after app launch, the
previously open room's timeline is **not restored instantly** — a "loading"
state appears for seconds — even though the SDK event cache on disk already
holds the events (one local profile: ~85,850 events, ~2,473 event chunks).

So this is **not** "events are not persisted". The problem is that app-level
load semantics are fragmented:

- Startup display is gated behind sync + room-select + timeline-build; the
  cheap part (reading the cache) runs last.
- The app cannot tell whether a given load was satisfied from cache, network,
  or a mix — there is no origin signal anywhere.
- User-driven loads are not uniformly on a priority path, so background work
  can delay a user-visible load.
- Timeline and search maintain separate notions of event data.

Per the issue's own follow-up comment, the **immediate** requirement is to make
the startup phases observable ("add private-data-free timing logs around
restore, timeline build, restoreTimelineAnchor, pagination, and crawler page
fetches, and report cache/network/mixed origin where the SDK exposes it"),
**before** any rewrite. Measure first.

## 2. Confirmed current behavior (evidence)

All file references verified read-only on branch `codex/fix-room-transition-lag`.

**Startup → first paint path (in order):**

1. `restore_session` → crypto store open + SDK client rebuild from the
   per-account encrypted store — `crates/koushi-core/src/account.rs:2993`,
   `koushi_sdk::restore_session_with_store`.
2. `RestoreSessionSucceeded` → session `Ready` → `StartSync` —
   `crates/koushi-state/src/reducer/session.rs:16`.
3. Continuous **sync (network)** populates `AppState.rooms`.
4. A room is selected → `SubscribeTimeline` —
   `crates/koushi-state/src/reducer/mod.rs` navigation path.
5. `TimelineBuilder::new(&room).build().await` —
   `crates/koushi-core/src/timeline.rs:832` (can wait on room state; **not**
   on the priority gate).
6. `timeline.subscribe().await` — `crates/koushi-core/src/timeline.rs:1419`
   (the disk-cache read; fast; **not** on the priority gate).
7. `TimelineEvent::InitialItems` emitted → React renders.

**The "loading" the user sees** is the backward-pagination spinner:
`apps/desktop/src/components/TimelineView.tsx:2134` renders it whenever backward
pagination state is `Paginating`. Auto-backfill
(`TimelineView.tsx:1935`, `maybeAutoBackfill`) fires `paginateBackwards`
whenever the viewport is not full (`scrollTop <= backfillThreshold`, default
threshold `0`). If `subscribe()` returns a small initial window, the viewport is
not full on open, so auto-backfill immediately issues a `/messages`-class
pagination and the spinner appears.

**Priority gate** (`crates/koushi-core/src/messages_backpressure.rs`):
account-wide gate for `/messages` history pagination. Global concurrency = 1
(`active: bool`). Two priorities: `Timeline` (user-visible) and `Crawler`
(background). A waiting timeline request blocks **new** crawler acquisitions
(`waiting_timeline`), so timeline has priority for *new* work. Limitations:

- **Non-preemptive.** If the crawler already holds the permit (mid `/messages`
  round-trip), a user pagination waits for that page to finish (one network
  round-trip).
- **`/messages`-pagination only.** The single production acquire site is
  `timeline.rs:1799`, immediately before `paginate_backwards` /
  `paginate_forwards`. Crawler acquire site: `search_crawler.rs:121`.
- **Initial room/context loads are not gated.** `TimelineBuilder::build` and
  `timeline.subscribe` (room open) and the initial focused-context build
  (search-result open) are **not** on the gate.

**Origin (cache / network / mixed): not observable today.** No `from_cache` /
`origin` field exists on `TimelineItem`, `TimelineMessageSource`, or pagination
state. `timeline.rs:1799` acquires the gate unconditionally before
`paginate_backwards`, regardless of whether the SDK will answer from cache or
go to the network.

**Search is a separate store.** `koushi-search` keeps an in-memory
`SearchDocumentStore` (verification text) plus an encrypted ngram index
(`crates/koushi-search/src/document.rs`). The history crawler
(`crates/koushi-core/src/search_crawler.rs`) reads `/messages` via
`room.messages()` and extracts visible text into the search pipeline; it does
**not** write the canonical SDK event cache. Search-result context open goes
through the timeline (focused), not the index.

**Existing diagnostics:** only `KOUSHI_SUBSCRIBE_TRACE=1` →
`app_loop_trace` in `crates/koushi-core/src/runtime.rs:71`, which logs AppActor
loop iterations slower than 100 ms. No startup/restore/subscribe/pagination
phase timing exists.

## 3. Decomposition of #123

- **Phase A — Observability (this spec).** Make the startup phases and load
  origin measurable on the real account, headless on Linux. Pure diagnostics;
  no product behavior change. Prerequisite for B. Satisfies the issue's
  short-term diagnostic request.
- **Phase B — The felt fix.** Make startup repaint the previous room's cached
  timeline instantly, informed by Phase A measurements. Candidate changes
  (to be designed in Phase B, not committed here): persist + restore the
  last-active room; render cached `InitialItems` without waiting on live sync;
  suppress the on-open auto-backfill network pagination when the cache already
  satisfies the viewport; put room-open / context-open on the priority path;
  reconsider non-preemptive crawler vs. user-load contention.
- **Phase C — Architecture.** Document the canonical event cache as the single
  source of truth, declare the search index a derived accelerator, and define
  one intent-based load API (`load older window`, `restore anchor`,
  `open event context`, `hydrate search result`) that reports origin. Satisfies
  acceptance criterion #1. The full code unification is incremental and lands
  against this north star.

Phase A's measurements decide B's concrete fixes, so B is intentionally not
pinned down here beyond candidates.

## 4. Phase A scope

### In scope

1. **Phase-timing instrumentation** (core, Rust), env-gated and
   private-data-free, following the existing `app_loop_trace` pattern
   (`runtime.rs:71`). Stamp begin/end of each startup→display phase and emit
   structured tokens with **durations and coarse buckets only**:
   - restore begin/end; crypto store open begin/end;
   - sync start → first `Ready`;
   - room-list first usable (rooms available for selection);
   - timeline `subscribe` begin/end + a **bucketed** initial item count
     (e.g. `0 / 1-10 / 11-50 / 51+`, never the exact count);
   - `TimelineBuilder::build` begin/end;
   - first auto-backfill / first backward pagination begin/end + outcome
     (`reached_start` bool, items-added bucket);
   - crawler page begin/end.
   Tokens MUST NOT contain room IDs, event IDs, user IDs, message bodies,
   timestamps that could fingerprint content, transaction IDs, or raw SDK
   errors. Correlate phases by the existing `RequestId` / `TimelineKey` where
   available, but do not emit those identifiers in tokens.

2. **Origin signal (cache / network / mixed), where the SDK exposes it.**
   Phase A first **discovers** what the vendored SDK actually exposes about
   whether a back-pagination / event-cache read hit the store or issued a
   network request (candidates to verify: timeline back-pagination status /
   `PaginationOutcome`, event-cache pagination return values, any store-hit
   signal). Then it surfaces a coarse origin token
   (`origin=cache|network|mixed|unknown`) for the load entry points.
   - If a clean origin signal requires a vendored-SDK delta, **do not** patch
     the SDK in Phase A. Record the gap in
     `docs/upstream/matrix-rust-sdk-feedback.md` and emit `origin=unknown` with
     a private-data-free reason. Phase A's job is to surface the gap, not to
     force a patch.

3. **Read-only real-account measurement lane** (Linux, headless). Reuse the
   `real-homeserver-qa` plumbing
   (`crates/koushi-core/src/bin/real-homeserver-qa.rs`,
   `scripts/desktop-real-homeserver-qa.mjs`): credentials from
   `.local-secrets/real-account-qa/credentials.json`, mandatory file credential
   store (`KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR`, refuses OS keychain),
   private-data-free token output, no display required. Differences from the
   existing write+cleanup smoke:
   - **Persistent data dir.** Use a stable, git-ignored data-dir root (e.g.
     under `.local-secrets/real-account-qa/profile/`) instead of the existing
     fresh `runs/<ts>/data`, so the SDK store + event cache + search index
     accumulate across runs. The event cache persists into
     `accounts/<hash>/cache/` via `store.rs:137-140`
     (`with_cache_path(account_cache_dir)`).
   - **Read-only.** No room create, no send/edit/redact, no leave/forget. Only
     login (run 1) / restore (run 2+), sync, room-list, and timeline
     build/subscribe/pagination of an **already-joined** room.
   - **Two-run shape.** Run 1: log in once (single device), sync to `Ready`,
     subscribe one **target room** (selected deterministically — e.g. the
     joined room with the most cached/recent history; the exact selection rule
     is a plan detail), optionally paginate back a bounded N pages to seed the
     cache, persist, shut down cleanly (drop SDK handles inside a Tokio runtime
     context per async rules #11/#12). Run 2+: cold process, `restore_session`
     from the persisted store, then subscribe the **same** target room and
     measure the Phase-A phases and origin tokens. No writes.
   - Implemented as a new scenario in `real-homeserver-qa` (e.g.
     `--scenario=startup_latency`) to inherit the existing credential/store
     guards and redaction self-checks.

4. **Diagnostic report.** A short, private-data-free summary of where the
   startup seconds go (per phase, with origin), written up to feed the Phase B
   spec. No real account data; durations and buckets only.

5. **Operational docs.** Add the new lane and its env to the AGENTS.md QA
   sections (search-path, how to run, expected tokens), consistent with the
   existing real-homeserver-qa notes.

### Out of scope (deferred to B / C)

- Any **product behavior change**: instant cached restore, last-room
  persistence/restore, auto-backfill suppression, priority-path changes,
  an `origin` field on the `TimelineItem` DTO, or the intent-based load API.
  Phase A only **observes**.
- Canonical-cache / search unification.
- A real-account GUI scenario. The webview "loading" felt symptom stays
  verified by local synthetic GUI scenarios plus (if needed) attended manual
  GUI; real-account post-login screenshots remain prohibited. Phase A locates
  the latency at the core boundary, which is sufficient to drive B.

## 5. Instrumentation design

- One core-owned, env-gated diagnostic switch (e.g. `KOUSHI_STARTUP_TRACE=1`),
  mirroring `app_loop_trace`: a small free function that no-ops unless the env
  var is set, emitting `eprintln!` tokens through the existing redacted
  diagnostic path. Native-only timing (`std::time::Instant`) behind the env
  gate is acceptable here because it matches the existing `app_loop_trace`
  pattern; keep it out of the wasm-clean pure crates (`koushi-state`,
  `koushi-search`).
- Token shape: stable `key=value` tokens, e.g.
  `koushi.startup phase=restore ms=NNN`,
  `koushi.startup phase=subscribe ms=NNN items=11-50 origin=cache`. Exact token
  grammar is finalized in the implementation plan; it must pass
  `npm --prefix apps/desktop run qa:secret-scan`.
- Phase boundaries are stamped at the actor sites named in §2 (account restore,
  sync→Ready, room-list, timeline build/subscribe/paginate, crawler page). No
  new control flow; only measurement.

## 6. QA / verification

Per the overview QA model (unit → local homeserver → real homeserver; GUI is
last and weakest):

- **Unit:** a test asserting the token formatter is private-data-free (rejects
  ids/bodies) and that buckets, not exact counts, are emitted. The
  secret-scan gate must pass.
- **Lane evidence:** the measurement lane's own private-data-free token output
  is the evidence — phases present, durations finite, an `origin` token emitted
  (`cache|network|mixed`) or `unknown` with a reason.
- **No GUI claim.** Phase A makes no native-GUI assertion.

The implementation follows headless-first / local-server-first: instrumentation
and the lane are exercised against the existing local Conduit/Tuwunel core QA
first (to prove the tokens emit and stay redacted), then against the real
account on Linux headless (with explicit maintainer GO, because run 1 performs a
real matrix.org login and creates one device).

## 7. Risks and open questions

- **SDK origin observability may be thin.** Mitigation: Phase A surfaces
  `origin=unknown` + a feedback-ledger entry rather than forcing a vendored
  patch. Whether a patch is worthwhile is a Phase B/C decision.
- **Persistent real-account profile at rest.** The reused profile dir holds an
  encrypted SDK store. Keep it under `.local-secrets/` (git-ignored), file
  credential backend only, never the OS keychain.
- **Device hygiene.** Run 1 creates one device on matrix.org; subsequent runs
  restore the same session (no new devices). Provide an explicit teardown step
  (logout + forget device) to run when finished so no stray device is left.
- **Reproduction fidelity.** The maintainer's profile has ~85k cached events;
  the QA profile must be seeded enough (sync + bounded backfill of a busy room)
  for run 2 to exercise a non-trivial cache. If the real account is sparse, the
  lane still measures the phase ordering, but the "seconds" magnitude may differ
  from the maintainer's daily profile; note this in the report.

## 8. Deliverables

1. Env-gated phase-timing instrumentation in `koushi-core` (no behavior change).
2. Coarse origin token at the load entry points (or `unknown` + feedback-ledger
   entry).
3. A read-only, persistent-profile `startup_latency` scenario in
   `real-homeserver-qa`.
4. A private-data-free unit test for token redaction.
5. A private-data-free diagnostic report feeding the Phase B spec.
6. AGENTS.md QA notes for the new lane.

## 9. Workflow

Per the maintainer's cadence: design (this spec, Opus) → implementation
delegated to a Sonnet agent against the approved plan → codex diff review
(AGENTS.md recipe). The main agent (Opus) owns integration points, the
real-account run GO, and independent gate verification; it does not accept a
masked subagent gate result as evidence. No real matrix.org login is run without
explicit maintainer GO.
