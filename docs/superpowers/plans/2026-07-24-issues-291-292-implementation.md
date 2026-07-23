# Issues 291 and 292 Implementation Plan

> **For agentic workers:** Follow `superpowers:test-driven-development` task by
> task. Do not write production code until the named failing check is observed.
> Re-run every claimed gate in the owning worktree; subagent reports are not
> verification evidence.

**Goal:** Distinguish ordinary reply from reply-in-thread in the room timeline,
and make cached room navigation independent of slow receipt, fully-read,
live-tail, and background timeline work while preserving exactly-once command
outcomes and durable convergence of the newest desired read state.

**Architecture:** #291 remains a presentation-only mapping over Rust-projected
row affordances. #292 splits timeline work into a reliable bounded foreground
control lane, a lossless coalescing read-state inbox, and generation-fenced
owned background workers. The manager never awaits Matrix network I/O in its
mailbox loop. Unconfirmed read intent is encrypted and account-scoped through
`StoreActor`; no private identifier enters snapshots, diagnostics, or QA output.

**Tech stack:** Rust/Tokio actors, `koushi-core`, `koushi-state`, Matrix Rust SDK
adapter, encrypted local stores, React/TypeScript/Vitest, Playwright, Cargo.

**Batch shape:** Keep the approved design commit. Land one implementation commit
for #291 and one implementation commit for #292, then open one non-squash PR.

---

## Task 1: Reproduce #291 at the presentation boundary

**Files:**

- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/i18n/messages.test.ts`

1. Add a fully actionable main-room row fixture and assert the semantic toolbar
   order: reaction, ordinary reply, reply in thread, edit, pin/unpin, more.
2. Assert the same relative order when edit and another optional action are
   absent.
3. Assert stable test-facing icon markers distinguish the bent reply arrow from
   the speech bubble without inspecting SVG path data.
4. Assert ordinary reply invokes only `onReply(roomId, eventId)` and thread
   reply invokes only `onOpenThread(roomId, eventId)`.
5. Assert distinct English and Japanese accessible labels/tooltips.
6. Assert the thread action is absent in `thread` and `focused` presentation
   contexts and remains absent for redacted, hidden, and non-actionable rows.
7. Preserve the existing thread-summary-chip assertion.
8. Run the focused checks and record the expected RED failures:

   ```bash
   npm --prefix apps/desktop test -- --run src/components/TimelineView.test.tsx
   npm --prefix apps/desktop test -- --run src/i18n/messages.test.ts
   ```

## Task 2: Implement #291 as thin GUI routing

**Files:**

- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Verify: `apps/desktop/src/components/TimelineView.test.tsx`
- Verify: `apps/desktop/src/i18n/messages.test.ts`

1. Pass the existing `presentationContext` down to the row action renderer
   without adding React-owned Matrix eligibility.
2. Keep ordinary reply mapped to `onReply`; add the room-only reply-in-thread
   control mapped to `onOpenThread`.
3. Order conditional controls according to the approved product contract.
4. Add distinct catalog entries in English and Japanese and use them for both
   accessible names and tooltips.
5. Add decorative/stable icon markers suitable for tests; do not use SVG path
   text as an oracle.
6. Run:

   ```bash
   npm --prefix apps/desktop test -- --run src/components/TimelineView.test.tsx
   npm --prefix apps/desktop test -- --run src/i18n/messages.test.ts
   npm --prefix apps/desktop run typecheck
   npm --prefix apps/desktop run lint
   ```

7. Commit only the #291 implementation and focused tests:

   ```bash
   git add apps/desktop/src/components/TimelineView.tsx \
     apps/desktop/src/components/TimelineView.test.tsx \
     apps/desktop/src/i18n/messages.ts \
     apps/desktop/src/i18n/messages.test.ts
   git commit -m "fix: distinguish reply actions in room timeline"
   ```

## Task 3: Reproduce #292 with controlled stalled effects

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs` (private actor seams and inline
  behavioral tests; an integration-test build cannot see library `cfg(test)`
  hooks)
- Add only for production-visible end-to-end contracts:
  `crates/koushi-core/tests/timeline_navigation_priority.rs`
- Add only for production-visible end-to-end contracts:
  `crates/koushi-core/tests/read_state_convergence.rs`
- Modify only if existing intent-lifecycle fixtures require extension:
  `crates/koushi-core/tests/runtime_intent_lifecycle.rs`

1. Add a private actor harness whose public receipt, private/thread receipt,
   fully-read bundle, live-tail refresh, and cancellation ACK can each be held
   by a controlled never-resolving future. Keep these behavioral tests inline
   unless the seam is a production-visible port; do not expose test-only
   internals merely to satisfy an integration binary. Use events and absolute
   deadlines, never sleeps.
2. Submit a stalled public receipt, then a cached room selection, and assert the
   selection receives one terminal result and initial projection before the
   navigation deadline.
3. Repeat for a stalled fully-read bundle and a missing live-tail cancellation
   ACK.
4. Fill the ordinary/background lane with tagged completions and assert an
   A → B → C foreground selection sequence converges to C without starvation.
5. Assert every admitted selection receives exactly one bounded terminal
   outcome, including foreground-lane admission failure.
6. Add RED convergence tests for a newer read target superseding an older
   target, stale session/actor/operation completions, duplicate wakes, distinct
   public/private/thread/fully-read keys, and reconnect retry sending only the
   newest provable target.
7. Run the new integration binaries directly and record the expected RED:

   ```bash
   cargo test -p koushi-core --test timeline_navigation_priority
   cargo test -p koushi-core --test read_state_convergence
   ```

## Task 4: Add the reliable foreground control lane

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`
- Modify as required for correlated routing:
  `crates/koushi-core/src/account.rs`
- Modify as required for intent completion:
  `crates/koushi-core/src/runtime.rs`
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`
- Verify: `crates/koushi-core/tests/timeline_navigation_priority.rs`
- Verify: `crates/koushi-core/tests/runtime_intent_lifecycle.rs`

1. Amend the async/runtime canon before code so the foreground lane,
   correlated admission failure, priority rule, and bounded cancellation are
   normative.
2. Add named bounded foreground capacity and typed subscribe,
   unsubscribe/preemption, and shutdown/control messages.
3. Make the manager select foreground work before ordinary commands and
   background completions without converting reliable delivery to lossy
   `try_send`.
4. Commit navigation state before optional target-network work; publish cached
   initial projection without waiting for the previous room.
5. Invalidate live-tail operation generation first, then share one short
   absolute deadline between cancellation delivery and ACK wait. Fence any late
   ACK or refresh completion.
6. Keep gap-repair publication/render ACK semantics unchanged; room change only
   removes previous-room foreground demand.
7. Run the navigation and intent-lifecycle binaries until GREEN.

## Task 5: Add lossless coalescing and monotonic read-state ownership

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`
- Modify if SDK boundary injection is needed:
  `crates/koushi-core/src/lib.rs`
- Verify: `crates/koushi-core/tests/read_state_convergence.rs`
- Verify existing focused receipt/live-signal tests under
  `crates/koushi-core/tests/`

1. Add a private shared read-state inbox and a bounded wake channel. Record
   desired data before returning from admission; a full wake must remain safe
   because an existing wake drains all pending data.
2. Key desired state by account/session, room timeline target, and read-state
   kind, keeping public, private/thread, and atomic fully-read-plus-private
   operations distinct.
3. Publish an atomically replaced actor position index containing only actor
   generation, ordered ranks, and confirmed positions. Replace it before later
   commands can observe a new projection.
4. Keep only the greatest provable target. For currently unordered candidates,
   retain a small explicit bounded set rather than guessing from timestamps,
   event-id text, submission order, or completion order.
5. Add a named bound for correlated request waiters per key/kind. Reject new
   admission with one correlated terminal failure before acceptance when that
   bound is full; never evict an already accepted waiter.
6. Preserve every accepted `request_id`: a dominating success settles its
   dominated waiters once; timeout/failure settles once while internal desired
   convergence may continue without a second terminal result.
7. Move all SDK waits to supervisor-owned cancellable workers. Permit one
   in-flight operation per key/kind, tag it with session/actor/operation
   generations, and use one absolute timeout per operation.
8. On supersession, cancellation, disconnect, logout, account switch, or
   shutdown, settle ownership and reject stale completions.
9. Add capped exponential retry driven by backoff plus reconnect,
   subscription-checkpoint, and authoritative-receipt wakes. Coalesce repeated
   viewport observations.
10. Replace the fully-read success path's lossy reducer `try_send` with reliable
    delivery before emitting its success event.
11. Run the focused convergence and existing receipt/live-signal checks.

## Task 6: Persist pending convergence safely

**Files:**

- Modify: `crates/koushi-core/src/store.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Add: `crates/koushi-core/tests/read_state_persistence.rs`
- Modify if reset contract coverage is colocated:
  `crates/koushi-core/tests/local_encryption_state.rs`

1. Add RED tests for encrypted round-trip, absence of plaintext synthetic
   room/event identifiers in the file, wrong-key/corrupt fail-closed behavior,
   confirmed-entry removal, stale debounced-save rejection, reset removal, and
   bounded final shutdown save.
2. Add a dedicated HKDF domain and versioned, count/byte-bounded payload for
   minimal desired read-state data. Keep it separate from composer drafts and
   scheduled sends.
3. Make `AccountActor` own load/save/reset because it owns `StoreActor` and the
   active session key; exchange only typed private snapshots/wakes with the
   timeline manager.
4. Load before enabling retries. On restart, compare with authoritative SDK/sync
   state: delete already-satisfied targets and retry only server-behind targets.
5. Fence delayed saves by account/session generation, remove entries after
   confirmation, clear on reset, and save once after worker cancellation during
   orderly shutdown.
6. Ensure persisted data never enters `AppState`, Tauri/TypeScript DTOs,
   diagnostics, `Debug`, or QA stdout.
7. Run:

   ```bash
   cargo test -p koushi-core --test read_state_persistence
   cargo test -p koushi-core --test read_state_convergence
   cargo test -p koushi-core --test timeline_navigation_priority
   ```

## Task 7: Review #292 against the full behavior contract

**Files:**

- Review every file changed since the #291 implementation commit.
- Add focused regression tests only in per-feature integration binaries or as
  pure private-helper unit tests.

1. Verify the diff against `REPOSITORY_RULES.md`,
   `docs/architecture/overview.md`, `docs/architecture/state-machine.md`,
   `docs/policies/engineering-rules.md`, `AGENTS.md`, and the approved design.
2. Check task ownership, cancellation/join bounds, channel capacities,
   starvation behavior, exactly-once correlated outcomes, monotonic target
   ordering, persistence limits, account isolation, and private-data-free
   `Debug` output.
3. Run an external Codex diff review using the repository recipe; verify rather
   than blindly adopt each finding.
4. Commit the complete #292 implementation, tests, and canon amendments:

   ```bash
   git add crates/koushi-core docs/architecture docs/policies
   git commit -m "fix: keep room navigation ahead of timeline side effects"
   ```

## Task 8: Run focused and repository gates

1. Run focused Rust tests:

   ```bash
   cargo test -p koushi-core --test timeline_navigation_priority
   cargo test -p koushi-core --test read_state_convergence
   cargo test -p koushi-core --test read_state_persistence
   cargo test -p koushi-core --test runtime_intent_lifecycle
   cargo test -p koushi-core --lib
   ```

2. Run workspace and desktop gates:

   ```bash
   cargo test -p koushi-state
   cargo test -p koushi-auth
   cargo test -p koushi-core
   cargo check --workspace --all-targets
   npm --prefix apps/desktop test
   npm --prefix apps/desktop run typecheck
   npm --prefix apps/desktop run lint
   npm --prefix apps/desktop run build
   ```

3. Run the relevant local headless scenarios with private-data-free tokens,
   including `live_signals` and cached room navigation under a disabled/stalled
   proxy. Use the repository scenario entry points and their documented
   absolute timeout; do not invent fixed waits.
4. Inspect `git status`, ensure no generated/build artifacts or unrelated files
   are included, and verify the branch contains the design commit plus one
   implementation commit per issue.

## Task 9: Publish, monitor, and merge

1. Push `codex/issues-291-292`.
2. Open one non-squash PR referencing and closing #291 and #292. Summarize the
   architectural ownership change, RED→GREEN evidence, measured navigation
   deadline, privacy properties, and exact verification commands.
3. Monitor GitHub Actions. Fix any failure at its root and re-run the relevant
   local gate before pushing.
4. Resolve review threads with evidence, re-run final verification, and merge
   with a merge commit (not squash).
5. Confirm the PR is merged, both issues are closed, and `origin/main` contains
   both implementation commits.
