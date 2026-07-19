# Issue #285 Display Diff Projection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure every SDK timeline batch emitted to the desktop is expressed in the bounded display index domain, so a confirmed text or media send cannot leave a stale `Sending` transaction row.

**Architecture:** Keep `navigation_items` as Core's full canonical sequence and keep the desktop TimelineStore bounded. Replace the current reuse of canonical indices with one Core-owned projection transaction that applies canonical diffs once, advances explicit bounded-display membership/mirror state, and emits display-space diffs whose application exactly equals the authoritative display result. Runtime validation falls back to a display `Reset` and records a private-data-free diagnostic when incremental translation is ambiguous.

**Tech Stack:** Rust, Matrix SDK `VectorDiff`, Koushi Core timeline actor, TypeScript TimelineStore contract, Rust unit tests, Vitest.

## Global Constraints

- GitHub issue [#285](https://github.com/shinaoka/koushi-matrix/issues/285), including its current body and review comment, is the approved design specification.
- Every `ItemsUpdated` batch is exclusively in the display index domain and transforms the previously emitted display sequence into Core's new authoritative display sequence.
- Preserve enough pre-normalization membership state for canonical/display offsets, duplicate render identities, `Transaction -> Event` identity changes, and bounded-window entry/exit.
- Apply SDK canonical diffs exactly once.
- Validate numeric display indices in release builds, not only debug assertions.
- If incremental projection is ambiguous or invalid, emit display `Reset { items: display_after }` or use the existing resync path; never silently drop the transition.
- Record a private-data-free `display_projection_reset_fallbacks` diagnostic and keep it zero in the normal focused/headless path.
- Projection work is `O(display window + batch size)` per batch; never scan the full canonical sequence per diff.
- Anchor-restore buffering stores projected display diffs, not raw canonical-index diffs.
- Preserve the actor-generation lease across display-mirror commit, `ItemsUpdated`, and replay-known reconciliation.
- Do not match body, timestamp, sender, formatted content, or media metadata to reconcile identity.
- Do not move the full canonical timeline into the desktop store, remove bounded replay, change Matrix SDK semantics, or redesign send-queue UX.
- Follow strict RED -> GREEN -> refactor order. Run long-duration QA only once after the coherent implementation and review are complete.
- Diagnostics, fixtures, and reports must contain synthetic/private-data-free values only.

---

### Task 1: Project canonical SDK batches into one validated bounded-display transaction

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/src/event.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Test: `crates/koushi-core/src/timeline.rs`
- Test: `apps/desktop/src/domain/timelineStore.test.ts`

**Interfaces:**
- Consumes: SDK-derived `Vec<TimelineDiff>`, pre-batch `navigation_items`, current bounded display membership/mirror, viewport/replay window policy, actor generation lease.
- Produces: updated canonical state, updated bounded display membership/mirror, and validated display-space `Vec<TimelineDiff>` for `TimelineEvent::ItemsUpdated`.
- Invariant: applying the emitted display diffs to `display_before` yields exactly `display_after` after every batch.

- [ ] **Step 1: Add the production-shaped failing Core regression**

Add a unit test named `sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo` beside the existing timeline display-mirror tests. Construct at least 9,040 synthetic canonical slots, select a roughly 120-row live-edge display, append a transaction, then apply canonical-index `Set`, followed by canonical-index `Remove` plus `PushBack Event`. Apply every emitted display batch to a separate desktop-model vector and assert after each batch:

```rust
assert_eq!(desktop_model, projection.display_items());
assert!(projection
    .display_items()
    .iter()
    .all(|item| !matches!(item.id, TimelineItemId::Transaction { .. })));
assert_eq!(
    projection
        .display_items()
        .iter()
        .filter(|item| timeline_item_event_id(item) == Some("$confirmed:test"))
        .count(),
    1
);
```

Use only synthetic IDs and bodies. The test must exercise indices around 9,039 while the display contains about 120 rows.

- [ ] **Step 2: Run the new test and capture the expected RED**

Run:

```bash
cargo test -p koushi-core --lib sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo -- --nocapture
```

Expected: FAIL because a canonical `Set`/`Remove` index cannot update the bounded display and the transaction survives beside the confirmed event. Record the command and failure reason in the task report before editing production code.

- [ ] **Step 3: Introduce one projection transaction and make the production-shaped test GREEN**

Replace the raw `apply_timeline_diffs_to_display_items` SDK path with a stateful projection boundary. The implementation may choose internal names, but it must expose semantics equivalent to:

```rust
struct DisplayProjectionBatch {
    display_after: Vec<TimelineItem>,
    display_diffs: Vec<TimelineDiff>,
    used_reset_fallback: bool,
}

fn project_sdk_batch(
    canonical_items: &mut Vec<TimelineItem>,
    display_state: &mut DisplayProjectionState,
    canonical_diffs: &[TimelineDiff],
    context: &DisplayProjectionContext,
) -> DisplayProjectionBatch;
```

The transaction must retain explicit canonical-slot/window membership information rather than attempting to reconstruct discarded duplicates from the normalized display vector. Read old canonical identities before `Set` and `Remove`. Advance canonical state exactly once. Emit only indices valid against the evolving display sequence.

Run the focused test again. Expected: PASS with one confirmed event and no transaction.

- [ ] **Step 4: Add RED coverage for every diff variant and exceptional fallback**

Add focused tests covering:

```text
duplicate canonical identity removal with another owner retained
out-of-window Set / Remove / Insert / Truncate
boundary-adjacent Insert
live-edge PushBack
backward-pagination PushFront
Clear and Reset
Transaction -> Event identity-changing Set
anchor-restore buffering and flush
stale actor-generation rejection
invalid or ambiguous incremental translation -> Reset fallback
```

For each test, apply emitted display diffs to a separate model and assert:

```rust
assert_eq!(apply_display_diffs(display_before, &display_diffs), display_after);
```

Run the new focused test filter before adding the remaining production logic. Expected: at least one new test FAILS for missing behavior rather than a compile/setup error.

- [ ] **Step 5: Complete all projection semantics, runtime validation, restore integration, and diagnostics**

Implement all variants under the single projection boundary. Runtime-validate each numeric index against the display state immediately preceding that operation. On ambiguity or invalid output, replace the incremental batch with:

```rust
vec![TimelineDiff::Reset {
    items: display_after.clone(),
}]
```

Record one private-data-free diagnostic for fallback use with a stable token/counter named `display_projection_reset_fallbacks`; do not record IDs, bodies, room identifiers, or raw errors. Ensure ordinary production-shaped and focused headless flows leave this counter at zero.

Change `restore_emit_buffer` to accumulate projected display diffs while canonical state advances immediately. The final flush must transform the desktop's pre-restore display into the authoritative post-restore display. Keep the existing actor-generation lease as the atomic publication/replay-known boundary.

The hot path may scan the bounded display and current batch, but must not scan all canonical items for every diff. Maintain slot/window membership bookkeeping needed to meet `O(display window + batch size)` per batch.

- [ ] **Step 6: Document the wire contract and keep frontend defenses non-authoritative**

Update the Rust `TimelineEvent::ItemsUpdated` and TypeScript `ItemsUpdated` comments to state:

```text
All numeric TimelineDiff indices are relative to the desktop display sequence
immediately before that operation, never to Core's full navigation sequence.
```

Retain frontend bounds checks as defenses. Do not add body/timestamp matching or a frontend-only stale-transaction cleanup. Add a TypeScript regression combining an earlier collapsed duplicate with a later transaction/event transition so the reducer contract remains pinned.

- [ ] **Step 7: Run focused GREEN gates and review the finished diff**

Run:

```bash
cargo test -p koushi-core --lib sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo
cargo test -p koushi-core --lib display_projection
npm --prefix apps/desktop test -- src/domain/timelineStore.test.ts --reporter=dot
cargo check -p koushi-core
npm --prefix apps/desktop run typecheck
cargo fmt --all -- --check
git diff --check
```

Expected: all commands exit 0; no warnings/errors introduced; fallback diagnostics remain zero on ordinary paths. Do not run a long homeserver lane yet.

- [ ] **Step 8: Self-review and commit**

Review the full diff against `REPOSITORY_RULES.md`, `docs/architecture/overview.md`, `docs/architecture/state-machine.md`, `docs/policies/engineering-rules.md`, `AGENTS.md`, issue #285, and this plan. Confirm no raw canonical index reaches `ItemsUpdated`, no duplicate projection implementation was added, and no private data enters diagnostics.

Commit:

```bash
git add crates/koushi-core/src/timeline.rs crates/koushi-core/src/event.rs \
  apps/desktop/src/domain/coreEvents.ts apps/desktop/src/domain/timelineStore.test.ts
git commit -m "fix(timeline): project SDK diffs into display index space"
```

Write the implementation report with RED evidence, GREEN commands/results, commit SHA, self-review findings, remaining concerns, and whether any fallback occurred.
