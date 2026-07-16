# Activity Unread Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve Activity Unread placeholders into Rust-owned event rows with bounded, cancellable history loading and retryable failure state.

**Architecture:** Fix `ActivityProjection` to select known unread event rows first, then start a generation-guarded resolver for rooms that remain unresolved. `AccountActor` owns the session/task; an SDK helper consumes decrypted timeline cache/live diffs and bounded pagination. React renders only coarse resolution state and dispatches retry.

**Tech Stack:** Rust actors/reducer/Matrix SDK wrapper, serde DTOs, Tauri commands, TypeScript, Playwright, local headless QA.

---

### Task 1: Reproduce known-row placeholder behavior

**Files:**
- Modify: `crates/koushi-core/tests/runtime_activity.rs`
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] Add a core test with an unread room plus an injected event row and assert
  `activity.unread.rows` contains the event id, sender, preview, and timestamp
  and contains no `RoomUnread` row for that room.
- [ ] Add a browser test whose completed Unread snapshot has an event row and
  assert sender/context/preview/time render while no `data-kind=roomUnread`
  remains.
- [ ] Run:

  ```bash
  cargo test -p koushi-core --test runtime_activity known_unread_event_is_event_backed
  npm --prefix apps/desktop exec -- playwright test --config apps/desktop/playwright.config.ts apps/desktop/e2e/basic-operations.spec.ts -g "resolved Activity Unread" --workers=1
  ```

  Expected: core FAILS with a placeholder-only Unread stream.

### Task 2: Prefer known event rows

**Files:**
- Modify: `crates/koushi-core/src/runtime.rs`

- [ ] While building Recent, clone rows with `unread == true` into Unread and
  record their room ids. Do the same for `RoomSummary.latest_event` rows.
- [ ] Synthesize a placeholder only when the room has no event-backed unread
  row. Keep existing mute, low-priority, marker, cleared-event, and sorting
  logic unchanged.
- [ ] Re-run Task 1 commands and the full focused activity test binary.
- [ ] Commit with `fix(#264): prefer known unread activity rows`.

### Task 3: Add reducer-owned resolution state and commands

**Files:**
- Modify: `crates/koushi-state/src/state/activity.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/activity.rs`
- Modify: `crates/koushi-state/src/reducer/mod.rs`
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/event.rs`
- Test: `crates/koushi-state/tests/activity_state.rs`

- [ ] Write RED tests for `Idle -> Resolving -> Idle`, matching-generation
  success, matching-generation coarse failure, stale result rejection,
  explicit retry, close/session-clear cancellation, serde defaults, and Debug
  redaction.
- [ ] Add `ActivityResolutionState` to each `ActivityStream`, plus typed
  `RetryActivityResolution` command/event and request-correlated actions.
- [ ] Run:

  ```bash
  cargo test -p koushi-state --test activity_state
  cargo test -p koushi-core --lib activity_commands_debug
  ```

  Expected: PASS after minimal reducer/contract implementation.
- [ ] Commit with `feat(#264): model unread activity resolution`.

### Task 4: Add SDK bounded resolver behind a testable service

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-core/src/messages_backpressure.rs`
- Create: `crates/koushi-core/src/activity_resolution.rs`
- Modify: `crates/koushi-core/src/lib.rs`

- [ ] Define a private-data-safe request/result and an async resolver trait so
  core tests can drive cache rows, pagination pages, live diffs, network
  failures, and cancellation without a real server.
- [ ] Add SDK timeline resolution that deduplicates by event id, stops strictly
  after the fully-read marker, drains live updates between pages, uses 50-event
  pages and a 32-page cap, and returns coarse network/SDK/timeout kinds.
- [ ] Acquire the shared account `/messages` permit per page. Retry network
  failures twice with injectable backoff; do not use sleeps in tests.
- [ ] Add unit tests for cache-only, stale backfill, encrypted projection,
  live-during-pagination dedupe, end reached, cap exhaustion, retry, and cancel.
- [ ] Run:

  ```bash
  cargo test -p koushi-sdk --lib activity_resolution
  cargo test -p koushi-core --lib activity_resolution
  ```

- [ ] Commit with `feat(#264): resolve unread activity history`.

### Task 5: Wire the cancellable AccountActor lifecycle

**Files:**
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/tests/runtime_activity.rs`

- [ ] Add a redacted internal `AccountMessage` carrying generation and
  unresolved requests. Keep one task handle; abort on replacement, close,
  logout, lock, and account switch.
- [ ] `AppActor` derives unresolved requests only from the Rust projection,
  begins resolution on open/unread selection/retry, ingests matching results,
  and ignores stale generations. Re-evaluate current state before publishing.
- [ ] Extend runtime tests for stale history, task failure/retry, live event
  during resolution, generation replacement, mark-room/all-read, and no
  duplicate/disappearing rows.
- [ ] Run `cargo test -p koushi-core --test runtime_activity`. Expected: PASS.
- [ ] Commit with `feat(#264): drive activity resolver lifecycle`.

### Task 6: Mirror the wire contract and render status/retry

**Files:**
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/commands/activity.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Regenerate: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/components/panes.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] Add RED browser cases for resolving status, failed alert, retry command,
  and placeholders never rendering as final content.
- [ ] Mirror the Rust resolution enum and retry command through Tauri/fakes.
  Regenerate the checked-in event contract through its Rust test only.
- [ ] Render loading or retry status instead of placeholder rows whenever
  resolution is non-idle. Keep event row rendering and mark-read behavior.
- [ ] Run:

  ```bash
  cargo test -p koushi-desktop core_event_wire_format_matches_checked_in_contract_artifact
  npm --prefix apps/desktop run typecheck
  npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
  npm --prefix apps/desktop exec -- playwright test --config apps/desktop/playwright.config.ts apps/desktop/e2e/basic-operations.spec.ts -g "Activity Unread" --workers=1
  ```

- [ ] Commit with `feat(#264): render unread activity resolution`.

### Task 7: Canon, QA, and full verification

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `AGENTS.md`
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`
- Modify: `scripts/desktop-headless-local-qa.mjs`

- [ ] Replace the old terminal-placeholder canon with the resolution lifecycle
  and add token-only `activity_resolution=ok` evidence for stale/encrypted
  unread history. Never print identifiers/content/errors.
- [ ] Run focused gates plus:

  ```bash
  cargo fmt --all -- --check
  cargo test -p koushi-state --test activity_state
  cargo test -p koushi-core --test runtime_activity
  cargo test -p koushi-desktop
  npm --prefix apps/desktop run typecheck
  npm --prefix apps/desktop run test
  npm --prefix apps/desktop run lint
  git diff --check
  ```

- [ ] Run local Conduit/Tuwunel Activity QA when the harness/server binaries
  are available and record only approved tokens.
- [ ] Commit with `test(#264): prove unread activity resolution`.
