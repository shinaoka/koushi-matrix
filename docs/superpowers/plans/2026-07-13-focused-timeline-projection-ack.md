# Focused Timeline Projection Acknowledgement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent Focused timeline anchors from being published until the matching actor-generation projection is applied to the WebView's canonical timeline store, with actor-owned recovery after lost delivery.

**Architecture:** Core owns a request-correlated Focused navigation state machine and retained projection lease. `InitialItems` transports the lease to the app-level store; a typed, idempotent acknowledgement advances Core to anchor publication. Existing `EnsureSubscribed` replay is generation-fenced and becomes the event-driven recovery path.

**Tech Stack:** Rust (`koushi-core`, `koushi-state`, Tauri), TypeScript/React, Vitest, Cargo tests, Playwright/headless local QA.

---

### Task 1: Define the projection lease and navigation protocol

**Files:**
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Test: `crates/koushi-core/src/runtime.rs`

- [ ] **Step 1: Write failing protocol tests**

Add behavioral tests that construct a pending Focused navigation, reject an acknowledgement with a mismatched generation/revision, accept the exact lease once, and prove a duplicate acknowledgement is idempotent.

- [ ] **Step 2: Run the RED tests**

Run: `cargo test -p koushi-core focused_projection -- --nocapture`

Expected: FAIL because projection identity and acknowledgement commands do not exist.

- [ ] **Step 3: Add typed identities and reducer transitions**

Define opaque serializable `TimelineProjectionId`/navigation identity types, acknowledgement metadata on `InitialItems`, and a typed acknowledgement command. Store the pending Focused navigation in Core runtime state. Validate the active key, actor generation, projection revision, and navigation owner before dispatching `EnterAnchoredTimeline`; stale acknowledgement returns a typed non-success outcome without state mutation.

- [ ] **Step 4: Run the protocol tests**

Run: `cargo test -p koushi-core focused_projection -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add crates/koushi-core && git commit -m "feat(core): gate focused anchor on projection acknowledgement"`

### Task 2: Retain and generation-fence actor projections

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`
- Test: `crates/koushi-core/src/timeline.rs`

- [ ] **Step 1: Write failing actor tests**

Add tests for InitialItems-before-screen ordering, `EnsureSubscribed` replay after remount, stale generation rejection, unrelated-event burst identity preservation, and a dropped first delivery followed by actor-owned reprojection.

- [ ] **Step 2: Run the RED tests**

Run: `cargo test -p koushi-core projection_replay -- --nocapture`

Expected: FAIL because replay does not retain acknowledgement identity and emission success is discarded.

- [ ] **Step 3: Implement retained projection leases**

Retain the latest acknowledgement-bearing snapshot inside the active actor generation. Make the generation-lease emission result observable, preserve identity on `EnsureSubscribed { replay_existing: true }`, and clear the retained lease only after acknowledgement, supersession, or unsubscribe.

- [ ] **Step 4: Run actor tests**

Run: `cargo test -p koushi-core projection_replay -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add crates/koushi-core/src/timeline.rs && git commit -m "fix(core): retain focused projections until applied"`

### Task 3: Apply and acknowledge projections in the canonical WebView store

**Files:**
- Modify: `apps/desktop/src/domain/timelineStore.ts`
- Modify: `apps/desktop/src/domain/timelineStore.test.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: generated/checked-in IPC contract artifacts as required by repository tooling

- [ ] **Step 1: Write failing store tests**

Cover `Applied`, `RejectedStale`, and `Ignored` outcomes; verify the acknowledgement is emitted only after the matching key/generation/revision owns canonical items; cover a thread projection without main-anchor mutation.

- [ ] **Step 2: Run the RED tests**

Run: `pnpm --dir apps/desktop test -- src/domain/timelineStore.test.ts`

Expected: FAIL because store application has no acknowledgement result.

- [ ] **Step 3: Implement application results and typed acknowledgement**

Have the canonical reducer return the next store plus projection application result. In the app-level listener, commit the store first, then invoke the typed backend acknowledgement for `Applied` only. Preserve pre-mount application because the listener remains app-owned.

- [ ] **Step 4: Run frontend tests and contracts**

Run: `pnpm --dir apps/desktop test -- src/domain/timelineStore.test.ts && pnpm --dir apps/desktop typecheck && pnpm --dir apps/desktop test:ipc-contract`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add apps/desktop crates/koushi-core && git commit -m "feat(desktop): acknowledge applied timeline projections"`

### Task 4: Consolidate Focused navigation entry points

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands/navigation.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Test: `apps/desktop/src-tauri/src/commands/mod.rs`
- Test: `crates/koushi-core/src/runtime.rs`

- [ ] **Step 1: Replace source-string tests with behavioral RED tests**

Drive Activity Recent, Activity Unread, search-result, and date-jump entry points through the same request-correlated workflow. Assert no anchor before WebView acknowledgement and completion after the exact acknowledgement.

- [ ] **Step 2: Run the RED tests**

Run: `cargo test -p koushi-desktop focused_navigation -- --nocapture && cargo test -p koushi-core focused_navigation -- --nocapture`

Expected: FAIL while commands still use `wait_for_focused_timeline_event` and publish anchors directly.

- [ ] **Step 3: Implement the common workflow**

Extract one Focused navigation helper used by Activity and search and route date navigation into the same Core workflow. Remove `wait_for_focused_timeline_event` as a success criterion. Command completion observes only the correlated Core navigation completion.

- [ ] **Step 4: Run common-path tests**

Run: `cargo test -p koushi-desktop focused_navigation -- --nocapture && cargo test -p koushi-core focused_navigation -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git add apps/desktop/src-tauri crates/koushi-core && git commit -m "refactor(navigation): unify focused projection workflow"`

### Task 5: Verify transport diagnostics and all Issue #242 regressions

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Test: `apps/desktop/src-tauri/src/lib.rs`
- Test: relevant Rust/frontend/headless scenario files discovered from existing timeline tests

- [ ] **Step 1: Add forwarding failure and integrated regression tests**

Verify `app.emit` failure is observable without moving ownership into Tauri. Add integrated coverage for the ten cases listed in the design, including normal Activity events, thread reply, burst, and lost-delivery recovery.

- [ ] **Step 2: Run focused verification**

Run: `cargo test -p koushi-core focused_projection -- --nocapture && cargo test -p koushi-desktop focused_navigation -- --nocapture && pnpm --dir apps/desktop test -- src/domain/timelineStore.test.ts`

Expected: PASS.

- [ ] **Step 3: Run repository gates**

Run: `cargo test --workspace && pnpm --dir apps/desktop test && pnpm --dir apps/desktop typecheck && pnpm --dir apps/desktop lint && pnpm --dir apps/desktop lint:tauri-boundary && pnpm --dir apps/desktop test:ipc-contract`

Expected: PASS.

- [ ] **Step 4: Run relevant headless local QA**

Run the repository's local-server timeline/navigation scenario selected from `qa:headless-local`, recording the exact command and result in the PR.

- [ ] **Step 5: Commit final test/diagnostic changes**

Run: `git add apps crates docs && git commit -m "test: cover focused projection delivery recovery"`

### Task 6: Review, publish, and merge

- [ ] **Step 1: Audit every Issue #242 requirement against concrete tests and source evidence.**
- [ ] **Step 2: Request independent code review and address all actionable findings.**
- [ ] **Step 3: Push the branch and open a PR with `Closes #242`.**
- [ ] **Step 4: Wait for required CI checks to pass.**
- [ ] **Step 5: Merge using a merge commit, delete the remote branch, and verify Issue #242 is closed and origin/main contains the merge.**

