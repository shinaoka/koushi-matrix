# Headless Core Runtime Design Review

Date: 2026-06-12
Reviewer: Claude (deepseek-v4-pro)
Scope: design review of `docs/superpowers/specs/2026-06-12-headless-core-runtime-design.md`
        against current implementation, `docs/architecture/overview.md`, and
        vendored matrix-rust-sdk

## Reviewed Artifacts

- `docs/superpowers/specs/2026-06-12-headless-core-runtime-design.md` (the spec)
- `docs/architecture/overview.md` (normative architecture, amends the spec)
- `docs/architecture/state-machine.md`
- `docs/architecture/desktop-foundation.md`
- `docs/architecture/search-adapter.md`
- `crates/matrix-desktop-sdk/src/lib.rs` (current SDK adapter)
- `crates/matrix-desktop-state/src/` (pure state crate)
- `apps/desktop/src-tauri/src/commands.rs` (2038-line monolithic Tauri backend)
- `vendor/matrix-rust-sdk/` (vendored SDK, 10 crates)

## Summary

The spec defines an in-process actor system (`matrix-desktop-core`) with 7 actor
types, a `CoreCommand`/`CoreEvent` public boundary, and a state-projection
pipeline. The direction is correct: the current 2038-line `commands.rs`
monolith must be decomposed, and the security/QA boundaries need enforcement.
However, the actor granularity is likely excessive, and the spec has meaningful
drift from the normative `overview.md` that should be resolved before
implementation.

## Findings

### Important

1. **Actor granularity is excessive for the current system size.**
   Seven actor types with per-room/thread instances is a lot for a single-user
   Matrix client. matrix-rust-sdk already provides `SyncService`,
   `RoomListService`, `Timeline` (with subscription streams), and a send queue
   --- the actors largely wrap existing SDK abstractions. Consider starting
   with 3--4 actors (AppActor, AccountActor encompassing sync/rooms/timeline,
   StoreActor) and splitting only when concrete pain emerges.

2. **The spec and overview.md have unresolved drift.**
   `overview.md` amends the spec's public API in significant ways:
   `TimelineKey` addressing, `request_id` correlation, diff-based timeline
   updates, SDK send queue usage, sync capability probing with `LegacySync`
   fallback, and fail-closed `LocalEncryptionUnavailable`. These amendments
   are listed in overview.md's "Relationship to Dated Specs" section but are
   not integrated into the spec body. Implementation will be confused by the
   dual sources of truth.

3. **Actor supervision strategy is undefined.**
   The spec defines shutdown ordering but not crash recovery. If `SyncActor`
   panics, who restarts it? If `TimelineActor` hangs, how is it detected? An
   actor system without supervision is incomplete.

4. **State projection has high ceremony for simple operations.**
   `CoreCommand → actor → CoreEvent → AppAction → reduce → StateChanged` is 6
   hops. For a simple "send text" operation this is heavy. The spec argues
   this is necessary for deterministic UI state, which is valid for
   state-changing operations, but the overhead should be acknowledged and
   possibly short-circuited for pure queries.

5. **`matrix-desktop-auth` naming is misleading.**
   The crate already does sync, room operations, timeline, and search beyond
   authentication. The spec acknowledges this ("can later be renamed to
   `matrix-desktop-sdk`") but defers the rename. Renaming during Milestone A
   would reduce long-term confusion at minimal cost.
   Resolved in Phase 9 cleanup: the crate is now `matrix-desktop-sdk`.

6. **Concrete types and constants are missing.**
   `TimelineKey`, `RequestId`, channel capacities, and backpressure recovery
   procedures are referenced but not defined. These should be specified before
   Milestone A implementation begins.

7. **Wasm portability path is aspirational, not concrete.**
   `overview.md` mandates executor abstraction and wasm-clean pure crates, but
   the current `matrix-desktop-sdk` uses `tokio::runtime::Builder` and
   `block_on` extensively. The spec should acknowledge which parts of the
   migration will resolve this and which are deferred.

### Minor

1. **`CoreCommand` enum is monolithic.** The sub-enum pattern (AppCommand,
   AccountCommand, etc.) helps, but the top-level enum still requires
   exhaustive matching everywhere. Consider whether command routing could use
   a trait-based approach internally while keeping the enum for the public
   boundary.

2. **RoomActor and TimelineActor coordination is unspecified.** Does
   RoomActor spawn TimelineActors? Do they share state or communicate via
   channels? The relationship between these actors should be diagrammed.

3. **Unsubscribe lifecycle needs resource bounds.** Rule 7 in overview.md
   adds explicit subscribe/unsubscribe, which is good, but there is no bound
   on how many concurrent timeline subscriptions can exist. A user with
   hundreds of rooms needs a defined policy.

## Strengths

The following aspects of the design are well-conceived and should be preserved:

- **Layer separation**: pure state / SDK adapter / runtime orchestration /
  transport / presentation is the right decomposition.
- **Security model**: fail-closed encryption, redacted Debug, coarse public
  failures, webview as least-trusted layer. Comprehensive and production-ready.
- **Headless-first workflow**: GUI-free QA via `CoreCommand`/`CoreEvent`
  against local homeservers is the correct approach for both correctness and
  development speed.
- **QA hierarchy**: unit → local homeserver → real homeserver → GUI smoke is
  properly layered with clear gate criteria.
- **SDK respect**: the Async Design Rules correctly mandate relaying the SDK
  rather than reimplementing its sync/pagination/send-queue internals.

## Recommendation

Proceed with the design direction, but resolve the following before Milestone A
implementation:

1. Integrate overview.md amendments into the spec (TimelineKey, request_id,
   diffs, send queue, sync probing, LocalEncryptionUnavailable).
2. Reduce initial actor count to 4 (AppActor, AccountActor, SearchActor,
   StoreActor); split AccountActor later if needed.
3. Define supervision strategy (panic recovery, hang detection).
4. Rename `matrix-desktop-auth` to `matrix-desktop-sdk`.
5. Define concrete types for `TimelineKey`, `RequestId`, and channel capacities.
