# Startup-Latency Observability (Phase A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the app-startup → first-timeline-paint path measurable on a real account, headless on Linux, without changing any product behavior — so we can see where the "seconds before the cached timeline appears" go (cache vs network, which phase) before fixing it in Phase B.

**Architecture:** Two complementary, private-data-free mechanisms. (1) A core-internal, env-gated phase trace (`KOUSHI_STARTUP_TRACE=1`) mirroring the existing `app_loop_trace` in `runtime.rs`, stamping sub-phases hidden inside single handlers (restore, timeline build vs subscribe, pagination gate-wait vs SDK paginate, crawler page) plus a cache/network/sync **origin** token sourced from the SDK's `EventsOrigin`. (2) A read-only `startup_latency` scenario in the existing `real-homeserver-qa` binary that times macro-phases by timestamping `CoreEvent` arrivals and runs against a **persistent** profile dir so a second (cold) launch restores a populated event cache. No product code path changes; every new observation is a no-op unless the env var is set or the scenario is selected.

**Tech Stack:** Rust (`koushi-core`), the vendored `matrix-rust-sdk` (`matrix-sdk` / `matrix-sdk-ui`), Node ESM QA runner (`scripts/desktop-real-homeserver-qa.mjs`), `cargo test`, `cargo run --features qa-bin`.

## Global Constraints

Every task's requirements implicitly include these (copied from the design spec
`docs/superpowers/specs/2026-06-23-canonical-event-cache-startup-latency-design.md`):

- **No product behavior change.** Phase A only observes. All new core observation is a no-op unless `KOUSHI_STARTUP_TRACE` is set; the new QA path runs only under the `startup_latency` scenario.
- **Private-data-free, always.** Emitted tokens carry **durations and coarse buckets only**. Never room IDs, event IDs, user IDs, message bodies, timestamps that could fingerprint content, transaction IDs, pagination tokens, or raw SDK errors.
- **Env gate name:** `KOUSHI_STARTUP_TRACE=1`, checked inline at each call site (mirror `app_loop_trace`, `crates/koushi-core/src/runtime.rs:71`).
- **Origin source:** the SDK `EventsOrigin` enum (`Sync` / `Pagination` / `Cache`) reached via `room.event_cache().await` → `.subscribe().await` → `RoomEventCacheUpdate::UpdateTimelineEvents(diffs)` → `diffs.origin`. **No vendored SDK patch.** Map to token text `sync` / `network` / `cache`.
- **wasm cleanliness:** the trace module uses `std::time::Instant`; it lives only in `koushi-core`, never in the wasm-clean pure crates `koushi-state` / `koushi-search`.
- **QA guards (reused):** `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` is mandatory; the binary refuses the OS keychain; it self-checks its transcript for password/recovery leakage; no GUI launch.
- **Async rules:** drop SDK handles inside a Tokio runtime context (overview async rules #11/#12); follow the existing background-task spawn pattern in `TimelineActor` rather than scattering `tokio::spawn`.
- **Real-account run requires explicit maintainer GO.** Run 1 performs a real matrix.org login and creates one device. Provide an explicit teardown (logout) for when measurement is finished.
- **Gate:** `npm --prefix apps/desktop run qa:secret-scan` must pass on every commit.

---

## File Structure

**Created:**
- `crates/koushi-core/src/startup_trace.rs` — the env-gated, private-data-free phase-trace module (phase enum, count bucketing, emit functions, origin mapping, unit tests). One responsibility: format and emit diagnostic tokens.
- `docs/qa/startup-latency-observability.md` — operator doc + private-data-free diagnostic-report template (filled in at run time, not with real data).

**Modified:**
- `crates/koushi-core/src/lib.rs` — declare `mod startup_trace;`.
- `crates/koushi-core/src/account.rs` — stamp `Restore` (and `CryptoOpen` if separable) around session restore.
- `crates/koushi-core/src/timeline.rs` — stamp `TimelineBuild` and `TimelineSubscribe` (+ initial item bucket) in `handle_subscribe`; stamp `PaginateGateWait` + `Paginate` (+ `reached_start`) in `handle_paginate`; spawn the env-gated origin observer.
- `crates/koushi-core/src/search_crawler.rs` — stamp `CrawlerPage` around the `/messages` page fetch.
- `crates/koushi-core/src/bin/real-homeserver-qa.rs` — add the read-only `StartupLatency` scenario (restore-or-login, macro-phase timing, target-room subscribe + bounded paginate, optional teardown).
- `scripts/desktop-real-homeserver-qa.mjs` — add a persistent-profile path + `startup_latency` orchestration (run1 populate / run2 measure) + token assertions.
- `AGENTS.md` — QA note for the new lane (how to run, expected tokens).

---

### Task 1: `startup_trace` diagnostic module

**Files:**
- Create: `crates/koushi-core/src/startup_trace.rs`
- Modify: `crates/koushi-core/src/lib.rs` (add `mod startup_trace;` next to the other `mod` declarations)
- Test: inline `#[cfg(test)] mod tests` in `crates/koushi-core/src/startup_trace.rs`

**Interfaces:**
- Produces (used by Tasks 2–4):
  - `pub(crate) enum StartupPhase { Restore, CryptoOpen, SyncToReady, RoomList, TimelineBuild, TimelineSubscribe, Paginate, PaginateGateWait, CrawlerPage }`
  - `pub(crate) fn enabled() -> bool`
  - `pub(crate) fn count_bucket(n: usize) -> &'static str`
  - `pub(crate) fn trace_phase(phase: StartupPhase, elapsed: std::time::Duration)`
  - `pub(crate) fn trace_phase_items(phase: StartupPhase, elapsed: std::time::Duration, items: usize)`
  - `pub(crate) fn trace_paginate(elapsed: std::time::Duration, gate_wait: std::time::Duration, reached_start: bool)`
  - `pub(crate) fn trace_origin(origin: &'static str)`

- [ ] **Step 1: Write the failing test**

Add to `crates/koushi-core/src/startup_trace.rs` (create the file with just the test + empty items first so it fails to compile/run):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_bucket_never_leaks_exact_counts() {
        assert_eq!(count_bucket(0), "0");
        assert_eq!(count_bucket(1), "1-10");
        assert_eq!(count_bucket(10), "1-10");
        assert_eq!(count_bucket(11), "11-50");
        assert_eq!(count_bucket(50), "11-50");
        assert_eq!(count_bucket(51), "51+");
        assert_eq!(count_bucket(85_850), "51+");
    }

    #[test]
    fn phase_tokens_are_stable_lowercase_identifiers() {
        for phase in [
            StartupPhase::Restore, StartupPhase::CryptoOpen, StartupPhase::SyncToReady,
            StartupPhase::RoomList, StartupPhase::TimelineBuild, StartupPhase::TimelineSubscribe,
            StartupPhase::Paginate, StartupPhase::PaginateGateWait, StartupPhase::CrawlerPage,
        ] {
            let token = phase.as_token();
            assert!(!token.is_empty());
            assert!(token.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "phase token must be a private-data-free lowercase identifier");
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib startup_trace`
Expected: FAIL — `count_bucket` / `StartupPhase` / `as_token` not found.

- [ ] **Step 3: Write the module**

Prepend to `crates/koushi-core/src/startup_trace.rs`:

```rust
//! Diagnostic-only, private-data-free startup / event-load phase tracing.
//!
//! Enabled with `KOUSHI_STARTUP_TRACE=1`. Mirrors `app_loop_trace`
//! (`runtime.rs`): a no-op unless the env var is set, emitting stable
//! `key=value` tokens via `eprintln!`. Tokens carry durations and coarse
//! buckets ONLY — never room/event/user ids, message bodies, timestamps,
//! transaction ids, or raw SDK errors (engineering-rules Secrets / QA
//! redaction). Phase A adds observation only; it changes no product behavior.

use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupPhase {
    Restore,
    CryptoOpen,
    SyncToReady,
    RoomList,
    TimelineBuild,
    TimelineSubscribe,
    Paginate,
    PaginateGateWait,
    CrawlerPage,
}

impl StartupPhase {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            StartupPhase::Restore => "restore",
            StartupPhase::CryptoOpen => "crypto_open",
            StartupPhase::SyncToReady => "sync_to_ready",
            StartupPhase::RoomList => "room_list",
            StartupPhase::TimelineBuild => "timeline_build",
            StartupPhase::TimelineSubscribe => "subscribe",
            StartupPhase::Paginate => "paginate",
            StartupPhase::PaginateGateWait => "paginate_gate_wait",
            StartupPhase::CrawlerPage => "crawler_page",
        }
    }
}

/// Coarse item-count bucket so exact event counts never leak.
pub(crate) fn count_bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1..=10 => "1-10",
        11..=50 => "11-50",
        _ => "51+",
    }
}

/// True when startup tracing is enabled. Cheap; checked at each call site.
pub(crate) fn enabled() -> bool {
    std::env::var_os("KOUSHI_STARTUP_TRACE").is_some()
}

pub(crate) fn trace_phase(phase: StartupPhase, elapsed: Duration) {
    if enabled() {
        eprintln!("koushi.startup phase={} ms={}", phase.as_token(), elapsed.as_millis());
    }
}

pub(crate) fn trace_phase_items(phase: StartupPhase, elapsed: Duration, items: usize) {
    if enabled() {
        eprintln!(
            "koushi.startup phase={} ms={} items={}",
            phase.as_token(),
            elapsed.as_millis(),
            count_bucket(items)
        );
    }
}

pub(crate) fn trace_paginate(elapsed: Duration, gate_wait: Duration, reached_start: bool) {
    if enabled() {
        eprintln!(
            "koushi.startup phase=paginate ms={} gate_ms={} reached_start={}",
            elapsed.as_millis(),
            gate_wait.as_millis(),
            reached_start
        );
    }
}

/// `origin` must be one of the fixed strings "cache", "network", or "sync"
/// (mapped from the SDK `EventsOrigin`). Caller passes a `&'static str` so no
/// dynamic content can leak.
pub(crate) fn trace_origin(origin: &'static str) {
    if enabled() {
        eprintln!("koushi.startup phase=origin origin={origin}");
    }
}
```

Then add `mod startup_trace;` to `crates/koushi-core/src/lib.rs` alongside the other `mod` declarations.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p koushi-core --lib startup_trace`
Expected: PASS (2 tests).

- [ ] **Step 5: Secret scan + commit**

Run: `npm --prefix apps/desktop run qa:secret-scan`
Expected: `secret scan ok`.

```bash
git add crates/koushi-core/src/startup_trace.rs crates/koushi-core/src/lib.rs
git commit -m "diag: add env-gated private-data-free startup_trace module (#123, Phase A)"
```

---

### Task 2: Instrument restore + timeline build/subscribe + pagination sub-phases

**Files:**
- Modify: `crates/koushi-core/src/account.rs` (the `handle_restore_session` path around `:2993`)
- Modify: `crates/koushi-core/src/timeline.rs` (`handle_subscribe` around `:832` build and `:1419` subscribe; `handle_paginate` `:1773-1823`)
- Test: inline `#[cfg(test)]` source-assertion tests in `timeline.rs` (mirroring `timeline_pagination_uses_account_wide_messages_backpressure` at `:6836`)

**Interfaces:**
- Consumes (Task 1): `startup_trace::{StartupPhase, trace_phase, trace_phase_items, trace_paginate}`.
- Produces: trace call sites that Task 4's scenario observes via stderr tokens.

- [ ] **Step 1: Write the failing source-assertion test**

Add to the `#[cfg(test)] mod tests` in `crates/koushi-core/src/timeline.rs`:

```rust
#[test]
fn timeline_subscribe_and_paginate_emit_startup_trace() {
    let source = include_str!("timeline.rs");

    let subscribe_src = source
        .split("async fn handle_subscribe")
        .nth(1)
        .and_then(|s| s.split("async fn ").next())
        .expect("handle_subscribe should exist");
    assert!(
        subscribe_src.contains("StartupPhase::TimelineBuild"),
        "subscribe must time the SDK TimelineBuilder::build phase"
    );
    assert!(
        subscribe_src.contains("StartupPhase::TimelineSubscribe"),
        "subscribe must time the event-cache subscribe phase with an item bucket"
    );

    let paginate_src = source
        .split("async fn handle_paginate")
        .nth(1)
        .and_then(|s| s.split("async fn handle_send_text").next())
        .expect("handle_paginate should exist");
    assert!(
        paginate_src.contains("trace_paginate"),
        "pagination must emit a startup_trace paginate token (gate-wait + duration + reached_start)"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib timeline_subscribe_and_paginate_emit_startup_trace`
Expected: FAIL — strings not present.

- [ ] **Step 3: Add the instrumentation**

In `crates/koushi-core/src/timeline.rs`, add `use crate::startup_trace::{self, StartupPhase};` to the module imports.

In `handle_subscribe`, wrap the build (`:832`) and subscribe (`:1419`):

```rust
// around TimelineBuilder::new(&room).build()
let build_started = std::time::Instant::now();
let timeline_result = matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
    // ... existing builder chain ...
    .build()
    .await;
startup_trace::trace_phase(StartupPhase::TimelineBuild, build_started.elapsed());
```

```rust
// around timeline.subscribe().await
let subscribe_started = std::time::Instant::now();
let (initial_sdk_items, diff_stream) = timeline.subscribe().await;
startup_trace::trace_phase_items(
    StartupPhase::TimelineSubscribe,
    subscribe_started.elapsed(),
    initial_sdk_items.len(),
);
```

In `handle_paginate`, replace the `let result = { ... }` block (`:1798-1806`) to time the gate wait and the SDK call separately:

```rust
let gate_started = std::time::Instant::now();
let result = {
    let _permit = self.messages_backpressure.acquire_timeline().await;
    let gate_wait = gate_started.elapsed();
    let paginate_started = std::time::Instant::now();
    let outcome = match direction {
        PaginationDirection::Backward => self.timeline.paginate_backwards(event_count).await,
        PaginationDirection::Forward => self.timeline.paginate_forwards(event_count).await,
    };
    startup_trace::trace_paginate(
        paginate_started.elapsed(),
        gate_wait,
        matches!(outcome, Ok(true)),
    );
    outcome
};
```

In `crates/koushi-core/src/account.rs`, add `use crate::startup_trace::{self, StartupPhase};` and wrap the restore body in `handle_restore_session` (around `:2993`):

```rust
let restore_started = std::time::Instant::now();
// ... existing restore body up to the point a store-backed session exists ...
startup_trace::trace_phase(StartupPhase::Restore, restore_started.elapsed());
```

If a distinct crypto-store-open call is visible inside the restore body, wrap just that call with `StartupPhase::CryptoOpen` the same way; if it is not separable from `restore_session_with_store`, leave only `Restore` and note it in the commit message (do not invent a boundary).

- [ ] **Step 4: Run test + build to verify**

Run: `cargo test -p koushi-core --lib timeline_subscribe_and_paginate_emit_startup_trace`
Expected: PASS.
Run: `cargo build -p koushi-core`
Expected: builds clean (no behavior change; trace calls are no-ops unless env set).

- [ ] **Step 5: Secret scan + commit**

Run: `npm --prefix apps/desktop run qa:secret-scan`

```bash
git add crates/koushi-core/src/timeline.rs crates/koushi-core/src/account.rs
git commit -m "diag: time restore + timeline build/subscribe/paginate sub-phases (#123, Phase A)"
```

---

### Task 3: Env-gated cache/network/sync origin observer

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs` (`handle_subscribe`; add an origin-observer background task, spawned only when `startup_trace::enabled()`, modeled on the existing diff-relay / send-queue-monitor background tasks)
- Test: inline source-assertion test in `timeline.rs`

**Interfaces:**
- Consumes (Task 1): `startup_trace::{enabled, trace_origin}`.
- SDK APIs (verified in vendored SDK): `room.event_cache().await` → `(RoomEventCache, _)`; `room_event_cache.subscribe().await` → subscriber stream of `RoomEventCacheUpdate`; `RoomEventCacheUpdate::UpdateTimelineEvents(TimelineVectorDiffs { origin, .. })`; `EventsOrigin::{Cache, Pagination, Sync}`.

- [ ] **Step 1: Write the failing source-assertion test**

```rust
#[test]
fn timeline_subscribe_spawns_env_gated_origin_observer() {
    let source = include_str!("timeline.rs");
    let subscribe_src = source
        .split("async fn handle_subscribe")
        .nth(1)
        .and_then(|s| s.split("async fn ").next())
        .expect("handle_subscribe should exist");
    assert!(
        subscribe_src.contains("startup_trace::enabled()"),
        "origin observer must be gated on KOUSHI_STARTUP_TRACE so production is unaffected"
    );
    assert!(
        subscribe_src.contains("event_cache()"),
        "origin observer must subscribe the SDK room event cache"
    );
    assert!(
        subscribe_src.contains("EventsOrigin"),
        "origin observer must read the SDK EventsOrigin (cache/network/sync)"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib timeline_subscribe_spawns_env_gated_origin_observer`
Expected: FAIL.

- [ ] **Step 3: Add the env-gated observer**

In `handle_subscribe`, after the SDK timeline + room handle are available, add (modeled on the crate's existing `run_diff_relay` background-task spawn — use the same spawn mechanism the actor already uses, and store/abort the handle on unsubscribe alongside the existing relay handles):

```rust
if startup_trace::enabled() {
    let observer_room = room.clone();
    // Use the same task-spawn + handle-tracking pattern as run_diff_relay so
    // the handle is aborted on unsubscribe / actor shutdown (async rules #7,#12).
    let origin_task = /* existing spawn helper */ async move {
        if let Ok((cache, _drop_guards)) = observer_room.event_cache().await {
            if let Ok((_initial, mut updates)) = cache.subscribe().await {
                use matrix_sdk::event_cache::{EventsOrigin, RoomEventCacheUpdate};
                while let Some(update) = updates.recv().await.ok() {
                    if let RoomEventCacheUpdate::UpdateTimelineEvents(diffs) = update {
                        let origin = match diffs.origin {
                            EventsOrigin::Cache => "cache",
                            EventsOrigin::Pagination => "network",
                            EventsOrigin::Sync => "sync",
                        };
                        startup_trace::trace_origin(origin);
                    }
                }
            }
        }
    };
    // store `origin_task` handle next to the existing relay handles for cleanup
}
```

Note for the implementer: match the exact stream API the vendored SDK exposes for the subscriber (`recv()` vs `next()`), the exact `event_cache()` return tuple, and the actor's real spawn/handle-tracking helper — read `run_diff_relay` and the unsubscribe path first. The observer must be abortable on unsubscribe and must never emit anything but the three fixed origin strings.

- [ ] **Step 4: Run test + build**

Run: `cargo test -p koushi-core --lib timeline_subscribe_spawns_env_gated_origin_observer`
Expected: PASS.
Run: `cargo build -p koushi-core`
Expected: clean.

- [ ] **Step 5: Secret scan + commit**

```bash
git add crates/koushi-core/src/timeline.rs
git commit -m "diag: env-gated cache/network/sync origin observer via SDK EventsOrigin (#123, Phase A)"
```

---

### Task 4: Instrument crawler page fetch

**Files:**
- Modify: `crates/koushi-core/src/search_crawler.rs` (the page runner around `:121`, where `acquire_crawler().await` precedes `room.messages(...)`)
- Test: inline source-assertion test in `search_crawler.rs` (mirroring `history_crawler_page_runner_acquires_crawler_messages_backpressure` at `:607`)

**Interfaces:**
- Consumes (Task 1): `startup_trace::{StartupPhase, trace_phase}`.

- [ ] **Step 1: Write the failing source-assertion test**

```rust
#[test]
fn crawler_page_emits_startup_trace() {
    let source = include_str!("search_crawler.rs");
    assert!(
        source.contains("StartupPhase::CrawlerPage"),
        "crawler page fetch must be timed so background /messages work is visible in startup traces"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib crawler_page_emits_startup_trace`
Expected: FAIL.

- [ ] **Step 3: Add the instrumentation**

In `crates/koushi-core/src/search_crawler.rs`, add `use crate::startup_trace::{self, StartupPhase};` and wrap the page fetch (around `:121`):

```rust
let _permit = messages_backpressure.acquire_crawler().await;
let page_started = std::time::Instant::now();
let messages_result = room.messages(options).await;
startup_trace::trace_phase(StartupPhase::CrawlerPage, page_started.elapsed());
```

(Keep the existing variable name for the messages result; only add the `Instant` + `trace_phase` lines.)

- [ ] **Step 4: Run test + build**

Run: `cargo test -p koushi-core --lib crawler_page_emits_startup_trace`
Expected: PASS.
Run: `cargo build -p koushi-core`
Expected: clean.

- [ ] **Step 5: Secret scan + commit**

```bash
git add crates/koushi-core/src/search_crawler.rs
git commit -m "diag: time background crawler /messages page fetch (#123, Phase A)"
```

---

### Task 5: Read-only `startup_latency` scenario in `real-homeserver-qa`

**Files:**
- Modify: `crates/koushi-core/src/bin/real-homeserver-qa.rs` (add `StartupLatency` to `RealQaScenario` `:88` + `from_env_value` `:100`; add the scenario body + a `--teardown`/env hook)
- Test: inline `#[cfg(test)]` test for `RealQaScenario::from_env_value` parsing the new value

**Interfaces:**
- Consumes: existing helpers `wait_for_logged_in`, `wait_for_post_login_ready_snapshot`, `wait_for_sync_running`, `wait_for_non_empty_room_list`, `conn.recv_event()`, `EVENT_TIMEOUT`; `TimelineCommand::Subscribe` / `Paginate`; `TimelineEvent::{InitialItems, PaginationStateChanged}`.
- Produces: stderr/stdout macro-phase tokens `startup_lat phase=<restore|sync_to_ready|room_list|subscribe> ms=<NNN>` and `startup_lat phase=paginate ms=<NNN> reached_start=<bool>`. Durations only; no ids.

**Scenario behavior (read-only):**
1. Attempt `RestoreLastSession`. On `SessionNotFound`, fall back to credentials login (this is run 1; the persistent dir is empty). Otherwise restore (run 2+, cold start over the populated store/event-cache).
2. Time and emit: restore→Ready, sync Start→Running/Ready, room-list first non-empty.
3. Select a **target room deterministically** from the room-list snapshot (e.g. the first joined non-DM room in the snapshot's stable order; document the rule in a code comment). Send `TimelineCommand::Subscribe`; time until `InitialItems`; emit `subscribe`.
4. Issue a bounded number of backward `Paginate` commands (e.g. `STARTUP_LAT_PAGES`, default 3); time each until its terminal `PaginationStateChanged` (`Idle`/`EndReached`/`Failed`); emit `paginate ms=… reached_start=…`. Stop early on `EndReached`.
5. **No writes**: never create rooms, send, edit, redact, react, leave, or forget. The target room is only subscribed and paginated.
6. Teardown: by default **keep the session** (so run 2 can restore). When `KOUSHI_STARTUP_LAT_TEARDOWN=1`, log out at the end (used by the operator to remove the QA device when finished). Honor the existing logout-cleanup-on-failure rule only for the teardown path.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(any(debug_assertions, test))]
#[test]
fn startup_latency_scenario_parses_from_env() {
    assert_eq!(
        RealQaScenario::from_env_value(Some("startup_latency".to_owned())),
        Ok(RealQaScenario::StartupLatency)
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --features qa-bin --bin real-homeserver-qa startup_latency_scenario_parses_from_env`
Expected: FAIL — `StartupLatency` variant missing.

- [ ] **Step 3: Add the variant + parsing + scenario dispatch**

Add `StartupLatency` to the `RealQaScenario` enum; in `from_env_value` add `Some("startup_latency") => Ok(Self::StartupLatency)`. Add a `is_startup_latency`/match arm in the run dispatch that calls a new `run_startup_latency_scenario(conn, creds, ...)` implementing the behavior above using the existing wait helpers and `std::time::Instant` deltas. Emit only the documented `startup_lat …` duration tokens. Guard all of it behind the existing `#[cfg(any(debug_assertions, test))]`.

- [ ] **Step 4: Run the parsing test + build the bin**

Run: `cargo test -p koushi-core --features qa-bin --bin real-homeserver-qa startup_latency_scenario_parses_from_env`
Expected: PASS.
Run: `cargo build -p koushi-core --features qa-bin --bin real-homeserver-qa`
Expected: clean.

- [ ] **Step 5: Secret scan + commit**

```bash
git add crates/koushi-core/src/bin/real-homeserver-qa.rs
git commit -m "qa: read-only startup_latency real-account scenario (macro-phase timing) (#123, Phase A)"
```

---

### Task 6: Runner orchestration (persistent profile, run1/run2) + operator docs

**Files:**
- Modify: `scripts/desktop-real-homeserver-qa.mjs` (persistent-profile dir + `startup_latency` orchestration + token assertions; reuse `checkForLeaks`, `assertNoMatrixIdentifiers`, `assertNoLocalPaths`, `assertNoRawSdkErrors`)
- Create: `docs/qa/startup-latency-observability.md`
- Modify: `AGENTS.md` (add a "Startup Latency Observability QA" note)
- Test: manual lane run (evidence-based); plus a Node assertion that run 2 output contains the required tokens.

**Interfaces:**
- Sets env for the binary: `KOUSHI_REAL_QA_SCENARIO=startup_latency`, `KOUSHI_STARTUP_TRACE=1`, `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR=<persistent cred-store>`, `KOUSHI_QA_DATA_DIR=<persistent data dir>`.

- [ ] **Step 1: Add the persistent-profile branch**

When `--scenario=startup_latency`, replace the fresh `runs/<ts>` dirs with a stable profile under `.local-secrets/real-account-qa/profile/startup_latency/` (git-ignored: `.local-secrets/` is already in `.gitignore`):

```js
const isStartupLatency = scenarioOption === "startup_latency";
const profileDir = join(realAccountDir, "profile", "startup_latency");
const dataDir = isStartupLatency ? join(profileDir, "data") : join(runDir, "data");
const credStoreDir = isStartupLatency ? join(profileDir, "cred-store") : join(runDir, "cred-store");
mkdirSync(dataDir, { recursive: true });
mkdirSync(credStoreDir, { recursive: true });
```

Add `KOUSHI_STARTUP_TRACE: "1"` to `env` for this scenario.

- [ ] **Step 2: Orchestrate run1 (populate) → run2 (measure)**

For `startup_latency`, invoke the binary twice against the same persistent dir: run 1 logs in and seeds the cache; run 2 restores cold and is the measured run. Capture both transcripts; run the existing redaction checks (`checkForLeaks`, `assertNoMatrixIdentifiers`, `assertNoLocalPaths`, `assertNoRawSdkErrors`) on BOTH before writing any artifact. Treat run 2 as the evidence run.

- [ ] **Step 3: Assert the measured run's tokens**

```js
const required = ["startup_lat phase=restore", "startup_lat phase=sync_to_ready",
  "startup_lat phase=room_list", "startup_lat phase=subscribe", "koushi.startup phase=origin"];
for (const token of required) {
  if (!run2Output.includes(token)) {
    throw new Error(`startup_latency run 2 missing required token: ${token}`);
  }
}
```

(`koushi.startup phase=origin` proves the env-gated origin observer fired. The `origin=` value will be `cache` for a warm restore that paints from disk, `network` where a `/messages` gap is filled.)

- [ ] **Step 4: Operator doc + AGENTS.md note**

Create `docs/qa/startup-latency-observability.md` documenting: purpose, the persistent-profile two-run model, exact command, the env vars, the expected private-data-free tokens, the teardown command (`KOUSHI_STARTUP_LAT_TEARDOWN=1`), and a **template table** (phase | ms | origin) to fill in with run output — no real data committed. Add a short AGENTS.md "Startup Latency Observability QA" subsection pointing at it and at this plan.

- [ ] **Step 5: Verify (with maintainer GO) + commit**

Real-account run requires explicit maintainer GO (run 1 = real matrix.org login + 1 device). With GO:

Run: `npm --prefix apps/desktop run qa:real-homeserver -- --scenario=startup_latency`
Expected: exit 0; run-2 transcript contains the required tokens; redaction checks pass; the run log shows a per-phase `ms=` breakdown and at least one `phase=origin origin=cache`.

```bash
git add scripts/desktop-real-homeserver-qa.mjs docs/qa/startup-latency-observability.md AGENTS.md
git commit -m "qa: persistent-profile startup_latency runner + operator doc (#123, Phase A)"
```

---

## Self-Review

**1. Spec coverage** (against `2026-06-23-...-design.md` §4 In scope):
- §4.1 phase-timing instrumentation → Tasks 2 (restore/build/subscribe/paginate) + 4 (crawler) + 5 (macro phases in the scenario). ✓
- §4.2 origin (cache/network/mixed) where SDK exposes it → Task 3 (`EventsOrigin`, no patch). "mixed" surfaces as multiple `origin=` tokens within one load; documented in Task 6. ✓
- §4.3 read-only real-account lane, persistent dir, run1/run2 → Tasks 5 + 6. ✓
- §4.4 diagnostic report → Task 6 doc template. ✓
- §4.5 AGENTS.md note → Task 6. ✓
- Out-of-scope items (behavior changes, DTO origin field, intent API, search unification, real-account GUI) → none introduced. ✓

**2. Placeholder scan:** Task 3's observer intentionally points the implementer to the real spawn/stream API ("read `run_diff_relay` first") rather than guessing the exact handle-tracking lines, because those are codebase-specific and verified at edit time; the required behavior, gating, APIs, and test are concrete. No "TBD"/"add error handling"/"write tests for the above" left. ✓

**3. Type consistency:** `StartupPhase` variants and the `trace_*` signatures defined in Task 1 are used verbatim in Tasks 2–4. Token strings (`startup_lat phase=…`, `koushi.startup phase=origin`) match between Task 5 (emit) and Task 6 (assert). `RealQaScenario::StartupLatency` defined and used consistently in Task 5. ✓

---

## Execution Handoff

Per the maintainer's workflow, **implementation is delegated to Sonnet**, with Opus reviewing between tasks and codex doing the final diff review. That maps to **subagent-driven execution with `model: sonnet` subagents**: a fresh Sonnet subagent per task, Opus verifies each task's gates independently (not trusting masked subagent output) before the next task, and the real-account run (Task 6 Step 5) only proceeds with explicit maintainer GO.
