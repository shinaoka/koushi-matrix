# Timeline Navigation Phase A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Rust-owned timeline-navigation contract for issue #41: read-marker/unread position derivation, jump-to-bottom count, and jump-to-date resolution through focused context.

**Architecture:** Timeline item lists remain event-driven CoreEvent data; they must not be copied into `AppState`. The Rust timeline actor owns item order and combines it with GUI-reported viewport observations plus the fully-read marker to emit a typed navigation projection. The GUI may observe visible item ids and scroll, but it must render only the Rust projection and dispatch typed commands.

**Tech Stack:** Rust (`matrix-desktop-state`, `matrix-desktop-core`, Tauri command adapter), TypeScript wire types/timeline store, local headless core QA, Vitest/Playwright in Phase B.

---

### Task 1: Rust State Contract For Timeline Navigation

**Files:**
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src-tauri/src/lib.rs`

- [x] Add `TimelineNavigationSnapshot` and `TimelineViewportObservation` DTOs in `event.rs`.
- [x] Add `TimelineEvent::NavigationUpdated { key, snapshot }`.
- [x] Add `TimelineCommand::ObserveViewport { request_id, key, observation }`.
- [x] Redact event ids in `Debug` for the new command/event.
- [x] Extend `coreEvents.ts` and `coreEvents.generated.json` coverage so wire drift fails locally.
- [x] Run:
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact`

### Task 2: Timeline Actor Derivation

**Files:**
- Modify: `crates/matrix-desktop-core/src/timeline.rs`
- Test: `crates/matrix-desktop-core/src/timeline.rs`

- [x] Write RED unit tests for:
  - first unread inside viewport reports `InsideViewport`.
  - marker before viewport reports unread below state from Rust item order.
  - viewport not at bottom reports `newer_event_count` for jump-to-bottom.
  - local echoes and synthetic items are ignored for unread/navigation counts.
- [x] Store current projected `TimelineItem` order and the current fully-read marker in the actor.
- [x] Recompute navigation after `ItemsUpdated`, fully-read marker changes, and viewport observations.
- [x] Emit `TimelineEvent::NavigationUpdated` only when the projection changes.
- [x] Run:
  `cargo test -p matrix-desktop-core --lib timeline_navigation`

### Task 3: Fully-Read Marker Flow Into The Actor

**Files:**
- Modify: `crates/matrix-desktop-core/src/timeline.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`

- [x] Ensure the actor records `room.fully_read_event_id()` during subscribe.
- [x] Ensure `handle_set_fully_read` updates the actor-local marker before emitting navigation.
- [x] Keep `LiveSignalsEvent::FullyReadSet` and `AppAction::FullyReadMarkerUpdated` behavior unchanged.
- [x] Run:
  `cargo test -p matrix-desktop-core --lib fully_read`

### Task 4: Jump-To-Date Command Contract

**Files:**
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Test: `crates/matrix-desktop-core/src/runtime.rs`
- Test: `apps/desktop/src-tauri/src/commands.rs`

- [x] Add `AppCommand::OpenTimelineAtTimestamp { request_id, room_id, timestamp_ms }`.
- [x] Route it through Rust only: validate ready session, resolve timestamp to event, then reuse the focused-context timeline path.
- [x] If the SDK has no high-level helper, add a small wrapper around the ruma MSC3030 endpoint in the Rust account/session owner with a unit-test seam; do not let React call raw Matrix APIs.
- [x] Add a Tauri command `open_timeline_at_timestamp(room_id, timestamp_ms)`.
- [x] Redact room/event ids in `Debug`.
- [x] Run:
  `cargo test -p matrix-desktop-core --lib open_timeline_at_timestamp`
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib open_timeline_at_timestamp`

### Task 5: Private QA Token

**Files:**
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Modify: `AGENTS.md`
- Modify: `docs/architecture/state-machine.md`

- [x] Add a local core timeline navigation check that observes a real viewport path without printing room ids, event ids, or message bodies.
- [x] Print `timeline_nav=ok` when the navigation projection is observed.
- [x] Document that GUI viewport observations are facts, while marker/unread/bottom/date semantics are Rust-owned.
- [x] Run:
  `cargo test -p matrix-desktop-core --features qa-bin --bin headless-core-qa`

### Task 6: Phase A Commit And Issue Evidence

**Files:**
- Commit only Phase A Rust/state-machine/headless/wire contract files.

- [x] Run:
  `cargo test -p matrix-desktop-core --lib`
  `cargo test -p matrix-desktop-core --features qa-bin --bin headless-core-qa`
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib`
  `npm --prefix apps/desktop run typecheck`
  `npm --prefix apps/desktop run test -- --run`
  `npm --prefix apps/desktop run qa:secret-scan`
- [ ] Commit as `Add timeline navigation Phase A contract`.
- [ ] Push to `origin/main`.
- [ ] Comment on #41 and #12 with Phase A evidence; keep #41 open for Phase B GUI/browser-headless/Linux virtual-display evidence.
