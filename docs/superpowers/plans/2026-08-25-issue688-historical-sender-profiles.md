# Issue #688 — historical sender-profile hydration

## Problem

A newly joined room can paginate events whose senders were never observed in the
live sync window. The SDK timeline then exposes `sender_profile = Unavailable`,
so Core projects no `sender_label` and React renders “Unknown user”, even though
the room member store and People panel know the display name.

## Contract boundary

The product Room/Thread/Focused timelines are built directly by
`TimelineManager` and subscribed by `TimelineActor`; the similarly named
`koushi-sdk` helpers are Activity/QA adapters and do not project sender labels.
The existing SDK `Timeline::fetch_members()` is the intended repair: it calls
the deduplicated/no-op-when-synced `Room::sync_members()`, updates missing
profiles, and publishes ordinary timeline diffs. No vendor, public SDK wrapper,
DTO, state, persistence, Browser Fake, or React change is allowed.

Element X precedent is authoritative when equivalent:

- Android starts `fetchMembers()` in the timeline-owned coroutine for Live and
  FocusedOnEvent modes, after construction, without blocking first paint.
- iOS installs its timeline provider/listener and then starts
  `timeline.fetchMembers()` asynchronously.

## Design

After `TimelineActor` has subscribed to the SDK timeline and started its diff
relay, start one existing actor-owned auxiliary task that awaits
`timeline.fetch_members()` for `TimelineKind::Room` and `TimelineKind::Focused`.
The existing auxiliary-task teardown aborts it when the actor retires. Thread
actors do not start another fetch, matching Element X Android; room membership
is room-scoped and the live Room timeline hydrates it before ordinary thread
navigation. SDK request deduplication and `are_members_synced()` prevent repeated
network work.

Failures remain the SDK's closed `Pending` → `Error` profile state and continue
to render the existing fallback; no raw error crosses the boundary. A later
actor may retry through the same SDK path.

## Verify first

Add a Matrix mock integration test through `TimelineActor::spawn`:

1. create a joined room with a historical event from a sender whose member event
   is absent from the local store;
2. mount exactly one `/members` response containing that sender's display name;
3. assert the initial Core item has no sender label;
4. without any live sender event, require an unchanged `ItemsUpdated` Set for
   the same event with the real display name.

Before production wiring the test must time out/fail because no `/members`
request occurs. After wiring the same test must pass. Add a small policy test
that Room and Focused hydrate while Thread does not.

## Validation

- focused RED/GREEN test and complete `cargo test -p koushi-core --lib`;
- complete workspace all-targets and Tauri/state/SDK applicable Rust tests;
- rustfmt, clippy/lint-equivalent repository hooks, SDK submodule, agents-doc,
  diff, secret/boundary/generated checks;
- frontend typecheck/lint/Vitest/build and Playwright because the PR already
  changes root agent guidance and current-head CI is the merge authority;
- real `timeline_basic` or the closest existing both-server timeline lane must
  assert a pre-join historical sender label without a live repair event.

## Implementation evidence

- RED: `room_actor_hydrates_a_historical_sender_without_a_live_event` timed out
  after two seconds because no `/members` request or profile Set occurred
  (`/tmp/issue688-red.log`, exit 101).
- GREEN: the unchanged test observed initial `sender_label = None`, exactly one
  mocked `/members` request, then an ordinary Set for the same event with
  `sender_label = "Carol"`, without a live Carol event.
- Core lib: 1,092 passed / 8 ignored. Workspace all-targets: 2,568 passed /
  13 ignored. QA binary: 133 passed. Wasm, Tauri check, frontend typecheck,
  lint, 1,497 Vitest tests, build, SDK/diagnostic/docs checks, cargo-deny, and
  diff checks passed.
- The existing `timeline` real-runtime lane passed on both tuwunel and synapse.
  The exact missing-profile transition remains in the deterministic Matrix mock:
  reproducing the same pre-join lazy-member condition in the broad scenario
  would add three-party/history fixture machinery without exercising a new
  product boundary.
- Independent `reviewer-flash` review traced subscription/relay ordering,
  buffered delivery, auxiliary-task teardown, scope, test assertions, and docs,
  and returned `Correct-to-merge`. It noted only the nonblocking pre-existing
  direct-thread-deep-link case; the selected Room/Focused policy intentionally
  matches Element X Android.
- `cargo machete --with-metadata` reports the repository's pre-existing unused
  dependency baseline identically on `origin/main`; this diff changes no
  manifest or dependency.

## Stop conditions

Do not add a Koushi profile cache, per-row lookup, frontend inference, SDK
extension, correctness sleep, unbounded retry, or full-members request on every
pagination. If the existing SDK path does not emit a normal diff, stop and
redesign rather than introducing a second semantic owner.
