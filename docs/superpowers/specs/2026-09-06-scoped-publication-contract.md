# Scoped publication and resource contract (#839 / #840 / #846)

Status: working design; not approved for implementation. This replaces no canon
until its review and normative amendments are recorded. See the
[current reader implementation worklog](../plans/2026-09-06-issue839-scoped-readers.md).
The reader vertical's linked constraints are approved; this document's remaining
application-wide proposals are not blanket implementation approval.

## Ordered-reader index investigation (not a selected implementation)

A separate primitive probe of imbl 6.1.0 Vector binary-search/remove/insert with
an immutable prior vector retained measured 32 / 1,600 / 8,448 key clones at
32 / 1,500 / 100,000 readers; comparisons were 12 / 24 / 36. A 20-row window was
read by index without materializing preceding rows. Source/log:
`/tmp/issue839-reader-order-probe.rs` and `.log`; the disposable binary was removed.
The library documents logarithmic insertion/removal, but the measured copy constants
are substantial. This is not an approved reader index, Core performance result or
proof of changed-reader bounds. Do not assume an ordered persistent vector has the
same 32-value update-copy behavior as the earlier OrdMap primitive. Final storage
choice must be measured with its actual owner/retention model. Follow-up with a
single mutable owner and no retained old vector measured 0 / 256 / 256 key clones
at the same sizes, with the same 12 / 24 / 36 comparisons. Source/log:
`/tmp/issue839-reader-order-owned-probe.rs` and `.log`; binary removed. This favors
keeping a derived ordered index out of whole-state snapshot cloning, but is still
only a primitive result, not Core/network/UI acceptance.

## Problem proved before design

The production receipt-observation regression
`compact_receipt_profile_lookup_is_bounded_for_1500_readers` retains the correct
1,500-reader total but fails its compact-profile budget: **requested 1,500,
expected at most 3**, in 0.12s. It uses the SDK's local mock-server session and
real Core emitter/reducer/diagnostic path. This is synthetic Core evidence,
not an actual avatar-network or full-scale homeserver measurement. No production
fix has been applied; limiting an input vector would lose required reader data.

A concrete, still-unapproved next reader vertical is recorded in
[receipt-reader-vertical](2026-09-06-receipt-reader-vertical.md), including SDK
endpoint batching, single-owner order, existing portable avatar references and
remaining lifecycle/type decisions.

## Reuse, not a new application framework

- Keep `AppState` and its established Rust semantic owners while changing
  publication and resource boundaries. Do not put a generic message bus,
  second canonical profile/member database or UI state machine alongside them.
- Keep `RuntimeConnectionId`, `RequestId`, account/session and TimelineKey
  identities. Scope identities are connection-owned handles, not another
  account identity. Existing typed commands and operation outcomes survive.
- Refine the existing reducer `AppEffect::EmitUiEvent` reporting with affected
  identities. Mutation owners report changes; do not add an action-name lookup
  table that guesses what a reducer changed. Audit Core-owned mutations too.
- Keep DesktopApi/platform ports and ordered read-only replicas. Change scoped
  payloads and recovery, not platform-independent command semantics.
- Keep the existing AccountActor network fetch/retry implementation. Move
  relevance, queue admission and cancellation decisions to the common Rust
  subscription/demand contract, then remove superseded URI request paths.

## Proposed connection and projection lifecycle

A subscription identifies its connection, account/session epoch, typed scope,
scope generation and current model revision. A window request identifies its
anchor/visible records against a revision, not an unversioned array offset.
Rust owns ordered membership, limits and bounded prefetch; the renderer supplies
actual visible identities and native lifecycle observations.

1. Attach establishes a coherent initial projection and revision before ordered
   changes for that scope can be delivered. Subscription installation and the
   initial read must not race publication.
2. A committed batch identifies its scope and base/next revision. Coupled facts
   share one commit: reader count with compact readers, navigation target with
   generation, and permissions with actions. A global commit/admission sequence
   may remain; unrelated scope commits need not be consecutive for a consumer.
3. Model-applied acknowledgement is not layout visibility. #846's observation
   identifies the actually committed display revision. A newer model revision
   invalidates prior positive live-end evidence immediately; withholding a new
   observation alone is insufficient. Core validates these facts.
4. Slow-consumer recovery is scope-local. Ordered changes cannot be silently
   dropped; recover the affected projection from coherent state on a gap.
   Bound the entire adapter path, including a Tauri forwarder that otherwise
   drains a bounded Rust channel into an unbounded WebView event queue.
5. Detach/suspend/account replacement retires that consumer's demand and
   callback scope. Another consumer of the same resource remains valid. Drop
   and failed transport cleanup must not rely on a potentially full data queue.
6. Explicit inspection/recovery is different from routine publication.
   `CoreConnection` request-outcome waiters currently inspect watch snapshots
   after lag/closure; replace their required coherent facts before removing the
   watch. Keep exact request matching, absolute deadlines and shutdown results.

No transport type contains DOM pixels, Tauri URLs, window handles or GPUI types.
Media records carry portable identity/readiness; hosts resolve native resources.

### Verified internal-read decision inputs

`koushi-state/src/state/mod.rs:258–340` derives Clone for AppState and embeds
owned profile state, room/Space/invite vectors, maps and stores.
`state/profile.rs:83–100` embeds ordinary BTreeMaps for users, room-local users
and aliases, not persistent/shared maps. `koushi-protocol/src/state_update.rs:19–24`
embeds an owned AppState in VersionedAppStateSnapshot. Therefore the current
snapshot clone is a deep data copy, not an existing cheap shared-root handle.
Merely wrapping that type in Arc cannot satisfy the migration. A persistent-root
alternative would first require per-domain collection/value conversion, bounded
root retention and mutation-indexed updates; it is not an already-present shortcut.
These facts are available for the next bounded read-boundary review, which should
not repeat whole-runtime exploration to rediscover the types.

### Read-boundary review verdict

Sol high, read-only, bounded review of lines 65–136: **APPROVE** the actor-borrowed
outcome query direction, private-write wakes and final-state move before closure.
No blocking findings. This supersedes the prior timed-out attempt only for this
bounded decision; it does not approve scoped payloads, capacity numbers or the
whole application migration. Implementation must never await an actor query from
that actor itself, must preserve the absolute deadline and terminal-event matching,
and must publish final state before exposing closure. Existing plain collections
remain canonical rather than introducing a second persistent global root.

### Implementation-boundary finding: timeout recovery

Parent follow-up inspection found a constraint not supplied to that bounded review:
`runtime/request_outcome.rs:530–569,2393–2409` checks the latest snapshot before
checking the deadline and checks it again in `final_result` after timeout. Thus an
already satisfied predicate can succeed with an already expired deadline. An
actor query cannot necessarily obtain that same current-state predicate under the
original expired deadline, even if the actor's state already satisfies it. Merely
caching a previous query result can lose such a success. Do not implement or amend
canon for the query migration until this boundary is resolved without silently
weakening existing behavior, extending the deadline or adding a pending registry.
The direction approval above is not implementation clearance for this newly found
case. The existing `select_room_and_wait_accepts_the_already_active_snapshot_without_new_generation`
test now additionally proves initial success with a deadline one second in the
past and exact state/generation equality: PASS, 0.08 s, in
`/tmp/issue840-outcome-expired-deadline.log`.

Sol high material-finding follow-up: pure actor-query replacement has an Important
finding. Conditional corrected direction is one latest-only synchronously readable
immutable operation-facts publication, coherent at the same state commit, with
existing wake-before-read and final predicate precedence. Reuse scoped product
models, no pending-result registry/history or second canonical owner. Actor queries
are reserved for reads without this synchronous-current contract. Exact predicate
fields must be derived before implementation; no payload implementation approval.
The existing outcome variants still return whole AppState, which narrow facts
cannot reproduce. This must be an explicit public payload migration, not a silent
substitution. Preserve whole-state inspection assertions at the explicit inspection
boundary while adding exact narrow-result assertions; do not weaken behavioral
conditions or claim the old full-state return type remains compatible.

### Concrete API direction

Extend `CoreConnection`, rather than introduce another application facade:

- `subscribe(typed_scope, window)` returns an owned subscription handle and a
  coherent initial revision; the handle receives typed scoped changes and
  accepts revision-scoped window updates. A concrete scope enum replaces
  string paths or arbitrary JSON queries. Use the existing connection identity
  and a connection-owned subscription sequence for admission.
- Recovery reinitializes only that handle's scope. Subscribe before reading its
  initial state so no update can fall into the attachment gap. Keep terminal
  operation signals separate from replaceable view projections.
- Explicit close is acknowledged after demand retirement; native handle drop
  and host teardown also retire the lease. Use a lifecycle wake/closed-sender
  path independent of the potentially full data queue, following the existing
  Core lease-notification pattern. Do not add restoration/retry timers. Actor
  resource dispatch must check live consumer demand before starting work.
- Keep `wait_for_request_outcome` and its closed `RequestOutcomeExpectation`
  variants, connection-owned context, cancellation and typed terminal-event matching.
  Evaluate predicates synchronously against one coherent latest-only immutable
  operation-facts read model, derived by the authoritative mutation/publication
  owner. This is a read-only projection, not another canonical database. Define
  its exact fields from all predicates before implementation. Return matched
  operation facts and committed generation, not embedded full AppState; explicitly
  migrate public result types and their callers/tests together.
- Arm the revision wake before loading current facts. Evaluate before checking
  the absolute deadline and again on timeout/lag/closure, preserving current
  precedence and exact request/account/target/submission and allow-initial rules.
  Private draft/scheduled-send refreshes must update coherent facts and wake even
  without a public-generation change. A wake is never outcome proof. No actor-side
  pending-outcome registry or result-history cache is introduced. Actor queries
  may serve other explicit reads, but cannot replace these synchronous predicates.
- Shutdown settlement: publish final coherent operation facts after existing
  child teardown/final updates and before signaling closure. Retain that latest
  read root for existing and post-closure waiters; evaluate the same predicates
  before reporting closed/unmatched. Closure never proves success. This no longer
  requires moving full AppState for settlement. Any retained full-state diagnostic
  inspection has a separate explicit lifetime, not an operation-result cache.
  Account sign-out still clears account state through its existing owner. Verify
  final-facts-before-closure ordering before removing `snapshot_rx`.
- Full-state inspection may remain explicit for diagnostics/test inspection;
  it is not the ordinary UI subscription, command settlement or lag-recovery
  path. Keep independently requested inspectors out of routine update cost.

These signatures describe the intended Rust boundary, not shipped symbols.
Queue/byte limits and concrete per-scope payloads still need to be settled in
review before implementing this API. The corrected synchronous read model supersedes the earlier
actor-query/final-state-move proposal for settlement. Its concrete predicate and
return-payload inventory is in
[operation-read-facts](2026-09-06-operation-read-facts.md); it remains unimplemented.

### Typed scope and observation shapes for review

The public scope is a closed enum, not field paths or arbitrary JSON selectors.
Reuse existing account, room, TimelineKey, ComposerTarget and RequestId types.
Proposed variants and payload boundaries:

| Scope | Contents / selection |
| --- | --- |
| AccountSummary / Authentication / AccountSwitcher | Current session/header and auth operation facts; saved-account rows are windowed separately. |
| Settings / OwnProfile / Security | Existing bounded policy/profile scalars; security device collections are windows, not an embedded account database. |
| Navigation | Existing semantic destination, operation and generation facts; no pixels or desktop-pane identity. |
| Rooms | Rust filter/Space selection, windowed room/Space/invite rows, counters and edges. |
| Timeline(TimelineKey) | Versioned window rows and structural ordering/count information from the existing timeline owner; not account profile/live-signal maps. |
| RoomDetails(room_id) / Members(room-or-Space target) | Room capabilities/settings and explicit member windows, preserving Rust filtering, membership and labels. |
| ReceiptReaders(TimelineKey, event_id) | Explicit full-reader window/count. Compact readers belong to their timeline rows and do not imply opening this scope. |
| Person(room context, user_id) | A specifically opened person's profile; not an unrestricted list of MXC fetch targets. |
| Activity / Directory / Search / Files(room_id) / Threads(room_id) | Existing query owner/status and generation plus a bounded result window; subscribe does not invent a second query/navigation state machine. |
| Composer(ComposerTarget) / Uploads(ComposerTarget) / Gallery(room_id) | Accepted document/operation facts or bounded attachment/gallery records; no unrelated draft/store contents. |
| Attention / Errors | Bounded current Rust attention/error facts; platform side effects stay on existing ports. |

Subscription identity combines a runtime-incarnation nonce with
`(RuntimeConnectionId, local sequence)` so recreated runtimes cannot accept old
serialized scope messages with reused counters. This is a runtime/transport fence,
not a new account identity. Scope generation fences descriptor/session replacement
and suspend/recreation. Model revisions
identify delivered windows. Each lifecycle/window request is admitted at the Rust
owner. Native observations report that identity/generation, the model revision
actually installed, and visible row identities (or a revision-qualified requested
range when rows are not loaded). Rust validates membership, derives prefetch and
resolves resource identities. Caller-supplied MXC lists are not this API.

Keep model application and layout observation separate. Window scrolling does
not create a new account identity. A model ACK releases transport capacity but
does not mark a timeline read or declare it at the live edge. New relevant model
data invalidates previous positive live-edge evidence; a late observation cannot
restore that evidence across a newer model or replaced scope. Scope retirement
uses an independent lease/drop notification, not an enqueued data message.

These shapes reuse current semantic owners; they do not introduce independent
multi-window search/navigation semantics. Multiple consumers can have different
projection windows and demand while the existing Rust product-state owner remains
singular. Public payload structs must exclude whole-map fields from the inventory
below; preserving an old full slice under a scope name is not migration.

## Bounded delivery direction for review

Reuse the existing Tokio watch/channel machinery rather than create a durable
message log. Each subscribed scope retains one latest delivery containing its
base/next revision, a narrow delta, and a coherent bounded window snapshot.
The snapshot is shared Rust data, not serialized on each normal update. A
consumer applies the delta only when its installed revision matches the base;
otherwise it installs that same delivery's scoped snapshot. Coalescing therefore
cannot silently lose inserts/removals or require unrelated scopes to reload.
Terminal operation events keep their existing non-replaceable matching path.

Snapshot construction must update only affected indexed rows. A watch containing
a newly deep-cloned account map is explicitly not this design. Initial/recovery
window work is separately counted. Producer, mailbox and in-flight adapter data
all need byte accounting; a bounded channel before an unbounded WebView queue
is not sufficient. The Tauri adapter allows one unacknowledged projection
message per consumer and retains latest unsent scope state, not a growing JSON
queue. Model application acknowledges transport progress only, never visibility.
Native adapters apply the same typed records without requiring JSON.

Proposed admission/scheduling limits for review (not shipped):

| Boundary | Limit and handling |
| --- | --- |
| Active projection scopes | 64 per runtime, including internal consumers; explicit capacity result on excess, never silent replacement of another consumer. |
| Window records | At most 256 delivered rows; larger lists expose total/count, stable identities, revision-qualified range requests and edge/cursor information, not hidden full arrays. |
| Projection mailbox | One latest coherent delivery per scope; no historical delta queue. |
| Adapter transport | One unacknowledged projection message per consumer; encode only the next admitted message, not every overwritten mailbox value. |
| Projection payload accounting | 64 MiB per scoped initial/recovery snapshot and 256 MiB aggregate retained encoded-data budget, including retained/in-flight versions. Ordinary deltas remain independently bounded by affected rows. These are encoded-data limits, not a false exact Rust-heap bound. |
| Avatar prefetch | At most eight additional targets total per active surface; visible membership comes from validated model-relative observations, not submitted MXC lists. |
| Avatar scheduler | Six in flight (existing limit), at most 256 queued distinct resources; do not spawn a waiting task per URI. Visible demand precedes prefetch. Excess logical demand remains bounded by admitted scopes/windows and is selected as slots free. |
| Avatar file cache | 128 MiB; evict unreferenced recomputable entries and retire their host resources. SDK canonical data and durable drafts/aliases are not eviction targets. Native decoded-image storage is a host capability, separately measured. |

Oversized projection admission is explicit and localized. Drop prefetch before
refusing a new window; do not truncate visible semantic records, fabricate totals,
or delete durable data to fit a budget. An unavailable/capacity response needs
localized recoverable UI feedback and is included in adapter tests. Suspended
scopes retire foreground demand; memory pressure removes prefetch and evicts
unreferenced recomputable resources before affecting active presentation.

The earlier profile publication budget RED remains historical: before the
scoped path, a single changed profile with 1,500 retained source profiles emitted
351,847 bytes instead of the 4 KiB target (`/tmp/issue840-publication-red.log`).
The current executable receipt-publication measurement seeds 100 rooms and
10,990 events, including 1,500 active-room readers, then moves one receipt between
two existing events. It emits a 1,773-byte nested delta touching exactly two
events (`/tmp/umbrella-publication-perf-measurement.log`). This is still a debug
line-table wall-time characterization (16.85 ms build slice), not an actor CPU,
retained-heap, network, browser-task, p95, or whole-migration acceptance result;
those measurements remain required.

### Work scheduling and recovery episodes

The merged 32-command turn cap is only a count bound, not proof that handlers or
projection work meet the CPU target. Keep semantic action batches atomic; do not
publish half a role/permission or count/reader transition to get a lower timing
number. Expensive initialization/query projection belongs at its existing Rust
owner with bounded preparation and a coherent commit, not a new background
application-state framework. Routine publication visits affected subscribed rows.

A history-repair episode has an aggregate 32-batch budget, retained across its
own successful topology mutations. Exhaustion publishes an explicit incomplete/
paused outcome while preserving SDK tokens and continuity proofs. A genuinely
new user recovery intent may start another episode; repeated layout observations,
new model revisions and self-induced topology changes may not silently renew it.
Keep input evidence separate from model/layout churn when defining that episode.
Search/background preparation yields between bounded work batches and cannot
occupy the foreground queue behind an unbounded backlog. Do not add timers that
restore retired demand or declare an incomplete gap complete.

## Reference workload and performance targets for review

Before publication migration, build one deterministic workload and a smaller
control. Reference: 100 rooms, 1,500 members/readers in the active room, 10,000
historical events in that room plus ten in each remaining room (10,990 total).
Control: ten rooms, 50 members/readers, 500 active-room historical events. Keep
visible row counts, changed identities, common top readers and display strings
fixed for the measured operations. Setup/cold initialization is separate from
ordinary-update measurement; compare after backfill/crawl become idle.

Measure one profile change, one receipt move between two existing events,
foreground input during queued background work, scroll/window changes and
close/reopen. Initial member/reader records may be synthetic Core/SDK fixtures,
but the image-request phase must count real media HTTP requests against each
disposable Tuwunel/Synapse lane, including unexpected/failed requests. Give
unrequested readers distinct media identities so shared-image deduplication
cannot disguise eager acquisition. Also assert requested visible images actually
arrive; zero requests is not success. Synthetic ingress is not evidence that
1,500 real homeserver accounts or a full-scale room were seeded.

Proposed reference hardware/profile: pinned Linux Ubuntu runner/container, two
CPU cores, 4 GiB memory, Rust unoptimized line-table profile and production frontend
bundle. Targets: p95 input-to-visible <=100 ms, p99 <=200 ms, individual measured
foreground Core CPU work slices <=8 ms, and no >50 ms browser main-thread task
caused by an ordinary measured update. Report wall-clock waits separately from
CPU work. Track touched records and serialized bytes, not latency alone. The
small-to-reference increase in retained non-SDK projection/resource metadata is
limited to 32 MiB with the same visible scopes; SDK canonical/store memory and
host image memory are separately reported, not hidden in that claim.

Fixture setup ceiling: ten minutes and 2 GiB disposable disk, reported separately;
measurement runs finish within 120 seconds. This is not the #846 full-scale
characterization or an excuse to lengthen its added-CI-path target/ceiling.
That characterization needs its own actual achieved scale, seed limits and
cleanup evidence. These targets require design approval and real results; none
is currently recorded as passing. Do not relax them after a failure without
explicitly revisiting the design/requirements.

## AppState field-to-scope inventory

This inventory covers every current field in `state/mod.rs::AppState`, not
just the first slice. Scope families are proposed payload boundaries, not new
crates or permission to put entire existing maps inside renamed DTOs. The
separate runtime/SDK resource inventory still needs reconciliation.

| Existing fields | Proposed publication owner/scope |
| --- | --- |
| `session`, `session_lock_reason`, `secure_backup_gate`, `auth`, `soft_logout_reauth`, `qr_login`, `current_session_status` | Session/header and explicit authentication operation facts; no room/profile collections. |
| `sliding_sync_account_epoch`, `sliding_sync_capability`, `sync`, `sync_generation` | Rust lifecycle/admission authority; publish coarse session sync status and existing epoch fences, not a renderer sync owner. |
| `account_management_url`, `account_management`, `account_management_capabilities`, `device_cleanup`, `e2ee_trust`, `local_encryption` | Account/security operation views; device lists are explicit bounded windows, not header payloads. |
| `settings`, `cjk_text_policy` | Account display/policy view; native IME/editor state remains outside this data. |
| `navigation`, `timeline`, `thread`, `focused_context`, `basic_operation` | Semantic navigation/operation views and existing timeline identities; layout observations stay separate. |
| `rooms`, `spaces`, `invites`, `room_list` | Rust-owned room/sidebar windows and counters; updates identify affected rows, not replacement account vectors. |
| `room_preferences`, `room_notification_settings`, `link_preview_settings`, `room_interactions`, `room_management`, `invite_workflow` | Relevant room/Space or explicit operation view; no unrelated rooms' overrides or membership records. |
| `profile` | Own profile plus demanded user/room-profile windows and targeted alias/ignore operation facts; keep durable preferences distinct from evictable display records. |
| `live_signals` | Visible room typing/presence, compact receipt projections and explicit full-reader windows. No account-wide room/readers map. |
| `space_members`, `mention_candidates` | Explicit member/query windows with Rust filtering/order/permissions and generation admission. |
| `activity`, `directory`, `search`, `search_crawler`, `files_view`, `threads_list`, `thread_attention` | Explicit query/list/room scopes plus bounded background-operation summaries. Preserve Rust target, unread and operation semantics. |
| `composer_drafts`, `scheduled_sends`, `upload_staging`, `media_gallery` | Existing Rust stores/leases and explicit target/operation/gallery projections; do not broadcast durable store contents. |
| `thread_root_projections` | Existing Core/state projection lifecycle retained; publish only scoped ordered display rows. |
| `native_attention`, `native_attention_context`, `errors` | Existing Rust attention/error authority; publish current bounded attention/error facts, with platform effects through existing ports. |

An account-wide locale or lifecycle reset can invalidate relevant subscribed
views explicitly. It does not justify scanning all retained data for a routine
one-profile/one-receipt change. Diagnostic inspection is an explicit separate
operation, not an automatic source for each subscriber's update.

## Consumer removal map

Read-only Flash inventory plus parent source checks identify these migrations.
Paths below are current source anchors, not claims that the migrations are done.
Unqualified Rust paths are relative to `crates/koushi-core/src`.

| Current consumer / path | Replacement or retention contract |
| --- | --- |
| `runtime.rs` AppActor before-state copies and `publish_state_delta` / `publish_state_change` | Mutation-owned affected identities and scoped immutable delivery; delete whole-AppState comparison/publication copies, not merely the unused clone probe already removed by #848. |
| `runtime.rs::publish_snapshot_refresh_without_delta` | Targeted composer/scheduled-send publication and query wake. Preserve private-change visibility without reinstating a full-account watch. |
| `runtime.rs` media snapshot task around 644; `media_preparation.rs::reconcile_snapshot` around 572 | Session-account observation and active/retired staging identities only. This helper currently reads session and staging membership, not profiles/rooms; remove its full-state clone/watch dependency. |
| `runtime/connection.rs::project_event_for_consumer` around 797; `event_projection.rs` | Resolve affected labels/policy through shared Rust projection owners. Current helper needs session user identity, relevant profile/alias facts and `hide_redacted`, not a per-event account clone. |
| `runtime/request_outcome.rs::wait_for_request_outcome` around 519 | Keep request/event matching and absolute deadlines; narrow state queries plus final moved-state handoff described above. Remove snapshot-bearing outcome variants and migrate their consumers. |
| Tauri command modules and `media_staging.rs` generation-only baselines | Read scalar generation/context, never call `versioned_snapshot()` just to discard its state. |
| `apps/desktop/src-tauri/src/commands/session.rs` get/settlement/resync snapshots (25–75) | Scoped initial attachment, narrow command outcomes and scope-local recovery. Remove all three routine full-snapshot endpoints and bindings. |
| `apps/desktop/src-tauri/src/core_event_forwarder.rs` lag resync (47–76,158+) | Per-scope mailbox recovery and bounded acknowledged transport; no full-account replay on a lost scope batch. |
| `apps/desktop/src/backend/{desktopApi,client}.ts` | Expose the same typed subscriptions/outcomes; replace get/settlement/resync snapshot implementations and contract fixtures together. |
| `App.tsx` initial setup (1634), resync (1620), command watermark (822), login settlement (2272); `domain/{stateUpdateConsumer,commandWatermark}.ts` | Ordered scoped replicas and per-scope revisions; no global snapshot overwrite or account-wide minimum-generation repair. Keep listener-before-initial-read and command-order guarantees. |
| `koushi-qa` real-homeserver waiters, headless event_wait, participants and orchestrator | Use session/security/navigation/operation scopes and typed outcomes. Existing generic `predicate(&conn.snapshot())` loops (orchestrator 61/69) are routine consumers, not exempt diagnostics. |
| Explicit inspection assertions / diagnostics | May request an on-demand coherent inspection, with separate cost accounting. They may not keep an eagerly copied production watch alive or masquerade polling loops as free inspection. |

### Runtime/resource owners to retain or migrate

| Existing owner | Boundary disposition |
| --- | --- |
| TimelineManager `session_subscribed_rooms` vs `subscribed_room_leases` (`timeline/manager.rs:379–399`) | Preserve #518's distinction: retiring a presentation lease does not erase session residency or SDK security-required room subscriptions. Scope lifetime governs presentation actor resources, not canonical sync membership. |
| `TimelineManagerActor.timelines`, generation gates, navigation projection and live-tail coordinators | Reuse these owners for Room/Thread/Focused subscriptions and stale-result fencing; do not add a competing timeline database or navigation coordinator. |
| Existing send-completion coordinator, admission ledger, enqueue/read supervisors and terminal ingress | Retain committed-send/terminal-delivery lifetimes and their existing bounds. Closing a view does not cancel or duplicate an SDK-committed send. Audit actual limits, not just queue type names. |
| `AccountWorkScheduler` (`account_work.rs:24–162`) | Reuse its interactive/foreground/background priorities, one-history-request ceiling and 64-event history batches. The aggregate repair-episode budget is additional to per-permit batching. |
| AccountActor avatar cache/inflight/semaphore/JoinSet/session generation (`account/actor.rs:956–977`) | Keep the network/cache owner, replace per-URI waiting tasks/request-id waiters with live scope references and bounded admission, and retain session/result fencing. |
| AccountActor auth/verification observers and timeout tasks | Preserve their security/operation lifetimes; they are not presentation resources to cancel when a panel scrolls away. Publish only their bounded operation views. |
| Thread-root hydration/fetch registry and preview policy under TimelineManager | Retain generation, policy and shared-fetch authority; align recomputable presentation demand with active scopes without restarting settled/failed work on actor replacement. |
| StoreActor, native artifact, media staging/preparation ports | Keep durable persistence and platform effects at these established boundaries. Replace full-state observation with required identities/operation facts; native paths/decoded objects do not enter portable projection records. |

The broader feature ownership inventory remains
`docs/architecture/frontend-ownership-inventory.md` (#552); reconcile its avatar
request-ref retention decision on cutover. State's 53-field mapping above covers
the data side. Runtime task/cache ownership and all actual endpoint deletion
checks remain part of the final integration audit, not satisfied by this table.

## First data migration: profiles and receipts

- Retain SDK receipt placement authority. At the pinned SDK revision,
  `controller/read_receipts.rs::maybe_update_read_receipt` resolves old/new
  visible timeline positions and rejects backward movement;
  `compute_event_receipts` includes receipts from subsequent filtered events.
  Therefore raw protocol receipt event IDs are not interchangeable with SDK
  display-item receipt placement. Do not build an independent placement map
  from raw receipt events.
- The inspected public `EventTimelineItem::read_receipts` returns the entire
  `IndexMap`; public Timeline receipt subscriptions inspected so far concern
  the own user's receipt. They do not establish a sparse changed-reader API.
  The mutation boundary is `ReadReceiptTimelineUpdate::remove_old_receipt` /
  `add_new_receipt`: these know the changed user and resolved display item and
  replace that item in the existing observable vector. `compute_event_receipts`
  also moves hidden-event readers when insertion changes their display owner.
  A separate subscription to raw receipt events cannot correlate these changes
  atomically with timeline placement and is rejected.
- SDK extension decision for review: expose structurally shared receipt snapshots
  through the existing immutable item / VectorDiff, not a second receipt bus.
  Use the already installed `imbl::OrdMap::diff`, which skips shared nodes, rather
  than inventing change journals, receipt revision counters or predecessor chains.
  The Core adapter retains the last source snapshot for its active projection
  and derives changed reader identities from map differences. Coalesced Sets can
  compare their endpoints directly. Initial/reconstructed maps and stream reset
  remain explicitly accounted full initialization; this is not a blanket O(1)
  claim for comparisons of unrelated maps.
- Preserve the existing public full-read accessor and IndexMap insertion /
  swap-remove ordering. Proposed SDK-internal representation: persistent user-key
  map containing receipt plus order slot, and a persistent ordered user-ID vector.
  These are one collection plus its lookup/order index, not another placement
  authority. Swap-remove changes at most the removed and last user's index entries;
  the public change iterator filters slot-only changes. Keep a lazy full IndexMap
  only for explicit legacy access, never copy that cache during snapshot mutation.
  Ordinary Koushi consumption and SDK mutation must avoid that accessor.
- The SDK currently deep-clones the item's `IndexMap` before the two mutation
  methods above. Merely adding a changed-user field would leave proportional
  copying in the source path. The reviewed SDK design must remove that copy too,
  using existing `imbl` structural sharing while retaining insertion/swap-remove
  iteration behavior. Do not retain an unbounded chain of previous versions.
  Public full-map access may materialize a legacy representation on explicit
  demand; neither SDK mutation nor Koushi routine consumption may use that path.
  Test placement, hidden-event redistribution, ordering, coalescing/gap recovery
  and retained-version memory at the SDK owner before integrating the gitlink.
  This is a proposed input-boundary change, not an approved implementation or
  a claim that the 1,500-reader regression has turned green.
- Primitive feasibility evidence (not SDK, network or application acceptance):
  installed `imbl 6.1.0` `ord/map.rs:288–312` documents and implements shared-node
  skipping. `/tmp/issue840-receipt-map-probe.rs` counts value cloning/comparison
  for a single changed entry: 32 readers → 32 clones / 32 compares; 1,500 and
  100,000 readers → 32 clones / 33 compares each; exactly one diff each. This
  supersedes the more complex candidate based on per-item change provenance.
  Worst-case unrelated-map diff is still O(n), as the library documents.
- Review attempt: Sol high read-only reached its 300-second deadline without a
  necessity/representation verdict. No approval is inferred from that attempt.
  A subsequent bounded review covered only the concrete representation.
- Representation review: Sol, high, read-only — **APPROVE**, no material
  representation findings. Keep map/vector invariants private; clone with an
  empty compatibility cache and invalidate it on successful mutation. Diffs are
  net per-user changes, not historical IndexMap-order replay. Test deletion of a
  non-final user and filtering of the moved-last-user slot-only update.
  This approves the SDK collection representation only, not the common scoped
  API, Core caller migration, reconstruction costs or lifecycle integration.
- Representation implementation checkpoint (uncommitted SDK work; gitlink still
  `600044a4`): private `ReadReceiptSnapshot` replaces the remote item's IndexMap.
  Existing public full-map return type/order is unchanged. The existing SDK live
  receipt test gained an assertion that cloning an event shares receipt records
  after full-map access: RED before implementation, GREEN afterward. A 1,500-row
  owner test compares insert/update/swap-remove/reinsert and cached results with
  IndexMap, checks slot invariants, and checks immutable originals/cache-free
  cloning. Receipt tests **15 passed**; full SDK UI library **371 passed**, 2.90s.
  Logs: `/tmp/issue839-sdk-sharing-{red,green}.log`,
  `/tmp/issue839-sdk-collection.log`, `/tmp/issue839-sdk-ui.log`.
  SDK standalone dependency/test compilation is separate setup evidence, not
  part of those run times; the root workspace cannot directly test this dependency.
- The read-only `read_receipt_snapshot()` API and direct iterator now feed the
  sole Core receipt collector, avoiding its compatibility-map materialization.
  Existing SDK redistribution also uses a cheap retained snapshot. SDK UI tests
  remain 371 passed and both new public-item doctests pass. A sparse changed-reader
  iterator and Core ordering/index update path are still not implemented; Core
  still materializes all reader DTOs and the 1,500-reader profile lookup remains RED. No SDK commit/push, gitlink update, final
  integration review or merge is claimed. Ruma `Receipt` lacks `PartialEq`:
  the planned map-diff adapter must compare the actual receipt fields (`ts`,
  `thread`) explicitly rather than assume tuple equality compiles. The owner
  tests compare serialized fields and iteration order without weakening checks.
- Preserve full raw receipt membership/timestamps for explicit reader queries.
  Keep derived ordering/dependency indices only for retained application scopes;
  define insertion/removal/eviction together. Do not maintain another canonical
  Matrix receipt or timeline database.
- A compact summary contains accurate total, zero-to-four displayed readers
  (all through four; three plus remaining count from five onward), and only
  those readers' display/profile records. Preserve own-user exclusion and
  timestamp-descending/user-ID tie ordering. Full reader data is a separate typed,
  virtualized query/window, not a hidden array inside the compact DTO.
- Resolve/enrich only demanded display identities; a profile change updates
  affected retained records rather than every room/event/reader. Global locale
  or display-policy changes must be explicit invalidations, not the default path
  for one user update.
- The first production cutover removes account-wide profile/live-signal payload
  consumers on the migrated paths, including eagerly formatted hidden reader
  details. It must also eliminate their routine full-AppState-copy dependencies;
  a sparse wire event alongside unchanged eager watch copying is insufficient.
- Preserve hover/focus access, timestamps, localized text and keyboard operation
  when replacing the full-reader popup. Reuse existing list mechanics where
  appropriate; no new all-items-rendered fallback for large readers.

## Avatar demand ownership

`AccountWorkScheduler` already supplies account-wide priority policy; do not
invent a second general-purpose scheduler. If shared media admission is added
there, give it its own six-slot resource pool rather than accidentally applying
the existing one-slot history ceiling or increasing history concurrency. The
AccountActor selects eligible demanded resources before spawning work, with at
most one pending admission decision, not one waiting future/task per URI.

Existing semaphore admission limits active SDK calls, not waiting tasks.
Subscription-owned relevance must instead determine a bounded pending set and
start work only when a slot exists. Deduplicate by the existing account-scoped
media identity across surfaces; visible demand precedes bounded prefetch.

Rust resolves timeline sender/compact-reader, member-window and room/Space/invite
window identities into media demand. The renderer does not submit an unrestricted
MXC list. Closing a surface removes only its references; queued unused work is
removed and unnecessary in-flight work is cancelled with late results fenced.
Reuse cached Ready/terminal results and the established retry semantics. Network
fetches remain asynchronous children of the existing actor, with awaited teardown.

Use the account/session-qualified Matrix media identity and a typed media variant
as the portable reference; do not invent a second account identity or put a Tauri
URL/native path in the record. AccountActor/native-artifact ports retain actual
files/handles. Adapters resolve references outside product-state deltas and fence
late resolution against scope/resource identity.

Readiness belongs to the shared media owner and its demanded scope records, not
copied thumbnail state in every cached profile/room/receipt. On cutover, remove
that redundant status propagation and the matching-URI scans (including the
thumbnail-preservation helpers made obsolete by the new reference). Preserve
cached-image reuse via the media owner; evicting an unreferenced file must not
leave a fake Ready value in a future subscription. Native decoding/geometry facts
remain host/renderer observations, not another acquisition/retry policy.

## Canon adoption and host contract

Before implementing the common API, adopt this requirement in
`REPOSITORY_RULES.md` as the single durable owner:

> Product behavior and authoritative application state belong to the
> toolkit-independent Rust core shared by React/Tauri, a possible GPUI Kit
> desktop renderer, and future iOS/Android clients. Renderers and hosts adapt
> that core; they are not alternative owners of product semantics. Every
> behavior change identifies its shared owner and portable contract. A touched
> ownership violation must converge in a bounded vertical change that removes
> its superseded owner. Geometry, native composition/unacknowledged input,
> transient interaction and ordered read-only replicas stay renderer/adapter
> owned. Language alone does not make a GPUI entity shared-core state.

The overview owns the concrete dependency and scoped-publication model; its
snapshot-centric descriptions must be corrected, not contradicted by an appendix.
State-machine canon gets the attach/initial/delta-or-recovery/observe/retire flow,
separate model and layout evidence, resource cancellation and paused repair outcome.
The i18n canon keeps Rust locale/display-policy ownership and adds scoped
invalidation; this does not move native geometry or all formatting caches into
product state. The #552 inventory is reconciled in place. Engineering/review
policy and PR guidance require owner, portable contract, removed legacy owner
and shared-core verification; presentation-only changes may state that briefly.
AGENTS links the rule rather than duplicating it.

Extend the existing dependency guard to cover Tauri/GPUI/toolkit dependencies
in every shared application-contract/SDK-adapter crate, including renamed and
target-specific dependency declarations. Keep portable compilation and behavioral
contract tests. Static dependency checks do not prove that an arbitrary type or
field has the right semantic owner; review remains required.

Hosts report foreground/background, execution permission/suspension, connectivity,
memory pressure and optional power/metered facts through typed capability inputs.
Reuse existing attention, executor, StoreActor and artifact/media ports. Rust
applies resource policy; the OS grants background execution. Suspended consumers
do not leave layout ACK waits alive. Resume installs fresh scope generations and
coherent windows. A recreated runtime has a fresh incarnation fence. Checkpoint
acknowledgement follows actual store completion; do not promise preservation of
unacknowledged native input after abrupt process death. SDK-committed sends retain
the existing durable queue/idempotence owner and must not be replayed as new sends.

The lightweight toolkit-independent consumer replays the same typed
intents/observations as Tauri, including slow delivery, stale batches, multiple
consumers and narrow windows. Mobile support in this migration is a backend-neutrality contract. Its upper
layer may remain a thin adapter over the shared Rust contracts; it must not
recreate product semantics, publication, resource policy, or a second state
owner. Shipping or exercising an Android/iOS UI lifecycle harness is not
required here: shared Rust unit/consumer contract tests are the mobile
acceptance evidence. Android/iOS toolchain and SDK-store details are separate
platform work, not inferred as product behavior from a Linux compile. Only the
user-authorized native macOS wheel/trackpad qualification is deferred.

## Evidence required before design approval

This working draft deliberately does not invent success claims or a generic
subscription framework. Before implementation review, finish:

- concrete typed scope/payload and registration/drop/recovery APIs, reusing
  existing identities and ports, with all existing feature owners inventoried;
- bounded queue/window/byte/cache policies and measured reference workload
  budgets, distinguishing initial/recovery work from routine updates;
- source-backed SDK receipt-update choice without reimplementing placement;
- a consumer-by-consumer migration/removal map for snapshots, outcome waiters,
  Tauri, React, Browser Fake and headless QA;
- canon amendments and the precise first mergeable cutover, with retained
  tests and the already failing production-boundary regression as proof.

A separate Sol high review of actor queries/final-state handoff versus a
structurally shared internal root reached its 300-second deadline without a
choice or approval. Neither alternative gained approval from that attempt; do
not repeat its broad investigation unchanged or treat it as a cleared gate.
The common API is still unapproved despite the SDK representation approval and
small merged prerequisite changes.

The user-deferred Mac qualification needs an exact-build handoff. Other
acceptance gates in all three issues remain required.
