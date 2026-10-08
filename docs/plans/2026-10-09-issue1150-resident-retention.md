# #1150: resident retention (M2 follow-up)

Status: **the SDK fork pin landed; the retention changes are NOT implemented.**
Revision 3 of this record. Two independent pre-implementation reviews rejected
the revision-2 retirement design (revision 1: gpt-6.1-sol, one BLOCKER + five
IMPORTANT; revision 2: deepseek-v4-pro, one BLOCKER + three IMPORTANT). Every
finding was re-verified against the source and is recorded below so a later
attempt does not start from scratch. Parent issue:
[#1150](https://github.com/shinaoka/koushi-matrix/issues/1150). Predecessors:
PR #1151 (compact `CoreEvent`), #1152 (drop the duplicated indexed room map),
#1157 (M2 index-first search, plan
[2026-10-06](2026-10-06-issue1150-search-index-first.md)).

## What this PR does

1. Lands fork PR #19's revision on the SDK fork's `main` (via fork PR #20) and
   moves the app's `vendor/matrix-rust-sdk` gitlink and `THIRD_PARTY_NOTICES.md`
   from the throwaway branch pin to that `main` commit. The tree is identical, so
   there is no content change.
2. Records the retention design, its rejected variants, and the open decisions,
   and corrects the now-stale plan/status docs.

Canon consulted: `REPOSITORY_RULES.md`, `docs/policies/engineering-rules.md`
("Design Simplicity", "Async and Runtime"), `docs/architecture/overview.md`
("Async Design Rules", timeline actor items), `docs/architecture/state-machine.md`
(Focused Context, Room-Subscription Residency), `docs/agents/state-ownership.md`
("Timeline items and the outbound send queue", "Room-subscription residency"),
`docs/agents/verification.md`.

## What is already done, and why the remaining scope narrowed

Verified at `20c2cf61`:

- Event-ring **slots** are bounded and enforced by a compile-time assertion over
  the documented formula (`crates/koushi-core/src/runtime.rs:130-141`). Retained
  payload bytes are deliberately outside it; the constant's own comment says so
  (`:125-129`).
- Search RAM bodies are bounded by M2
  (`crates/koushi-search/tests/search_memory.rs`).
- The React inactive timeline-key store already exists and is bounded
  (`TIMELINE_STORE_INACTIVE_RETAIN_LIMIT = 8`,
  `apps/desktop/src/domain/timelineStore.ts:91,436`). No React change is needed.

Both *named* memory sources in the issue are therefore fixed. The residual is
retention keyed by rooms visited in a session, which no check in this repository
measures.

## The residual, with evidence

`TimelineManagerActor::timelines: HashMap<TimelineKey, TimelineActorHandle>`
(`crates/koushi-core/src/timeline/manager.rs:493`) grows with rooms visited:

- Only Thread and Focused keys are ever unsubscribed; every
  `TimelineCommand::Unsubscribe` producer is a thread/focused/anchored path
  (`crates/koushi-core/src/runtime.rs:3096,3109,3146,3208,3266,3341`,
  `runtime/navigation.rs:766,893`), and `retire_undesired_focused_timelines`
  (`manager.rs:1764`) is Focused-only. A room switch cancels only pagination and
  link previews for the replaced room (`runtime.rs:1771-1786`). The frontend
  never issues an unsubscribe.
- `LiveTailRefreshCoordinator::states: HashMap<K, LiveTailFreshnessState>`
  (`crates/koushi-core/src/live_tail_freshness.rs:65`) is only ever inserted
  into (`:176,:265,:296,:397,:462`) and never pruned for a non-delayed room.

This contradicts normative text:

- engineering rules "Async and Runtime" 2: "No unbounded maps of owned tasks or
  live handles."
- overview item 7: "the runtime never leaks timeline state in an unbounded map",
  in the same paragraph as "Room switching policy (drop immediately vs.
  keep-warm) is decided by the UI through these commands" — the two sentences
  are in tension and the policy owner is undecided.
- `state-machine.md:3308-3318` says the same committed-demand cleanup leaves
  "room/thread actors … untouched", which any Core-owned eviction would falsify.

## Why the direct fix was rejected

The obvious change — retire the coldest Room actors from the warm set using the
existing unsubscribe bookkeeping — is **not safe as designed**. Verified hazards:

1. **Pending read intent can be lost, and the guard for it is racy.** A Room
   unsubscribe calls `remove_local_read_correlation`
   (`timeline/read_state.rs:702-717`), which retires desired read targets,
   cancels active writes, clears retries and republishes persistence without
   them; that is persisted product state which SDK history cannot reconstruct.
   Guarding on "no pending intent yet" is racy because the manager's loop is
   `biased` with the navigation-projection branch ahead of the message branch
   (`manager.rs:740,787,836`), so a victim's already-sent
   `LocalReadBoundaryObserved` may not be in `local_read_correlations` when the
   retirement runs; after removal,
   `handle_local_read_boundary_observed` returns early on the absent key
   (`timeline/read_state.rs:1694-1698`) and the observation is dropped.
2. **Room retirement has no quiescence fence.** `unsubscribe_timeline` runs
   `invalidate_and_quiesce` only for non-Room keys; the Room path does only
   `clear_thread_root_projections_for_room` (`manager.rs:1735-1741`). That was
   harmless while Room actors were only removed at teardown; it is not harmless
   for mid-session retirement.
3. **Teardown is abort-only.** `Drop for TimelineActorHandle` aborts without
   awaiting (`timeline/actor.rs:831-840`), which engineering rules 2 forbids
   ("Tokio `JoinHandle::drop` detaches the task and is never an orderly
   teardown"). `TimelineActorHandle::stop()` already exists and already awaits
   (`actor.rs:819-828`, used at `manager.rs:1106,1978,2010`), so the fix is to
   call it rather than rely on `Drop`.
4. **Scheduler state must be forgotten as a whole.** `running` is cleared in
   `finish` (`live_tail_freshness.rs:247`) and taken in `preempt_running`
   (`:391`), and `schedule_next` early-returns while it is set (`:412-414`).
   Deleting only a `states` entry can therefore strand every later live-tail
   refresh. A correct `forget` must drop `states`, remove the key from
   `delayed`/`delayed_members`, and — when it owns `running`/`cancelled_active` —
   clear `running` *without* re-queueing the victim (the existing
   `preempt_running` path re-queues, so it is the wrong model), then advance the
   next candidate.
5. **Settlement does not by itself release the decrypted window.** The diff
   relay holds a `TimelineWithDropHandle` and the handle holds an
   `enqueue_context` with a second `Arc<Timeline>`
   (`timeline/outbound_send.rs:151-154`); the relay is aborted fire-and-forget
   in `Drop for TimelineActor`. A retention assertion must therefore be
   structural (map size, scheduler state), not a claim about SDK memory release.

Together these mean the change needs a two-phase retirement (fence the victim's
generation, settle it, drain its pending observations, then remove) or an
equivalent ownership move of read intent to the manager — a concurrency change
that needs its own design and review, not a bolt-on to this record.

## Also not implemented, and why

- **SDK encrypted-search reader LRU / authenticated random-access index
  format.** Nothing in the repository attributes RSS to decrypted room readers.
- **A byte bound on retained ring payloads.** A `tokio::broadcast` retains
  cloneable values of arbitrary heap size, so only the slot count is enforceable;
  claiming a total byte bound would be arithmetic, not enforcement. The honest
  form is a measured workload result plus a documented residual, which is also
  what the issue's closing criterion asks for.
- **Sizing `EVENT_QUEUE_CAPACITY` against a measured burst.** Real work, but it
  needs instrumentation that does not exist: `--scenario=timeline_stress` gives
  100 rooms plus Spaces (`docs/agents/qa-lanes.md:62-75`) but builds rooms
  one-at-a-time, so it is not an initial-sync burst, and retained-ring-byte
  instrumentation must be added. Lowering the capacity without that measurement
  contradicts the constant's documented burst-absorption rationale
  (`runtime.rs:119`).
- **The SDK gap-inspection peak.**
  `load_all_chunks_metadata` (`crates/matrix-sdk-base/src/event_cache/store/traits.rs:75`)
  returns counts and links only, not the gap tokens and neighbouring event IDs
  `inspect_ordered_chunks` needs, and it is a transient deserialization peak
  rather than the steady-state quantity the issue bounds. Reopen with its own
  measurement.

## Open decisions for the maintainer

1. **Policy owner.** overview item 7 makes keep-warm a UI decision while also
   forbidding unbounded runtime retention, and the UI never unsubscribes. Should
   Core own an enforced bound (a canon amendment to overview item 7 and to
   `state-machine.md:3308-3318`), or should the frontend drive it through the
   existing `Unsubscribe` command so the documented owner stays the UI?
2. **Read intent.** Preserve a cold room's pending automatic-read intent (needs
   the two-phase retirement or a manager-owned read-intent move), or explicitly
   approve cancelling it (a behaviour change with its own test)?
3. **Measurement before implementation.** Build the burst/retention
   instrumentation first and implement only what it justifies, which is what the
   previous plan's own stop rule required.

## SDK fork pin

Fork PR #19's revision is what this branch's gitlink previously pinned
(`55e51ffe8`), reachable only from the throwaway branch
`integration/1157-burndown`. Fork PR #20 landed it on the fork's `main`
(PR #19's head plus its clippy follow-up plus the #1146/#1176 topics). Fork
`main`'s tree equalled the merge base, so the merge was additive:

| revision | tree |
| --- | --- |
| fork `main` before (`be17c637`) | `f64897ea` |
| merge base (`669834a55`) | `f64897ea` |
| previous client pin (`55e51ffe8`) | `f91ad0b6` |
| fork `main` after the merge (`f5d8028e0`) | `f91ad0b6` |

The app's gitlink and `THIRD_PARTY_NOTICES.md` now name `f5d8028e0`; fork PR #19
is merged. Local verification of the merged revision, in a `/tmp` target
directory: `cargo test -p matrix-sdk-search --lib` 41 passed;
`cargo test -p matrix-sdk --lib --features experimental-search
search_index::tests` 20 passed; `cargo test -p matrix-sdk-ui --lib` 391 passed;
`cargo clippy -p matrix-sdk-search -p matrix-sdk -p matrix-sdk-ui --features
experimental-search --all-targets` exit 0 with warnings only. Fork CI is not a
usable gate: `main` already failed the same check names (`cargo-deny`, `Lint`,
`msrv`, `All crates`, several benchmarks, coverage) before the merge, and the
merge introduced no new failing check name.
