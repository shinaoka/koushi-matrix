# Room Timeline Gap Repair Implementation Plan

> Issue: #260. Execute headless-first and test-first. The design contract is
> `docs/superpowers/specs/2026-07-03-room-timeline-cache-repair-design.md`.

## Goal

Pin the targeted-gap SDK API, add Rust-owned inspection/repair orchestration,
replace destructive reset with non-destructive repair, and render explicit
positioned gap state without exposing SDK tokens or identifiers.

## Task 1: Pin and wrap the SDK contract

- [ ] Update the workspace Matrix SDK revision and vendored submodule to SDK PR
      #1 merge `10b7cf7fad24e4ac5590e112c46d35428ef0cebf`.
- [ ] Add RED `koushi-sdk` adapter tests for token-free inspection mapping,
      bounded outcomes, and redacted `Debug`.
- [ ] Add app-owned adapter types/methods in `crates/koushi-sdk`; keep SDK
      descriptors in a non-serializable Rust handle.
- [ ] Run `cargo test -p koushi-sdk --lib timeline_gap` and focused event-cache
      integration checks.

## Task 2: Add the reducer state machine

- [ ] Add RED tests in
      `crates/koushi-state/tests/timeline_gap_repair_state.rs` for every state,
      stale-generation rejection, coalescing, logout/account clear, retry, and
      redacted `Debug`.
- [ ] Add cohesive state/action/reducer modules and integrate the new state into
      `AppState` with backward-compatible defaults.
- [ ] Update StateDelta slice classification and state-generation behavior.
- [ ] Run `cargo test -p koushi-state --test timeline_gap_repair_state`.

## Task 3: Build Core inspection and scheduling

- [ ] Add RED actor tests in `crates/koushi-core/src/timeline.rs` for subscribe
      inspection, priority, bounded continuation, stale reinspection, duplicate
      coalescing, retry/cancellation, and actor-generation guards.
- [ ] Add an actor-private per-room scheduler and descriptor registry. Route all
      timers/tasks through the executor abstraction.
- [ ] Trigger inspection on subscription, reconnect/cache updates/live diffs,
      known-event navigation, and manual commands where the corresponding Core
      fact exists.
- [ ] Preserve the current projection on every failure and never call
      `EventCacheStore::remove_room`.
- [ ] Emit reliable state actions and privacy-safe diagnostics.
- [ ] Run focused Core unit tests with `--lib`.

## Task 4: Project gap rows and authoritative start

- [ ] Add RED event/serialization tests for a content-free positioned gap item
      and authoritative-start state.
- [ ] Project gap rows through initial items/diffs using stable presentation
      keys; keep boundaries and handles actor-private.
- [ ] Gate **Start of conversation** on the SDK continuity proof rather than
      local emptiness or edge pagination state.
- [ ] Update display-label/policy traversal helpers to tolerate the synthetic
      row without inventing message semantics.
- [ ] Run Core event contract and timeline-store unit tests.

## Task 5: Replace reset across Tauri and TypeScript

- [ ] Add RED Tauri command tests for `repair_room_timeline` command shape and
      removal of the reset command.
- [ ] Update Tauri DTOs, generated event artifacts, TypeScript domain types,
      browser fake snapshots/commands, app harness snapshots, and IPC mocks.
- [ ] Replace Room info copy/confirmation/progress/failure UI. Repeated clicks
      dispatch the shared typed repair request and do not clear local rows.
- [ ] Render pending/failed positioned gap rows and keyboard retry affordance.
- [ ] Preserve the existing viewport anchor when repair inserts events.
- [ ] Run typecheck, focused Vitest, and Playwright gap/room-info specs.

## Task 6: Add end-to-end headless proof

- [ ] Add a RED local `headless-core-qa` scenario that creates a limited/internal
      gap with synthetic users and asserts automatic repair through
      `CoreEvent`/state, including network-failure preservation and unrelated
      room isolation.
- [ ] Emit only coarse success tokens such as `gap_detected=ok`,
      `gap_repaired=ok`, `projection_preserved=ok`, and
      `authoritative_start=ok`.
- [ ] Exercise both supported Core backends where the scenario applies and use
      event-driven timeouts only.

## Task 7: Verify, review, and publish

- [ ] Run `cargo fmt --all -- --check`, focused Rust suites, Tauri contract
      tests, TypeScript tests/typecheck, browser-headless specs, and the local
      homeserver scenario.
- [ ] Confirm `rg "remove_room|reset_room_timeline_cache"` has no normal repair
      path and inspect `git diff --check`.
- [ ] Run the repository Codex diff-review recipe against `origin/main`; verify
      or fix each actionable finding.
- [ ] Commit in reviewable layers, push, open a PR closing #260, monitor CI,
      address failures, and merge non-squash after all required checks pass.
