# #1150: resident retention (M2 follow-up)

Status: **the SDK fork pin landed; the retention changes are NOT implemented.**
Revision 4. Two independent pre-implementation reviews rejected the revision-2
retirement design (revision 1: gpt-6.1-sol, one BLOCKER + five IMPORTANT;
revision 2: deepseek-v4-pro, one BLOCKER + three IMPORTANT). Every finding was
re-verified against the source and is recorded below. Revision 4 adds the two
facts that finally decide the shape of the work: the per-actor retention is real
and structural, and preserving read intent while retiring an actor is not a
bounded warm-set change but a read-state ownership change. Parent issue:
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
2. **Correction (later finding): Room retirement *does* have a quiescence fence.**
   An earlier revision of this record claimed the opposite. `unsubscribe_timeline`
   calls `clear_thread_root_projections_for_room` for Room keys
   (`manager.rs:1735-1741`), and that function's first step is
   `timeline_actor_generations.invalidate_and_quiesce(key).await`
   (`timeline/thread_projection.rs:520-526`). The fence exists indirectly and must
   not be added a second time. What is *not* a fence is read ingress: the
   read-boundary check compares the handle's `Arc<TimelinePositionIndex>`
   (`timeline/read_state.rs:1694-1706`), not the generation gate.
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

## Revision 4: what the structural check settled

**The per-actor retention is real, and it is not the SDK's alone.** A retained
`TimelineActor` owns, per room, the full canonical item list and its derived
copies, none of which is truncated:

- `navigation_items: Vec<TimelineItem>` (`timeline/actor.rs:905`) is never
truncated, drained or retained — only indexed into (`actor.rs:1280`,
`item_projection.rs:645`, `navigation.rs:1757`).
- `navigation_items` "deliberately has a wider lifetime than the UI's replay
  window" (`thread_projection.rs:1157-1159`), while the bounded replay window is
  only `ROOM_REPLAY_INITIAL_ITEMS_MAX = 120` (`navigation.rs:53`).
- `media_gallery_items` and `media_sources` grow with the same window
  (`actor.rs:891,911`, `media.rs:586-596`), and the SDK `Arc<Timeline>` holds its
  own copy of the item vector (`actor.rs:849`).

So a room that was scrolled deep keeps that depth for the session, per visited
room: the canon violation in the section above has a real memory consequence,
not just a bookkeeping one. That is why the fix is worth doing at all.

**But it is not a warm-set change.** Read state is projected *into* the actor:
`project_local_read_correlation` looks the key up in `timelines` and, when the
handle is gone, removes the correlation outright
(`timeline/read_state.rs:1637-1640`), and the projection itself is a
`TimelineActorControl::ReadStateProjection` sent to that handle (`:1642-1646`).
The pending target, the position evidence needed to validate it
(`TimelinePositionIndex`, `read_state.rs:694-697`, one entry per event) and the
projected boundary all live on the actor side. Retiring the actor therefore
*drops* automatic read intent no matter how retirement is ordered; a guard only
chooses which intents are dropped, and a mailbox drain only narrows the window.
Preserving it means moving local-read ownership out of the actor — a read-state
machine change with its own canon amendment (`docs/architecture/state-machine.md`
read-receipt sections), its own tests, and its own review, which is a different
deliverable from a bounded warm set.

## Consequent decisions

- **Policy owner: Core.** overview item 7's two sentences conflict, and the
  normative prohibition on unbounded maps of owned live handles governs. When
  the work is done, Core owns the enforced backstop and the UI keeps the option
  to unsubscribe earlier. This still needs the canon amendment named below.
- **Read intent: preserved, so actor retirement waits.** Because preservation
  is an ownership move (above), it is the follow-up issue's subject rather than
  part of a bounded warm set.
- **Measurement: structural, not RSS.** The repository has no allocator or RSS
  instrumentation, and adding one would be new infrastructure. The check that
  decided this record is structural (what the actor owns and whether anything
  truncates it), which is deterministic and reviewable; a byte measurement
  remains unclaimed.

## Sequencing

The bounded warm set and the read-state ownership move are one deliverable, in
that order: the ownership move first (so retirement cannot drop intent), then the
warm set that uses it. Both need their own design record and independent review
before implementation, because the second is a read-state machine change. That
work is tracked in the follow-up issue
["Move local read intent off the timeline actor so cold rooms can be retired"](https://github.com/shinaoka/koushi-matrix/issues/1230),
which carries the same evidence and the acceptance criteria.

What must not happen in either: claiming a byte bound a `tokio::broadcast` and an
unbounded DTO list cannot enforce, inventing a threshold or latency gate the
issue does not define, or shipping retirement with intent loss and calling it
resource cleanup.

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
