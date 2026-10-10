# Issue #1255 — Thread pane receipt scopes and stale readers

## Evidence and scope

The report: the thread pane mixes room and thread read-receipt scopes and keeps
stale readers. Two independent root causes reproduce it, both with synthetic
identifiers only.

1. The Room timeline publishes unthreaded receipts while the Thread timeline
   publishes that root's threaded receipts, and both wrote
   `LiveRoomReceiptSummariesUpdated { room_id, .. }` into one map keyed by event
   ID. The later publisher therefore overwrote the earlier readers for the same
   event, so a thread reader could appear as a room reader and vice versa.
2. A timeline actor's start merged its initial receipt snapshot instead of
   replacing its own window, so a summary left by a retired actor survived on an
   event that the new actor's window still contained.

Canon consulted: `REPOSITORY_RULES.md` (Canon-First, Root-Cause Fix Discipline,
State-Machine Discipline, Architecture And Ownership, Concurrent Work/Test
Placement, Review And Audit), `docs/agents/verification.md`, and the amended
`docs/architecture/state-machine.md` "Live Signals" section, which now defines
the receipt scopes. That amendment is part of this change and its wording is
matched exactly by the implementation and tests.

## Design

- `koushi_state::ReceiptScope` is `Main` (default) | `Focused { event_id }` |
  `Thread { root_event_id }`. The timeline key, not the SDK's runtime focus
  resolution, is authoritative: `receipt_scope_for_timeline_kind` maps Room to
  `Main`, Thread to its root's thread scope, and Focused to its own focused
  scope.
- Storage is separate: `RoomLiveSignals.receipts_by_event` (main),
  `focused_receipts_by_event[event_id]`, and
  `thread_receipts_by_event[root_event_id]`. Two actors that observe the same
  event ID in different scopes cannot overwrite each other.
- `LiveRoomReceiptSummariesUpdated` and `LiveRoomReceiptsWindowReconciled` carry
  `scope` plus `scoped_event_ids`. A live incremental diff passes an empty
  `scoped_event_ids` and therefore only merges; an actor start passes its initial
  window and removes those entries in its own scope before merging, even when the
  initial snapshot has no receipt entries.
- Why Focused is not pinned to `Main`: the vendored SDK resolves
  `TimelineFocusKind::receipt_thread()` to `Thread(root)` when the focused target
  is a thread reply and to `Unthreaded` otherwise. Pinning Focused to `Main`
  would publish that timeline's threaded receipts into the main scope whenever a
  permalink target is a thread reply — the same contamination class this issue
  fixes. Focused therefore has its own scope, and a focused permalink on a thread
  reply can never change the thread root's scope. The SDK mapping is recorded
  here because it is the reason the scope is derived from the timeline key rather
  than from the resolved receipt thread.
- The wire adds two sibling delta slices,
  `live_signals_focused_receipts_by_room_event` and
  `live_signals_thread_receipts_by_room_event` (room → scope key → event →
  replacement, `null` removes). The GUI merges each into its own map and each
  pane reads only the scope of its own timeline key, with no cross-scope
  fallback.

## Changes

- `crates/koushi-state/src/state/live_signals.rs`: add `ReceiptScope`, the two
  scoped maps, scope accessors, and count-only `Debug` output.
- `crates/koushi-state/src/state/mod.rs`, `crates/koushi-state/src/lib.rs`:
  export `ReceiptScope`.
- `crates/koushi-state/src/action.rs`: add `scope`/`scoped_event_ids` to both
  receipt actions.
- `crates/koushi-state/src/reducer/live_signals.rs`: remove the scoped ids in the
  action's scope, then merge/replace there; read prior readers from that scope.
- `crates/koushi-state/src/reducer/mod.rs`: dispatch the new fields.
- `crates/koushi-state/src/reducer/avatar.rs`, `.../profile.rs`: read the main
  scope through the new accessor.
- `crates/koushi-state/src/reducer/tests.rs`: pass `Main`/empty scope in existing
  fixtures.
- `crates/koushi-state/tests/thread_receipt_scope.rs`: new RED-first suite (7
  tests) for cross-scope isolation and actor-start stale-reader removal.
- `crates/koushi-state/tests/{avatar_thumbnail_convergence,profile_state,space_members_state}.rs`:
  pass the main scope in fixtures.
- `crates/koushi-core/src/timeline/item_projection.rs`: derive the scope from the
  timeline key, thread it through both receipt observation targets and action
  builders, and add the SDK-window event-id helper.
- `crates/koushi-core/src/timeline/item_projection/receipt_scope_tests.rs`: new
  Core-level scope suite (5 tests).
- `crates/koushi-core/src/timeline/actor.rs`: the actor start reconciles its own
  scope over its own initial window.
- `crates/koushi-core/src/timeline/relay.rs`: authoritative reconciliation and
  live diffs carry the actor's scope.
- `crates/koushi-core/src/timeline/read_state/tests.rs`, `.../relay/tests.rs`,
  `crates/koushi-core/src/runtime/tests.rs`: pass the main scope in fixtures.
- `crates/koushi-core/src/runtime/reducer_support.rs`: include scoped removals in
  receipt source changes.
- `crates/koushi-core/src/runtime/profile_display_diagnostics.rs` and its new
  sibling `profile_display_diagnostics/tests.rs`: the inline module moved out
  because the two required `scope` fields pushed it to the 200-line inline test
  ceiling (Test Placement). No test logic changed.
- `crates/koushi-core/src/state_delta.rs`: compute both scoped slices.
- `crates/koushi-core/tests/receipt_scope_delta.rs`: new delta suite (3 tests).
- `crates/koushi-protocol/src/state_update.rs`, `.../lib.rs`: add and export
  `ScopedReceiptSummaryChanges` and the two delta fields.
- `apps/desktop/src-tauri/src/dto.rs`, `.../dto/tests.rs`: mirror both slices,
  include them in `is_empty`/merge, and test them at the frontend boundary.
- `apps/desktop/src-tauri/tests/golden/frontend_app_state.json`: regenerated.
- `apps/desktop/src/domain/types.ts`, `coreEvents.ts`, `appStore.ts`: mirror the
  fields and merge each scope into its own map (null removes).
- `apps/desktop/src/components/TimelineView.tsx`: each timeline reads receipts
  from its own scope.
- `apps/desktop/src/test/appHarnessMain.tsx`: the receipt reader harness reads
  the source's own scope.
- `apps/desktop/src/domain/appStore.test.ts` and the four
  `TimelineView.*.test.tsx` / `receiptReader.integration.test.tsx` fixtures:
  supply the new maps; a new store test asserts scopes do not cross.
- `docs/architecture/overview.md`: name the two new scoped delta slices.
- `docs/architecture/state-machine.md`: the Live Signals scope amendment (part of
  this change).

## Verification

RED, before implementation, with synthetic identifiers only:

- Behavioral probe against the pre-change API
  (`red_probe_room_reader_is_lost_when_the_thread_actor_publishes_last` and
  `red_probe_retired_reader_survives_an_actor_start_merge`): the room reader was
  replaced by the thread reader (`left: "@thread-reader:example.invalid" right:
  "@room-reader:example.invalid"`) and the retired reader survived
  (`assertion failed: !...receipts_by_event.contains_key(REPLY_EVENT_ID)`),
  exit 101.
- The final API was RED first: `crates/koushi-state/tests/thread_receipt_scope.rs`
  failed to compile with 6 errors — unresolved `ReceiptScope`, missing `scope` /
  `scoped_event_ids` fields, and missing scoped maps — exit 101.

GREEN gates, all from
`.worktrees/burn-2026-10-10` (detached HEAD `10771b0c`, SDK `55e51ffe8`):

| Command | Exit |
| --- | --- |
| `cargo test -p koushi-state` | 0 |
| `cargo test -p koushi-core` | 0 |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib` | 0 |
| `cargo fmt -p koushi-state -p koushi-core -p koushi-protocol --check` | 0 |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --check` | 0 |
| `cargo clippy -p koushi-state -p koushi-core -p koushi-protocol -p koushi-sdk --all-targets --locked -- -D warnings` | 0 |
| `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings` | 0 |
| `npm --prefix apps/desktop run typecheck` | 0 |
| `npm --prefix apps/desktop run lint` | 0 |
| `npm --prefix apps/desktop test` (1843 tests, 155 files) | 0 |
| `node scripts/check-rust-test-structure.mjs` | 0 |
| `node scripts/check-agents-docs.mjs` | 0 |
| `node scripts/check-sdk-submodule.mjs` | 0 |
| `git diff --check` | 0 |

## Notes

- `apps/desktop/node_modules` was installed in the worktree with
  `npm ci --offline --ignore-scripts` to run the frontend gates; it is ignored by
  git and is otherwise untracked. The temporary RED probe file was deleted after
  capturing the failure. No other temporary files remain in the worktree.
- The main scope keeps its existing semantics everywhere; only focused and thread
  timelines changed behavior.
- No push and no PR: the change is committed locally only.
