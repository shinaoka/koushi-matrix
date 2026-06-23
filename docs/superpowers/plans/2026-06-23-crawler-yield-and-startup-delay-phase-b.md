# Crawler Yield + Startup Delay (Phase B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the background history crawler from blocking user-driven `/messages` loads at startup, so a user pagination never waits on the crawler (`gate_ms` ≈ 0) — fixing the Phase-A-proven startup "loading" — while keeping the account-wide concurrency-1 rate-limit protection.

**Architecture:** Three actor/runtime-level changes in `koushi-core`. (1) `MessagesBackpressure` stays concurrency-1 but gains a per-acquire cancellation signal: when a user-visible (`Timeline`) request needs the gate while a crawler holds it, the gate cancels the crawler. (2) `run_history_crawl_page` runs its `/messages` call in a `select!` against the permit's cancellation, returning a new `Preempted` result on cancel. (3) `SearchActor` re-queues a preempted checkpoint and, separately, delays *automatic* crawl-page starts by ~60s after the crawler first has work. No reducer/DTO/AppState/CoreEvent/React change.

**Tech Stack:** Rust (`koushi-core`): `tokio::sync::Notify`, `tokio::select!`, the crate's `executor::{spawn,sleep}` abstraction, `cargo test`.

## Global Constraints

Copied from the spec `docs/superpowers/specs/2026-06-23-crawler-yield-and-startup-delay-phase-b-design.md`:

- **Keep account-wide concurrency-1.** Do NOT introduce a second concurrent `/messages` lane. The fix is cancellation + delay, not parallelism.
- **Actor/runtime level only.** No `koushi-state` reducer, no `AppState` field, no Tauri DTO, no TypeScript, no React, no `CoreEvent` variant.
- **Crawler timing is Rust-owned.** `CRAWLER_STARTUP_DELAY` is a core constant, not a user-facing setting.
- **Preserve existing gate behavior:** timeline priority for *new* acquisitions (`waiting_timeline`), and a dropped/cancelled timeline waiter must not starve the crawler. The existing tests in `messages_backpressure.rs` must still pass.
- **Manual crawls bypass the startup delay.** Only automatic (`checkpoint.manual == false`) crawl-page starts are delayed; explicit `StartHistoryCrawl` (manual) checkpoints start immediately.
- **Executor abstraction for timers.** Use `crate::executor::{spawn,sleep}`; no raw `tokio::time` in actor logic.
- **Diagnostics are private-data-free** and reuse the Phase A `startup_trace` module (env-gated by `KOUSHI_STARTUP_TRACE`). No room/event/user ids.
- **Gate:** `cargo build -p koushi-core` warning-free and `npm --prefix apps/desktop run qa:secret-scan` pass on every commit.

---

## File Structure

**Modified:**
- `crates/koushi-core/src/messages_backpressure.rs` — add per-acquire crawler cancellation; `acquire_timeline` signals it; `MessagesRequestPermit::cancelled()`.
- `crates/koushi-core/src/search_crawler.rs` — `run_history_crawl_page` selects on cancellation; new `HistoryCrawlPageResult::Preempted`; emit the preempt diagnostic.
- `crates/koushi-core/src/startup_trace.rs` — add `trace_crawler_preempted()`.
- `crates/koushi-core/src/search.rs` — `SearchActor`: handle `Preempted` (re-queue front); add the ~60s automatic-crawl startup delay (fields + select-loop timer arm + gate in `start_next_history_crawl_page`).

**Verification (controller, GO-gated):** re-run the Phase A `startup_latency` lane.

---

### Task 1: Cancellation in `MessagesBackpressure`

**Files:**
- Modify: `crates/koushi-core/src/messages_backpressure.rs`
- Test: inline `#[cfg(test)] mod tests` in the same file (alongside the existing gate tests)

**Interfaces:**
- Consumes: nothing.
- Produces (used by Task 2): `MessagesRequestPermit::cancelled(&self) -> impl Future<Output = ()>` — resolves when a `Timeline` acquirer has requested preemption of this (crawler) permit. For a `Timeline` permit it never resolves.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block:

```rust
#[tokio::test]
async fn timeline_acquire_cancels_active_crawler_permit() {
    let gate = MessagesBackpressure::default();
    let crawler = gate.acquire_crawler().await;

    // A timeline acquire while the crawler holds the permit must signal the
    // crawler to yield: its `cancelled()` future resolves.
    let timeline_gate = gate.clone();
    let timeline = tokio::spawn(async move {
        let _permit = timeline_gate.acquire_timeline().await;
    });
    tokio::task::yield_now().await;

    tokio::time::timeout(Duration::from_secs(1), crawler.cancelled())
        .await
        .expect("active crawler must be cancelled when a timeline acquire is waiting");

    // Releasing the crawler lets the timeline acquire proceed.
    drop(crawler);
    tokio::time::timeout(Duration::from_secs(1), timeline)
        .await
        .expect("timeline must acquire after the crawler yields")
        .expect("timeline task should finish");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib timeline_acquire_cancels_active_crawler_permit`
Expected: FAIL — `cancelled` method does not exist.

- [ ] **Step 3: Implement cancellation**

In `crates/koushi-core/src/messages_backpressure.rs`:

Add `use std::sync::Arc;` (already present) and extend the state + permit:

```rust
#[derive(Debug, Default)]
struct MessagesBackpressureState {
    active: bool,
    /// True while the active permit is held by a crawler request.
    active_is_crawler: bool,
    /// Cancellation signal for the currently-active crawler permit, taken by a
    /// waiting timeline acquirer to make the crawler yield mid-page.
    active_crawler_cancel: Option<Arc<Notify>>,
    waiting_timeline: u64,
}
```

```rust
#[must_use]
pub(crate) struct MessagesRequestPermit {
    inner: Arc<MessagesBackpressureInner>,
    /// `Some` for crawler permits — resolves when a timeline acquirer requests
    /// preemption. `None` for timeline permits (never cancelled).
    cancel: Option<Arc<Notify>>,
}

impl MessagesRequestPermit {
    /// Resolves when a timeline acquirer has asked this crawler permit to yield.
    /// For a timeline permit this never resolves.
    pub(crate) async fn cancelled(&self) {
        match &self.cancel {
            Some(cancel) => cancel.notified().await,
            None => std::future::pending::<()>().await,
        }
    }
}
```

Update `acquire` to set/clear the crawler tracking and to signal preemption from the timeline path:

```rust
async fn acquire(&self, priority: MessagesRequestPriority) -> MessagesRequestPermit {
    let mut timeline_slot = match priority {
        MessagesRequestPriority::Timeline => Some(WaitingTimelineSlot::new(self.inner.clone())),
        MessagesRequestPriority::Crawler => None,
    };

    loop {
        let notified = self.inner.notify.notified();
        {
            let mut state = lock_state(&self.inner);
            let can_acquire = !state.active
                && (priority == MessagesRequestPriority::Timeline
                    || state.waiting_timeline == 0);
            if can_acquire {
                if let Some(slot) = timeline_slot.take() {
                    slot.finish(&mut state);
                }
                state.active = true;
                let cancel = match priority {
                    MessagesRequestPriority::Crawler => {
                        let cancel = Arc::new(Notify::new());
                        state.active_is_crawler = true;
                        state.active_crawler_cancel = Some(cancel.clone());
                        Some(cancel)
                    }
                    MessagesRequestPriority::Timeline => {
                        state.active_is_crawler = false;
                        None
                    }
                };
                return MessagesRequestPermit {
                    inner: self.inner.clone(),
                    cancel,
                };
            }
            // Timeline is blocked behind an active crawler: ask it to yield.
            if priority == MessagesRequestPriority::Timeline
                && state.active
                && state.active_is_crawler
            {
                if let Some(cancel) = &state.active_crawler_cancel {
                    cancel.notify_one();
                }
            }
        }
        notified.await;
    }
}
```

Update the permit `Drop` to clear the crawler tracking:

```rust
impl Drop for MessagesRequestPermit {
    fn drop(&mut self) {
        {
            let mut state = lock_state(&self.inner);
            state.active = false;
            state.active_is_crawler = false;
            state.active_crawler_cancel = None;
        }
        self.inner.notify.notify_waiters();
    }
}
```

(`notify_one` stores a permit if the crawler has not yet registered `cancelled()`, so the signal is race-free.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p koushi-core --lib messages_backpressure`
Expected: PASS — the new test plus the existing `crawler_waiting_first_yields_to_timeline_waiter` and `waiting_timeline_drop_does_not_starve_crawler`.

- [ ] **Step 5: Build + secret scan + commit**

Run: `cargo build -p koushi-core` (warning-free) and `npm --prefix apps/desktop run qa:secret-scan`.

```bash
git add crates/koushi-core/src/messages_backpressure.rs
git commit -m "feat: add crawler permit cancellation to MessagesBackpressure (#123, Phase B)"
```

---

### Task 2: Preempt the in-flight crawler page + `Preempted` result + diagnostic

**Files:**
- Modify: `crates/koushi-core/src/search_crawler.rs` (`run_history_crawl_page`, `HistoryCrawlPageResult`)
- Modify: `crates/koushi-core/src/startup_trace.rs` (add `trace_crawler_preempted`)
- Test: inline source-assertion test (production-scoped) in `search_crawler.rs`; token test in `startup_trace.rs`

**Interfaces:**
- Consumes (Task 1): `MessagesRequestPermit::cancelled()`.
- Produces (used by Task 3): `HistoryCrawlPageResult::Preempted { checkpoint: HistoryCrawlCheckpoint }`.

- [ ] **Step 1: Write the failing source-assertion test**

Add to `#[cfg(test)] mod tests` in `search_crawler.rs`:

```rust
#[test]
fn crawler_page_yields_to_timeline_via_cancellation() {
    let source = include_str!("search_crawler.rs");
    let production = source.split("\nmod tests").next().unwrap_or(source);
    assert!(
        production.contains("permit.cancelled()"),
        "crawler page must race room.messages() against the gate's cancellation signal"
    );
    assert!(
        production.contains("HistoryCrawlPageResult::Preempted"),
        "a cancelled crawler page must return Preempted so the checkpoint is re-queued"
    );
    assert!(
        production.contains("trace_crawler_preempted"),
        "preemption must be observable via startup_trace"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib crawler_page_yields_to_timeline_via_cancellation`
Expected: FAIL.

- [ ] **Step 3a: Add the diagnostic to `startup_trace.rs`**

```rust
/// Emitted when a background crawler page yields the /messages gate to a
/// user-visible pagination (preemption). Private-data-free.
pub(crate) fn trace_crawler_preempted() {
    if enabled() {
        eprintln!("koushi.startup phase=crawler_preempted");
    }
}
```

- [ ] **Step 3b: Add the `Preempted` variant**

In `search_crawler.rs`, extend the enum:

```rust
pub(crate) enum HistoryCrawlPageResult {
    Success {
        checkpoint: HistoryCrawlCheckpoint,
        messages: Vec<SearchIndexMessage>,
        completed: bool,
    },
    Failed {
        checkpoint: HistoryCrawlCheckpoint,
        kind: SearchCrawlerFailureKind,
    },
    /// The gate cancelled this page so a user-visible pagination could run.
    /// The checkpoint is unchanged and must be re-queued (no progress lost).
    Preempted {
        checkpoint: HistoryCrawlCheckpoint,
    },
}
```

- [ ] **Step 3c: Race `room.messages()` against cancellation**

Replace the `let messages = { ... }` block (currently `let _permit = messages_backpressure.acquire_crawler().await; ... match room.messages(options).await { ... }`) with:

```rust
    let messages = {
        let permit = messages_backpressure.acquire_crawler().await;
        let page_started = startup_trace::now_if_enabled();
        let page_result = tokio::select! {
            biased;
            // A waiting timeline pagination cancels the crawler: yield the gate
            // immediately and re-queue this checkpoint (no progress lost).
            _ = permit.cancelled() => {
                startup_trace::trace_crawler_preempted();
                return HistoryCrawlPageResult::Preempted { checkpoint };
            }
            result = room.messages(options) => result,
        };
        startup_trace::trace_phase(StartupPhase::CrawlerPage, page_started);
        match page_result {
            Ok(messages) => messages,
            Err(_) => {
                return HistoryCrawlPageResult::Failed {
                    checkpoint,
                    kind: SearchCrawlerFailureKind::Sdk,
                };
            }
        }
    };
```

Note: dropping the `room.messages()` future on the cancel branch aborts the in-flight request. Confirm against the vendored SDK that this leaves no partial event-cache write (the response — and any cache write — is applied only after the await resolves). If that assumption is wrong, STOP and report — the fallback is the cooperative variant in the spec.

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p koushi-core --lib crawler_page_yields_to_timeline_via_cancellation`
Expected: PASS.
Run: `cargo build -p koushi-core` → warning-free (note: `Preempted` is consumed in Task 3; a temporary non-exhaustive-match warning in `search.rs` is expected until Task 3 lands — if the build treats it as an error, add the arm in Task 3 before relying on a clean build, and state this in the report).

- [ ] **Step 5: Secret scan + commit**

```bash
git add crates/koushi-core/src/search_crawler.rs crates/koushi-core/src/startup_trace.rs
git commit -m "feat: preempt in-flight crawler page for user paginations (#123, Phase B)"
```

---

### Task 3: Re-queue preempted checkpoints in `SearchActor`

**Files:**
- Modify: `crates/koushi-core/src/search.rs` (`handle_history_crawl_page_result`)
- Test: inline source-assertion test (production-scoped) in `search.rs`

**Interfaces:**
- Consumes (Task 2): `HistoryCrawlPageResult::Preempted { checkpoint }`.

- [ ] **Step 1: Write the failing source-assertion test**

```rust
#[test]
fn preempted_crawl_page_is_requeued() {
    let source = include_str!("search.rs");
    let production = source.split("\nmod tests").next().unwrap_or(source);
    let handler = production
        .split("fn handle_history_crawl_page_result")
        .nth(1)
        .and_then(|s| s.split("\n    fn ").next())
        .expect("handle_history_crawl_page_result should exist");
    assert!(
        handler.contains("HistoryCrawlPageResult::Preempted"),
        "the result handler must handle Preempted"
    );
    assert!(
        handler.contains("push_front"),
        "a preempted checkpoint must be re-queued at the front (no history lost)"
    );
}
```

(If `search.rs` has no `\nmod tests` marker, anchor the production slice on the test module's actual declaration; confirm `mod tests` occurs once.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib preempted_crawl_page_is_requeued`
Expected: FAIL.

- [ ] **Step 3: Handle `Preempted` in the result handler**

In `handle_history_crawl_page_result`, add an arm (drop stale-generation results like the `Success` arm does, then re-queue at the front so it is retried after the timeline load):

```rust
            HistoryCrawlPageResult::Preempted { checkpoint } => {
                if checkpoint.settings_generation != self.crawl_settings_generation {
                    return;
                }
                if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
                    return;
                }
                // No progress was made; retry this checkpoint next. The crawler's
                // next acquire blocks behind the waiting timeline (waiting_timeline),
                // so this does not livelock.
                self.queued_crawl_rooms.insert(checkpoint.room_id.clone());
                self.crawl_queue.push_front(checkpoint);
            }
```

- [ ] **Step 4: Run test + build**

Run: `cargo test -p koushi-core --lib preempted_crawl_page_is_requeued`
Expected: PASS.
Run: `cargo build -p koushi-core` → warning-free (the `Preempted` match is now exhaustive).

- [ ] **Step 5: Secret scan + commit**

```bash
git add crates/koushi-core/src/search.rs
git commit -m "feat: re-queue preempted crawl checkpoints (#123, Phase B)"
```

---

### Task 4: Automatic-crawl startup delay in `SearchActor`

**Files:**
- Modify: `crates/koushi-core/src/search.rs` (const, `SearchActor` fields + `spawn` init, `run` select-loop arm, `start_next_history_crawl_page`)
- Test: inline source-assertion test (production-scoped) in `search.rs`

**Interfaces:**
- Consumes: nothing (independent of Tasks 1–3).
- Produces: nothing consumed by other tasks.

- [ ] **Step 1: Write the failing source-assertion test**

```rust
#[test]
fn automatic_crawl_starts_are_delayed_at_startup() {
    let source = include_str!("search.rs");
    let production = source.split("\nmod tests").next().unwrap_or(source);
    assert!(
        production.contains("CRAWLER_STARTUP_DELAY"),
        "there must be a crawler startup-delay constant"
    );
    let starter = production
        .split("fn start_next_history_crawl_page")
        .nth(1)
        .and_then(|s| s.split("\n    fn ").next())
        .or_else(|| production.split("fn start_next_history_crawl_page").nth(1))
        .expect("start_next_history_crawl_page should exist");
    assert!(
        starter.contains("crawl_delay_elapsed"),
        "automatic crawl-page starts must be gated on the startup delay"
    );
    assert!(
        starter.contains("manual"),
        "manual crawls must bypass the startup delay"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib automatic_crawl_starts_are_delayed_at_startup`
Expected: FAIL.

- [ ] **Step 3a: Add the constant**

Near the other crawl constants in `search.rs`:

```rust
/// Automatic history crawling is held off for this long after the crawler first
/// has work, so it does not contend with user-visible pagination during the
/// startup window. Crawler timing is Rust-owned (not a user setting). The
/// maintainer confirmed a ~1 minute delay is fully acceptable (#123).
const CRAWLER_STARTUP_DELAY: std::time::Duration = std::time::Duration::from_secs(60);
```

- [ ] **Step 3b: Add fields + init**

Add to `struct SearchActor`:

```rust
    /// True once the startup delay has elapsed (automatic crawls may start).
    crawl_delay_elapsed: bool,
    /// One-shot startup-delay timer; its completion is awaited in `run`.
    crawl_delay_timer: Option<executor::JoinHandle<()>>,
```

Initialize both in `SearchActor` construction in `spawn`: `crawl_delay_elapsed: false,` and `crawl_delay_timer: None,`.

- [ ] **Step 3c: Gate automatic starts + arm the timer**

In `start_next_history_crawl_page`, at the top (after the `active_crawl_page.is_some()` guard), gate non-manual fronts:

```rust
    fn start_next_history_crawl_page(&mut self) {
        if self.active_crawl_page.is_some() {
            return;
        }
        // Startup delay: hold AUTOMATIC crawls until the delay elapses; manual
        // (explicit StartHistoryCrawl) checkpoints bypass it.
        if !self.crawl_delay_elapsed {
            match self.crawl_queue.front() {
                Some(front) if !front.manual => {
                    if self.crawl_delay_timer.is_none() {
                        self.crawl_delay_timer = Some(executor::spawn(async {
                            executor::sleep(CRAWLER_STARTUP_DELAY).await;
                        }));
                    }
                    return;
                }
                _ => {}
            }
        }
        let Some(checkpoint) = self.crawl_queue.pop_front() else {
            return;
        };
        // ... existing body unchanged ...
```

- [ ] **Step 3d: Resume when the timer fires (select-loop arm)**

In `run`'s `tokio::select!` (it already has `biased;`), add an arm after the `active_crawl_page` arm and before the `msg_rx` arm:

```rust
                _ = async {
                    self.crawl_delay_timer.as_mut().unwrap().await.ok();
                }, if self.crawl_delay_timer.is_some() => {
                    self.crawl_delay_timer = None;
                    self.crawl_delay_elapsed = true;
                    self.start_next_history_crawl_page();
                }
```

- [ ] **Step 4: Run test + build + full lib suite**

Run: `cargo test -p koushi-core --lib automatic_crawl_starts_are_delayed_at_startup` → PASS.
Run: `cargo build -p koushi-core` → warning-free.
Run: `cargo test -p koushi-core --lib` → all pass (no regression).

- [ ] **Step 5: Secret scan + commit**

```bash
git add crates/koushi-core/src/search.rs
git commit -m "feat: delay automatic history crawl ~60s at startup (#123, Phase B)"
```

---

## Self-Review

**1. Spec coverage** (against the Phase B spec §4):
- §4.1 startup delay → Task 4. ✓
- §4.2 cancellation preemption → Task 1 (gate) + Task 2 (crawler select). ✓
- §4.3 re-queue on preemption → Task 3. ✓
- §4.4 diagnostic → Task 2 (`trace_crawler_preempted`). ✓
- §5 ownership (actor-level only) → no reducer/DTO/CoreEvent touched. ✓
- §6 verification → unit tests per task + the lane re-run (controller, below). ✓

**2. Placeholder scan:** Task 2 Step 3c flags an SDK-cancellation-safety check (verify-then-proceed, with a named fallback) — concrete, not a placeholder. No "TBD"/"handle edge cases"/"write tests for the above" left.

**3. Type consistency:** `HistoryCrawlPageResult::Preempted { checkpoint }` defined in Task 2, consumed in Task 3. `MessagesRequestPermit::cancelled()` defined in Task 1, used in Task 2. `crawl_delay_elapsed` / `crawl_delay_timer` defined and used within Task 4. `trace_crawler_preempted` defined in Task 2 Step 3a, used in Step 3c. Consistent.

---

## Verification (controller, GO-gated)

After Tasks 1–4 pass their gates and codex review, the controller re-runs the Phase A lane (requires maintainer GO; the persistent QA device is kept):

```
npm --prefix apps/desktop run qa:real-homeserver -- --scenario=startup_latency
```

Expected in run 2: `koushi.startup phase=paginate ... gate_ms=` ≈ 0 (was ~1473), proving the user pagination no longer waits on the crawler. `koushi.startup phase=crawler_preempted` may appear if a crawler page was in flight. No new identifiers in output; redaction checks pass.

## Execution Handoff

Implementation is delegated to a Sonnet agent (subagent-driven), one task at a time; the controller (Opus) independently re-verifies every gate, runs a codex review of the diff, and then the GO-gated lane re-run, before opening the PR.
