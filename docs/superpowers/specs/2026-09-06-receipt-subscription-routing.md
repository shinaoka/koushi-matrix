# Receipt subscription routing — next integration gate

Status: routing mechanics reviewed by Sol (GPT-5.6, high, read-only). Initial
ownership/registration/stamp verdicts passed; the scheduler finding passed on
re-review. The remaining capacity contradiction was resolved by the reviewed
[capacity amendment](2026-09-06-scoped-capacity-amendment.md), adopted into the
lifecycle canon before code. These mechanics may be implemented; host enablement
is still blocked on the explicit resource/readiness and integration conditions.
This is not a completed subscription API.

## Evidence and rejected advice

The current private round trip is CoreConnection → AppActor → AccountActor →
TimelineActor → CoreConnection → AppActor profile finalization. AppActor does not
wait for the SDK preparation reply. Publication is currently invoked only by
tests, despite a production-capable fenced publisher.

A bounded Flash read-only consultation confirmed that handle_command's existing
ReadReceiptWindow/ResolveReceiptWindow seam is the insertion point. Its claims
that only strong_count fencing exists, that ReaderLoading represents unavailable
sources, and that waiting for the SDK reply inside AppActor is safe are not
accepted: the code now has a validity-cell fence, unavailable sources retire, and
SDK waits must remain outside AppActor's bounded command turn.

## Required concrete flow

1. Keep the initial subscribe operation async: its caller may await the first
   coherent prepared model. Do not invent a requirement for immediate Loading.
   This does not waive subscribe-before-read or ongoing invalidations.
2. Before starting the existing read route, admit an owned scope plus its exact
   observed source and bounded desired window in the common registry. Reuse the
   original logical consumer, not the event forwarder's second connection.
3. Register dependency/source invalidation participation before that read. Changes
   during preparation advance the scope's desired/dependency state; they must not
   fall into an attachment gap. Initial counters and comparison semantics must be
   specified and checked, not filled with arbitrary constants in ReaderWindow.
4. Keep the SDK/profile preparation await in the caller/owned producer, using the
   existing bounded envelope/oneshot route and cancellation. A scope closing while
   work is queued must drop/reject its reply and release its charged draft.
5. Return the accepted raw draft through an AppActor envelope carrying scope,
   source/order revision and desired-window sequence. In one await-free turn,
   verify those against the registered context, borrow current profiles/locale,
   prepare resources and commit through publish_current. Return the owned handle
   only with a coherent initial result or a typed failure.
6. Retain the bounded unenriched raw input for relevant profile reprojection. This
   needs explicit ownership in the common scope lifecycle: close/retirement must
   release it independently of the data queue, not merely leave an AppActor map
   holding it. AppActor may index weak context handles, not another strong cache.
7. Ongoing source/dependency invalidations schedule coalesced bounded work after
   the initial subscribe future has returned. Identify the exact existing task
   owner and shutdown/cancellation seam before implementing; do not introduce a
   generic broker or clone a CoreConnection with reset request counters.

## Existing task ownership seam confirmed

`runtime.rs:295-330` already defines AbortOnDrop<T>, backed by the existing
executor::JoinHandle. AppActor's event-navigation preparation/deadline tasks use
it (`runtime.rs:870-871`). Reuse this ownership mechanism, not their unbounded
result channels. A scope-owned optional producer handle can be aborted from common
Control retirement/Drop; the worker must retain only a weak scope context while
awaiting, avoiding registry→task→strong-scope cycles. Completed drafts return via
the existing bounded command lane. Making the existing private task wrapper
crate-visible is preferable to introducing another cancellation wrapper.
This identifies the mechanism, not a reviewed implementation or a proof of prompt
payload release; raw reservations must survive until the actual last task owner
releases them.

## Direct invalidation seams traced

- `runtime/reducer_support.rs::reduce_app_action_state` is the synchronous
  post-reducer boundary. Invalidate registered inputs here before deferred awaits,
  not by subscribing to event_tx. Existing effects can carry the identities.
- Global/own profile and alias mutations already retain candidates in
  `UiEvent::ProfileChanged(ProfileDisplayChange { user_ids })` (profile.rs and
  reducer/mod.rs::profile_changed_effects). `runtime.rs::handle_ui_event_effect`
  currently consumes them only for legacy label projection. An empty user list
  must not be interpreted as "all users changed".
- **Missing room identities:**
  `live_signals.rs::handle_live_room_profiles_observed` mutates room_users but emits
  only LiveSignalsChanged. It must retain the exact changed room/user keys during
  its existing mutation loop. Recovering them by scanning ProfileState afterward
  would recreate the removed all-cache work and invalidate unrelated rooms.
- **Missing resource identity:** `profile.rs::handle_avatar_thumbnail_updated`
  emits an empty ProfileChanged, and may emit no profile event when an avatar is
  only present in a private reader hint. Preserve the updated MXC as a typed
  mutation input independent of whether a legacy ProfileState avatar changed.
  Scoped resource bindings, not all cached users, are the matching index.
- Locale invalidation should compare the actual small Rust display inputs used by
  the projection at the synchronous reducer boundary. SettingsChanged alone is
  too broad (read-receipt policy, for example, is unrelated to timestamp locale).
- `timeline/relay.rs` obtains net endpoint changes from
  `receipt_endpoints.apply_batch` before consuming them into the legacy full map.
  Notify only source-key/event memberships present in that net change. A removed
  event and a present event with zero readers remain different. Recovery and actor
  retirement require the same direct ownership path, not a lossy broadcast.

### Registry context ownership proposal

Keep accepted/pending charged raw drafts and one optional AbortOnDrop producer in
registry-owned per-scope control, alongside desired-window and dependency/source
high-water state. AppActor keeps only weak handles/index entries. Register source
participation before first read, and selected user/room/resource dependencies
before final publication in the same AppActor turn. Mutation hooks advance only
indexed matching scope state and coalesce a wake on the existing registry control
mechanism. A changed dependency during initial preparation must be resolved from
current state, not discarded with the pre-read dependency set. For an initial NotRequested hint, the actual row producer now probes the existing
avatar cache by its current MXC and acquires a charged lease if bytes already
exist, closing the successful-cache case without another HTTP request or cache.
Loading/Failed states remain unchanged. Empty legacy profile events still cannot
support subsequent resource invalidation; the typed MXC mutation path is required.

The stamp/registration algorithm below has been reviewed as described in the
status above. Do not add identity effects that no production subscriber consumes
and call that subscription routing complete.

## Selected routing mechanics for review

### One owner, not an AppActor payload map

Add a reader context to the existing registry Control. It owns the immutable
observed source, latest desired target/limit/sequence, accepted raw draft, latest
pending draft, source high-water, dependency revision, dirty flags, initial reply,
and at most one AbortOnDrop producer. AppActor indexes only Weak<Control> handles.
A producer upgrades that weak handle only for short admission/commit operations,
never while awaiting SDK/queue capacity. Retire/Drop clears raw slots and aborts
its producer through the existing control path; task-local artifacts retain their
reservations until actually dropped. No task owns a strong scope/control cycle.

### Stamps and admission

Initial desired-window sequence is zero. Subsequent requests require the installed
revision and a strictly newer client sequence; exact duplicates are idempotent,
conflicting duplicates/unissued revisions fail. The existing installed identity
mapping authorizes anchors. A result carries the admitted sequence and is rejected
if it is no longer desired.

Each new logical receipt epoch gets a checked process-monotonic source revision,
including replacement/recovery. No-op roots retain it. Exhaustion invalidates the
scoped source and returns CounterExhausted, never wraps or panics; legacy SDK data
must remain usable. Preserve the highest accepted/pending source revision while
that owner remains live. Private actor-owner replacement retires the context.

Dependency revision starts at one for the scope's initial projection and advances
on indexed relevant mutations; checked exhaustion retires the context. First-row
resolution uses current AppActor state and registers its selected dependencies in
that same await-free turn, so profile changes before first registration are part
of the initial state, not lost broadcasts. Later mutations dirty registered scopes.
Resource-state observation must additionally satisfy the precondition below.

### Bounded work through existing lanes

Admit/register the context before the first read. The initial async subscribe
future owns its handle while awaiting the initial reply; cancellation drops it.
Raw producers use the existing command/account/timeline lanes and send completed
work back through the bounded command lane, never an unbounded result channel.
Each context has one active raw preparation and one replaceable desired request.
A registry-owned deduplicated FIFO of dirty scope IDs (at most 64) plus one Notify
wakes AppActor; drain one projection completion per fair turn, not all scopes.
Source-dirty work refetches; dependency-only work reuses accepted unenriched raw.
New dirty state arriving during work remains set and schedules the next turn.

Use one registry-locked phase enum Idle/Queued/Running plus dirty reason bits.
Invalidation atomically ORs reasons: Idle becomes Queued and is appended once;
Queued is not appended again; Running retains reasons without enqueuing. Popping
an entry atomically changes Queued to Running and captures/clears its serviced
reasons. Running lasts through delivery and handling of the completion envelope,
not merely until the worker sends it. Completion/rejection clears Running, then
appends exactly once at the tail if dirty reasons remain; otherwise it becomes
Idle. Stale result rejection preserves any newer dirty reasons. Retired entries
are removed/skipped; they never requeue. No popped work waits invisibly outside
this phase machine. Window replacement coalesces while a bounded raw preparation
is active; scope retirement aborts it. Unexpected producer exit must retire its
context through control rather than leave Running forever; an abort/drop guard
uses only a weak context and no blocking data-queue send for that path.

Reserve raw/builder capacity before launching work, transferring RAII ownership
with the draft. Initially use a conservative 64-MiB reservation for an active raw
preparation under the existing 256-MiB ledger, then shrink to measured retained
application data before acceptance. An initial reservation failure immediately retires/drops the provisional context
and returns typed Capacity to the subscribe future; it does not await a future
budget release. For already-live scopes, failure to reserve raw/builder/model
capacity terminates that derived subscription with an explicit Capacity retirement
reason, independent of ACK. Do not evict durable data, return an incomplete model,
or silently keep a permanently stale subscription. Reopening is a new explicit
request, not an automatic retry loop. Counter exhaustion similarly uses an explicit
CounterExhausted retirement reason, and unexpected producer exit uses ProducerFailed;
add these closed reasons and native localized terminal handling in the same
vertical before host enablement. This removes budget-wait requeue/spin paths.
The 64-MiB raw/builder reservation is a hard pre-acceptance ceiling: an oversized
result is dropped while still charged and fails Capacity, never grown silently or
truncated. This is a conservative application-data admission policy, not a heap
estimate or permission to exceed any scoped/string/data bound. Accepted/pending raw and model
reservations remain independently accounted; replacement must not temporarily
release the old charge before new admission succeeds.

Finalization borrows current profiles, captures desired/dependency/source stamps,
prepares outside the source fence, and rechecks all stamps at the existing fenced
commit. Register input membership before exposing the first model. Initial errors
distinguish capacity, inactive session, unavailable source, closed consumer and
counter exhaustion; later source retirement uses terminal control.

### Explicit resource precondition, not an accepted shortcut

The successful cached-byte case is implemented, but hint-only avatars cannot
reconstruct a prior Failed/Loading outcome from ProfileState if that user has no
profile entry. The resource-demand integration must expose authoritative current
readiness (including failures), reusing/replacing the existing account avatar
cache rather than adding a per-scope authoritative resource cache. Typed MXC
mutation identity alone does not supply this missing current-state read. Do not
enable host subscriptions until that state/readiness seam and subsequent direct
invalidations are integrated. This remains a concrete design question, not a
waiver of failed/loading-state behavior.

Review these choices before implementation. A first-frame-only helper is not an
acceptable substitute. Host/UI/HTTP/performance and all remaining #840/#846
obligations remain required.
