# Live-Edge Timeline Gap Repair Implementation Plan

> Issue #269. Execute test-first and headless-first. The approved design is
> `docs/superpowers/specs/2026-07-16-live-edge-gap-repair-design.md`.

## Goal

Repair the newest unprojected Room timeline gap automatically under a distinct,
small live-edge budget while preserving bounded historical repair and every
existing causal/render fence, then publish and merge the change.

## Constraints

- Ordinary `Automatic` remains `event_limit=64`, `cached_chunk_limit=0`, and
  never selects an unprojected descriptor.
- `LiveEdge` is Room-only, requires an actor-private rendered target, uses
  `event_limit=64`, `cached_chunk_limit=1`, and stops after four batches.
- Manual remains highest priority and keeps its existing semantics.
- No gap handle, token, boundary ID, target ID, message content, room ID, user
  ID, or raw SDK error enters diagnostics or a public DTO.
- Existing projection correlation, render acknowledgement, actor/timeline/
  repair/batch generation fences, and non-destructive cache behavior remain.

## Task 1: RED policy and adapter contract

**Files:**

- Modify `crates/koushi-sdk/tests/timeline_gap_adapter.rs`
- Modify `crates/koushi-core/src/timeline.rs` test module

1. Add an SDK contract test requiring a coarse topology revision accessor and
   proving redacted `Debug` does not expose IDs/tokens.
2. Add Core tests for:
   - `Automatic < LiveEdge < Manual` coalescing;
   - live-edge one-chunk budget and four-batch ceiling;
   - unprojected newest-gap fallback only with a live-edge target;
   - unchanged revision/ordinal after a batch returning `no_progress`;
   - target changes rearming a bounded attempt while identical targets idle;
   - stale/zero-progress outcomes stopping without requeue.
3. Run focused tests and capture the expected compile/assertion failures:

   ```bash
   cargo test -p koushi-sdk --test timeline_gap_adapter
   cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
   ```

## Task 2: Minimal SDK and Core implementation

**Files:**

- Modify `crates/koushi-sdk/src/lib.rs`
- Modify `crates/koushi-core/src/timeline.rs`

1. Expose `MatrixTimelineGapHandle::topology_revision() -> u64`; keep the SDK
   descriptor opaque and `Debug` private-safe.
2. Add `TimelineGapRepairTrigger::LiveEdge`, its token and budget.
3. Add pure descriptor-selection and progress-classification helpers so policy
   tests do not require a running homeserver.
4. Extend the tracker with live-edge selection fingerprint, target signature,
   attempt batch count, and the four-batch guard. A changed target resets only
   the live-edge attempt; it does not reset global/manual safety state.
5. Capture the newest rendered event identity during Room actor creation. Queue
   `LiveEdge` instead of `Automatic` for initial room inspection.
6. Recompute the target after accepted live diff batches and queue one
   coalesced `LiveEdge` inspection only when it changes.
7. During inspection, prefer projected selection. For `LiveEdge` with a target,
   fall back to the newest descriptor; reject an unchanged post-batch
   fingerprint as `no_progress`.
   Retain `LiveEdge` after repairing a projected descriptor, and downgrade a
   joined/start-reached continuation only when the completed descriptor was
   the live-edge fallback.
8. During repair completion, stop stale, deferred-zero, zero-event progress,
   and budget-exhausted live-edge attempts, reliably settle failure/incomplete
   state, release ownership, and do not immediately requeue.
9. Preserve the existing exact projection/render fence for positive outcomes.

## Task 3: GREEN regression coverage

**Files:**

- Modify `crates/koushi-core/src/timeline.rs` tests
- Modify `crates/koushi-sdk/tests/timeline_gap_adapter.rs`
- Add a focused integration fixture under `crates/koushi-core/tests/` if the
  current runtime test support can inject the required persisted topology;
  otherwise add a feature-gated Matrix SDK test constructor and drive the real
  actor against a mock-server-backed persisted event-cache topology.

1. Turn the Task 1 tests green.
2. Add a deterministic fixture containing older row, unprojected relation
   boundary, newest missing interval, and newer live row. Assert selection of
   the newest descriptor, exact-once ordered projection, and no historical-gap
   selection.
   The actor fixture must acknowledge the real initial projection and repaired
   desktop batch; helper-only sequencing is supplementary rather than the
   behavioral proof.
3. Retain existing tests that prove unprojected `Automatic` repair is idle and
   exact render ACK fences are unchanged.
4. Run:

   ```bash
   cargo test -p koushi-sdk --test timeline_gap_adapter
   cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
   cargo test -p koushi-core --test runtime_timeline
   ```

## Task 4: Canon and quality verification

1. Format and lint:

   ```bash
   cargo fmt --all -- --check
   cargo clippy -p koushi-sdk -p koushi-core --all-targets -- -D warnings
   ```

2. Run affected crate suites:

   ```bash
   cargo test -p koushi-sdk
   cargo test -p koushi-core --lib
   cargo test -p koushi-core --test runtime_timeline
   ```

3. Verify safety and diff hygiene:

   ```bash
   rg -n "remove_room|reset_room_timeline_cache" crates/koushi-core crates/koushi-sdk
   git diff --check
   git status --short
   ```

4. Review the complete diff against `REPOSITORY_RULES.md`, architecture,
   engineering policy, AGENTS.md, the approved design, and this plan. Resolve
   every actionable correctness/privacy finding and rerun affected gates.

## Task 5: Publish, CI, and merge

1. Commit the reviewed implementation with issue-closing context.
2. Rebase on the latest `origin/main` if needed and rerun focused gates.
3. Push `codex/issue-269-live-edge-gap-repair` and open a PR with `Closes #269`,
   design summary, RED/GREEN evidence, and verification commands.
4. Monitor GitHub Actions and review feedback; diagnose and fix every required
   failure, push updates, and wait for required checks.
5. Merge using a non-squash merge, confirm the PR state is `MERGED`, confirm
   #269 is closed, and verify `origin/main` contains the merge commit.
