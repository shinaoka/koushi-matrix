# Matrix Connection Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show Matrix homeserver connection health clearly and keep SyncService alive across transient mobile-network outages.

**Architecture:** Reuse the existing Rust-owned `sync` state instead of adding a new transport. Fix SyncService state handling so `Offline` and transient `Error` project reconnecting state without ending the observer, then render the existing TopBar connection indicator with homeserver context and stronger warning copy.

**Tech Stack:** Rust async actor tests in `crates/koushi-core`, Rust reducer tests in `crates/koushi-state`, React/Vitest tests in `apps/desktop`, existing i18n catalog and CSS.

## Global Constraints

- Keep Matrix sync semantics Rust-owned; React only renders snapshot state.
- Do not expose raw SDK error text in UI or logs.
- Do not add a modal, toast, or blocking overlay for transient connection issues.
- Keep unrelated untracked files untouched.

---

### Task 1: SyncService Transient Reconnect Semantics

**Files:**
- Modify: `crates/koushi-core/src/sync.rs`

**Interfaces:**
- Consumes: `observe_sync_service_states(state_sub, event_tx, action_tx) -> SyncTaskOutcome`
- Produces: `Offline` and transient `Error(_)` emit `SyncEvent::Reconnecting` / `AppAction::SyncReconnecting` and keep waiting; `Running` after reconnect emits `SyncRecovered`; `Terminated` remains terminal.

- [ ] **Step 1: Write failing tests**
  Add tests that inspect the `Offline` and `Error(_)` arms and require no `return SyncTaskOutcome::Failed` before the next arm. Update the existing offline test name and assertion to require observer continuity.

- [ ] **Step 2: Verify red**
  Run: `cargo test -p koushi-core sync_service`
  Expected: FAIL because the current code returns `SyncTaskOutcome::Failed`.

- [ ] **Step 3: Implement minimal code**
  Change `Offline` to emit reconnecting once per outage and continue. Change `Error(_)` to do the same with a stable `network_error` reason. Keep `Terminated` terminal.

- [ ] **Step 4: Verify green**
  Run the same `cargo test -p koushi-core sync_service` command and expect the sync service tests to pass.

### Task 2: Reducer Allows Running To Reconnecting

**Files:**
- Modify: `crates/koushi-state/src/reducer/sync.rs`
- Test: `crates/koushi-state/src/reducer/sync.rs`

**Interfaces:**
- Consumes: `handle_sync_reconnecting(state, reason)`
- Produces: `SyncState::Running` transitions to `SyncState::Reconnecting { reason }`; `Stopped` still ignores reconnecting.

- [ ] **Step 1: Write failing reducer tests**
  Add unit tests for `Running -> Reconnecting` and `Stopped` ignoring reconnecting.

- [ ] **Step 2: Verify red**
  Run: `cargo test -p koushi-state sync_reconnecting_from_running_updates_state`
  Expected: FAIL because `Running` is currently ignored.

- [ ] **Step 3: Implement minimal code**
  Remove `SyncState::Running` from the ignored-state match in `handle_sync_reconnecting`.

- [ ] **Step 4: Verify green**
  Run the same reducer test and expect pass.

### Task 3: TopBar Matrix Server Warning

**Files:**
- Modify: `apps/desktop/src/app/uiShared.ts`
- Modify: `apps/desktop/src/components/Shell.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/components/Shell.test.tsx`

**Interfaces:**
- Consumes: `syncStatePresentation(sync)` and `snapshot.state.domain.session.homeserver`
- Produces: TopBar renders the homeserver next to the connection label; reconnecting and failed states are accessible status alerts with warning styling and restart action.

- [ ] **Step 1: Write failing React tests**
  Add a TopBar test that passes `homeserver="https://matrix.org"` and `sync={{ reconnecting: "network_offline" }}` and expects a visible status containing `matrix.org` and `Reconnecting`. Add a failed-state test for restart button visibility.

- [ ] **Step 2: Verify red**
  Run: `npm test -- src/components/Shell.test.tsx -t "Matrix connection"`
  Expected: FAIL because TopBar does not accept/render `homeserver`.

- [ ] **Step 3: Implement minimal UI**
  Add optional `homeserver` prop, format it as hostname, pass it from `App.tsx`, and tune CSS for warning states without changing layout structure.

- [ ] **Step 4: Verify green**
  Run the same Vitest command and expect pass.

### Task 4: Full Verification

**Files:**
- Verify only.

- [ ] **Step 1: Rust targeted tests**
  Run: `cargo test -p koushi-core sync_service`
  Run: `cargo test -p koushi-state sync_reconnecting`

- [ ] **Step 2: Frontend targeted tests**
  Run: `npm test -- src/components/Shell.test.tsx`

- [ ] **Step 3: Workspace quality gates**
  Run: `npm run typecheck`
  Run: `npm run lint`

- [ ] **Step 4: Commit**
  Stage only intended files and commit with `fix: surface Matrix connection recovery`.
