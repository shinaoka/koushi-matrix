# Activity Edit Identity and Anchored Navigation Repair Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use executing-plans to implement this plan task-by-task.

**Goal:** Deduplicate edited Activity rows by displayed-event identity and make Activity navigation commit an anchor only when the requested target is renderable.

**Architecture:** Preserve relation metadata through the SDK→state projection, canonicalize only at the Activity boundary, and extend the existing projection acknowledgement with target-presence evidence. Preserve focused thread filtering options in the vendored SDK and refresh only an already-empty focused cache.

**Tech Stack:** Rust, TypeScript/React, Tauri, matrix-rust-sdk submodule, Cargo tests, Vitest.

---

### Task 1: Canonicalize Activity identities

**Files:**
- Modify: `crates/koushi-state/src/state/room.rs`
- Modify: `crates/koushi-core/src/room.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Test: `crates/koushi-core/src/runtime.rs`

- [ ] Add RED tests for an original row plus `m.replace` latest summary and for annotation suppression.
- [ ] Add relation fields with serde defaults to `RoomLatestEventSummary` and project SDK values into them.
- [ ] Add a canonical displayed-ID helper and use it in latest-row reconciliation and navigation identity.
- [ ] Run focused `koushi-core` tests and make them GREEN.

### Task 2: Gate anchored navigation on target presence

**Files:**
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `apps/desktop/src-tauri/src/commands/navigation.rs`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/App.tsx`
- Test: `crates/koushi-core/src/runtime.rs`
- Test: `apps/desktop/src/domain/timelineStore.test.ts`
- Test: `apps/desktop/src-tauri/src/commands/mod.rs`

- [ ] Add RED core tests proving accepted target-missing projections do not anchor and target-present projections do.
- [ ] Compute `item_count` and `target_present` from the applied canonical WebView store and send them with the acknowledgement.
- [ ] Route exact-owner evidence through Tauri/core and make target-missing navigation close focused state and retain the selected live room.
- [ ] Make the desktop wait helper terminate on explicit live fallback.
- [ ] Add private-data-free `core.activity_navigation` outcome logs.
- [ ] Run focused Rust and TypeScript tests and make them GREEN.

### Task 3: Preserve focused thread-mode and refresh empty caches

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/event_focused/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/timeline/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/timeline/controller/mod.rs`
- Test: vendored SDK focused timeline tests

- [ ] Add RED tests that `hide_threaded_events: false` retains thread replies in a non-thread focused context.
- [ ] Carry the boolean in `EventFocusThreadMode` and its cache key.
- [ ] Add a bounded `get_or_refresh_empty_event_focused_cache` path and use it for focused UI initialization.
- [ ] Add tests that a non-empty cache is reused while an empty cache is recreated once.
- [ ] Run focused SDK tests and make them GREEN.

### Task 4: Integrated verification and publication

**Files:**
- Review all modified files and the submodule gitlink.

- [ ] Run formatting and submodule guard.
- [ ] Run the focused core, desktop, and SDK suites once after implementation.
- [ ] Run appropriate workspace compile checks.
- [ ] Review the complete diff for privacy, stale fallbacks, and unrelated changes.
- [ ] Commit the SDK submodule change, then the parent changes.
- [ ] Push `codex/issue-364-activity-anchor` and open a draft PR linking `Fixes #364`.
