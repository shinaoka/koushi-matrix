# Issue #806 read-receipt avatar refresh design

Status: **Correct-to-merge design** — `reviewer-flash` round 2. Round 1 found one Important and one Minor issue; both were fixed before round 2. `reviewer-flash` remains selected (fast/low-cost, strong independent correctness review); `reviewer-gpt` is slower/costlier and strongest for broad architecture, while `reviewer-flash-opencode-go` has similar cost/speed but is redundant with the selected model family.

## Root cause

`ReceiptReaders` and existing CSS already render a ready thumbnail inside the fixed 18 px circular receipt footprint and fall back to initials for all non-ready states. Rust enriches a receipt with a copied `AvatarImage`, but `handle_avatar_thumbnail_updated` updates profile, room, space, and invite copies only. Receipt copies under `AppState.live_signals.rooms[*].receipts_by_event[*].readers[*]` therefore remain pending and no `LiveSignalsChanged` effect reaches React.

## Ownership and minimal change

- Keep the existing `AvatarThumbnailUpdated` reducer as the sole owner of thumbnail settlement.
- Reuse its exact-MXC `update_avatar_thumbnail` helper for relevant-room profile mirrors and while walking every room, event summary, and visible reader in `live_signals`; a later receipt enrichment must not reintroduce a stale pending copy.
- Emit one `UiEvent::LiveSignalsChanged` only when at least one receipt copy actually changes. Preserve existing `ProfileChanged` and `RoomListChanged` ordering; unrelated MXCs and identical thumbnail states are inert.
- Do not add a TypeScript profile lookup, fetch, timer, component state, new DTO, or second receipt/avatar lifecycle.
- Main and thread timelines consume the same Rust-owned room/event receipt projection and `ReceiptReaders`, so one projection update covers both without separate state.

## Verify-first evidence

1. Rust reducer RED (the sole root-cause RED gate): seed a profile avatar plus multiple pending receipt avatars sharing one MXC across different rooms/events and an unrelated receipt MXC; keep room/space/invite avatar surfaces unrelated so `RoomListChanged` cannot appear. Dispatch `AvatarThumbnailUpdated(Ready)`; assert all matching readers become Ready, unrelated remains unchanged, and effects are exactly `[ProfileChanged, LiveSignalsChanged]`. Before the fix, receipt copies remain pending and the live-signal effect is absent.
2. Rust reducer coverage dispatches duplicate Ready and an unrelated MXC and asserts both are effect-free for live signals.
3. Browser renderer regression GREEN proof (not a pre-fix RED): inject Rust-shaped snapshots to render a receipt pending, then Ready, without remounting the timeline; assert initials change to `<img>` and the marker remains 18 px/circular. Exercise both main and thread containers or their shared `ReceiptReaders` surface. Existing code already renders a directly injected Ready DTO, so this proves the wire/render contract while the Rust test proves the bug fix.
4. Existing renderer coverage continues to prove missing/Failed/NotRequested avatars use initials.
5. Run focused RED→GREEN, full `koushi-state`, full Vitest, targeted Playwright, typecheck, lint, build, format/diff/boundary/secret guards, and GitHub CI.

## Canon update

Update `docs/architecture/state-machine.md` and `docs/agents/state-ownership.md` to state that `AvatarThumbnailUpdated` refreshes matching receipt copies by exact MXC and emits `LiveSignalsChanged`; React remains render-only. Replace the stale state-machine paragraph claiming the explicit thumbnail workflow is future work and enumerate receipt readers among the settled avatar surfaces. State explicitly that the existing `AvatarThumbnailUpdated` action—not a new action—settles these copies.

## Gate record

- Design review: `reviewer-flash` round 1 found one Important and one Minor finding; both fixed. Round 2: **Correct-to-merge**.
- Verify-first RED: focused `profile_state` test received only `ProfileChanged`; both receipt copies remained pending and `LiveSignalsChanged` was absent.
- Local GREEN so far: focused and full `profile_state`, full `koushi-state`, and targeted browser pending→Ready→Failed in-place receipt rendering with fixed geometry.
- Implementation diff review: `reviewer-flash` **Correct-to-merge** with three Minor findings. All were fixed: explicit DOM-node preservation, neutral room/event fixture names, and exact-MXC settlement of relevant-room profile mirrors plus exhaustive canon wording. Final re-review: **Correct-to-merge**, no remaining findings.
