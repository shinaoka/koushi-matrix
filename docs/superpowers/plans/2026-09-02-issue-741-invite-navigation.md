# Issue #741 invite navigation design

Status: **Correct-to-merge design** — `reviewer-flash` round 2. Round 1 findings were fixed before round 2; two round-2 non-blocking clarity notes are addressed below. `reviewer-flash` remains the selected independent reviewer (fast/low-cost, strong correctness review); `reviewer-gpt` is slower/costlier and strongest for broad architecture, while `reviewer-flash-opencode-go` is similar cost/speed but redundant with the selected model family.

## Reproduced contract gap

`acceptInvite` and `joinRoom` wait for the Rust command settlement and observe the joined room in the authoritative snapshot, then call the shared `selectRoom`. Both discard its boolean settlement and unconditionally switch `primaryView` to `timeline`. If composer draining refuses navigation or a newer room-navigation epoch supersedes the selection, the handler still renders the old committed room. The existing source-text test cannot detect this, and the browser fake pre-selects the joined room in the `accept_invite` response, bypassing the production `select_room` path.

## Ownership and minimal change

- Rust continues to own invite membership, room-list projection, `SelectRoom`, active-room/timeline state, request settlement, and correlated failure.
- React continues to own the pre-command room-navigation intent epoch and composer drain exactly as required by `docs/agents/state-ownership.md`.
- Add one small pure orchestration helper that accepts the authoritative joined-room list and the shared `selectRoom` callback. It returns `false` without dispatch when the joined room is absent, otherwise returns the exact `selectRoom` boolean.
- Move `selectRoom`'s renderer-only `setPrimaryView("timeline")` until after the typed `SelectRoom` settlement, epoch fence, and explicit authoritative snapshot guard (`active_room_id` and `timeline.room_id` both equal the target). This timing applies to every existing `selectRoom` caller, while normally being a visual no-op for callers already on the timeline. A refused, superseded, failed-no-op, or mismatched settlement returns `false` without exposing the prior timeline. Transport/command exceptions continue to propagate to existing callers; the helper does not normalize them into `false`.
- `acceptInvite` and `joinRoom` call the helper and perform no independent `setPrimaryView("timeline")`. `selectRoom` remains the sole shared owner of the timeline view transition.
- If membership settlement does not yet contain the joined room, staying on Invites/Explore is intentional: Rust-owned invite/room projections provide feedback and a later user selection remains available; React must not pretend navigation succeeded.
- `confirmDirectoryJoin`, direct-message creation, pinned-event navigation, and other sibling flows are not changed by #741. Their different settlement/selection contracts require separate reproduction before alteration; this PR fixes only the two handlers with the demonstrated guard-and-discard shape.
- Do not merge room/space epochs, synthesize membership, mutate authoritative navigation locally, or add retries/timeouts.

## Verify-first evidence

1. Replace the source-text assertion with behavioral unit tests: absent joined room does not dispatch; successful/failed `selectRoom` results are returned unchanged.
2. Add a deterministic negative browser case: `accept_invite` adds the room without pre-navigation and `select_room` returns an unchanged authoritative snapshot. Assert the Invites view remains visible and the previously committed room is not rendered. The old code switches to the prior timeline before/after the mismatched settlement, so this is a real RED wiring test.
3. Change the happy-path browser fake so `accept_invite` adds the joined room and removes the invite but deliberately leaves prior `active_room_id`/timeline untouched. Only the existing `select_room` fake may commit navigation.
4. Assert `select_room` is invoked with the newly joined room and the final authoritative snapshot/timeline selects it before any manual sidebar click.
5. Retain the existing invite workflow and DM assertions.
6. Run focused Vitest/Playwright RED→GREEN, full Vitest, typecheck, lint, build, relevant Rust invite/core QA gates, format/diff checks, and GitHub CI.

## Gate record

- Design review: `reviewer-flash` round 1 found one Important and two Minor issues; all were fixed. Round 2: **Correct-to-merge**.
- Verify-first RED: the deterministic browser case rendered the previous timeline when `select_room` returned an unchanged authoritative snapshot; the helper unit suite also failed on the missing module.
- Local GREEN: helper behavioral tests, full 99-file/1212-test Vitest, full 23-test invite/room/space Playwright file, typecheck, lint, and production build.
- Implementation diff review: `reviewer-flash` **Correct-to-merge**, no Critical/Important or actionable findings.
