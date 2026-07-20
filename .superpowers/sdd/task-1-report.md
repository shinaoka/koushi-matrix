# Task 1 report: SDK diffs projected into display index space

## Result

Implementation commit: `0598615` (`fix(timeline): project SDK diffs into display index space`).

Core now owns one stateful SDK-to-display projection transaction. It advances
the canonical timeline once, retains the exact pre-normalization canonical
slots represented by the bounded display, derives one authoritative normalized
display, runtime-validates the emitted display-space diffs, and publishes only
those diffs to `TimelineEvent::ItemsUpdated`.

The same state is used for bounded replay ownership, non-SDK item revisions,
relay recovery, and send-queue resync. Restore buffering stores projected
display diffs while canonical state advances immediately. The actor-generation
lease fences projection commit, `ItemsUpdated`, and replay-known reconciliation;
it is released before unrelated thread-attention and live-edge calculations.

## RED evidence

The production-shaped regression was added first and run with:

```text
cargo test -p koushi-core --lib sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo -- --nocapture
```

It failed before the production change because the canonical `Set` at index
9,039 was applied to a roughly 120-row display sequence and was ignored. The
transaction local echo therefore survived beside the confirmed event after the
following canonical `Remove(9039)` and `PushBack Event` batch.

Additional RED coverage was then added for duplicate ownership, out-of-window
operations, boundary insertion, live-edge and historical-edge growth,
clear/reset, invalid translation, restore buffering, and stale generations.
Before the remaining production integration, seven projection tests passed and
the restore/generation cases failed for the missing behavior.

## GREEN evidence

Fresh focused gates after the final self-review change:

- Production-shaped Core regression: 1 passed, 0 failed (16.34 s including
  recompilation); the ordinary-path fallback counter remained unchanged.
- `cargo test -p koushi-core --lib display_projection -- --nocapture`: 9 passed,
  0 failed (0.08 s).
- `npm --prefix apps/desktop test -- src/domain/timelineStore.test.ts
  --reporter=dot`: 68 passed, 0 failed (0.20 s).
- `cargo check -p koushi-core`: passed without warnings (3.36 s).
- `npm --prefix apps/desktop run typecheck`: passed (2.97 s).
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.
- Impacted non-SDK and authoritative-recovery focused tests: 3 passed, 0
  failed.

The requested `cargo fmt --all -- --check` did not pass because clean,
out-of-scope files already present under `vendor/matrix-rust-sdk` differ from
the workspace rustfmt result. Stable rustfmt reported
`event_cache/redecryptor.rs` and the SDK event-cache integration test; nightly
rustfmt also reported pre-existing SDK event-cache source formatting. The
vendor repository remained clean and was not modified. The four Task 1 files
produce no rustfmt diff.

No long homeserver/integration lane was run, as requested.

## Projection and fallback behavior

All canonical diff variants are handled by `project_sdk_batch`. `Set` and
`Remove` capture the old canonical owner before mutation. Insertions use
displayed canonical neighbors and explicit prepend/append policy instead of
reusing a canonical index. Reset rebuilds the same bounded live-edge window as
fresh replay; restore temporarily disables that cap so historical prepends are
retained.

The release-path validator replays every output operation with the desktop's
render-identity normalization and checks both numeric bounds and final equality
with Core's authoritative display. Ambiguous canonical input or invalid output
emits `Reset { items: display_after }`. It increments the stable
`display_projection_reset_fallbacks` counter and records only fixed diagnostic
tokens and the count. The deliberate invalid-index test exercised this
fallback; ordinary production-shaped flows observed no fallback.

Projection work never scans the full canonical sequence for each diff. It
updates only explicit bounded-window membership and the current batch; Reset
scans its replacement payload once. Structural operations can visit the
bounded membership (normally capped at 120 slots) for each operation in a
multi-operation batch, so the current strict worst case is
`O(batch * display-window)` rather than the aspirational `O(batch +
display-window)`. It remains independent of the roughly 9,000-slot canonical
history on the live hot path.

## Self-review

Reviewed the full diff against `REPOSITORY_RULES.md`,
`docs/architecture/overview.md`, `docs/architecture/state-machine.md`,
`docs/policies/engineering-rules.md`, `AGENTS.md`, issue #285, its approval
comment, and the Task 1 brief.

Confirmed:

- Rust remains the owner of Matrix/index-domain semantics; no frontend repair
  heuristic or body/timestamp/sender matching was added.
- No raw canonical SDK diff is sent to `ItemsUpdated` or restore buffering.
- Numeric indices are documented in Rust and TypeScript as display-relative.
- Explicit duplicate canonical owners survive normalization until their last
  owner leaves membership.
- Generation rejection occurs before either canonical or display state commits.
- Replay-known lifecycle publication shares the same generation lease as the
  display update.
- Diagnostics contain no room, event, sender, transaction, body, URL, path, or
  raw-error data.
- The former raw display reducer and model helpers are test-only; no second
  production projection implementation remains.
- Repeated synthetic fixtures were consolidated into one helper, and
  `cargo check` introduced no `cfg(test)` or dead-code warnings.

## Remaining concerns

- Workspace-wide rustfmt is blocked by the pre-existing clean vendor baseline
  described above.
- The bounded structural bookkeeping is independent of canonical-history size
  but does not yet achieve a strict additive `O(display window + batch size)`
  bound for unusually large multi-operation batches.
- The intentionally deferred long homeserver lane still needs to be run once
  after parent review.

## PAUSE CHECKPOINT — 2026-07-20

The overflow-recovery atomicity slice is complete. A stale actor is rejected
before relay teardown and is fenced again after the SDK subscription await.
The recovery generation, batch-id reset, projected-gap reset, authoritative
canonical/display window, media-source cache, `ResyncRequired`, `InitialItems`,
and replay-known ownership now commit under one actor-generation lease.

Fresh checkpoint evidence:

- `cargo test -p koushi-core --lib authoritative_resync_projects_event_only_and_emits_ordered_recovery_events -- --nocapture`: 1 passed.
- `cargo test -p koushi-core --lib stale_generation_recovery_does_not_commit_candidate_or_publish -- --nocapture`: 1 passed.
- `cargo test -p koushi-core --lib relay_overflow_recovery_subscribes_once_emits_snapshot_then_next_live_update -- --nocapture`: 1 passed.
- `cargo check -p koushi-core`: passed without warnings.
- `cargo fmt -p koushi-core -- --check`: passed after applying package formatting.
- `git diff --check`: passed.

Also completed before this pause: the restore terminal now publishes its
coalesced projected `ItemsUpdated`, replay-known lifecycle, changed navigation,
and `AnchorRestoreFinished` under one lease. A causal projection whose display
transition normalizes to a no-op still emits an empty `ItemsUpdated` render
fence. The exact projection-overhead contract is documented as expected
`O(W + B log(W+B) + D)` time and `O(W+B+D)` temporary space, with `W <= 120`
at the Room live edge and `D = O(W)` for the private builder; canonical Vec
mutation costs are explicitly outside that bound. The operational test counter
now measures only visible-payload visits and no longer claims strict additive
tree work.

Remaining after resume:

1. Re-run the full focused display/restore/timeline unit set after the latest
   formatting and inspect any failures.
2. Run the pending TypeScript TimelineStore regression and desktop typecheck.
3. Consolidate the still-large implementation diff and remove any remaining
   superseded helpers/tests without weakening the projection/lease contracts.
4. Run the requested focused gates, package format/diff checks, and independent
   review; update this report with final GREEN evidence and commit SHA.
5. Leave the long homeserver lane to the root agent's single post-review run;
   do not create/push a PR from this worker.
