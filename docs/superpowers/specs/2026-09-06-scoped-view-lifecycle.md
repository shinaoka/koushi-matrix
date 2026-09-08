# Scoped view lifecycle: first concrete reader consumer

Status: **Correct-to-implement** within Sol's approved first receipt-reader vertical. Sol
read-only initial review found three Important issues: terminal delivery blocked
by ACK, missing installed-state budget, incomplete projection invalidations.
After the corrections below, Sol's targeted re-review passed all three with no
remaining finding in that scope. The subsequent full vertical re-review also
approved the concrete payload/UX choices after four material corrections. Companion to
[receipt reader vertical](2026-09-06-receipt-reader-vertical.md) and
[common contract](2026-09-06-scoped-publication-contract.md).

## Existing facilities, and why not to repurpose them

`ComposerDraftLeaseRegistry` (`composer_draft_lifecycle.rs:252–390`) provides a
useful bounded-notification pattern: mutex-protected records and a latest-only
`watch<()>` wake. Its semantics are NOT reusable as read-view leases: it has one
live renderer generation, composer scope identities, and persistence/command
permits. Starting one renderer generation retires all older generations. Applying
that behavior to independent read consumers would incorrectly retire another
window's subscriptions. Keep composer ownership unchanged; use one common view
registry for all migrated read surfaces, not one registry per feature.

Tauri's `core_event_forwarder.rs` owns a SECOND CoreConnection and emits legacy
broadcast events. A scope opened through the command connection cannot silently
be owned by that unrelated forwarder connection. A transferred owned subscription
must retain its originating connection identity. The legacy forwarder cannot be
used as an unbounded scoped-delivery queue.

## Public operations and ownership to review

- `CoreConnection::subscribe_view(spec)` creates an owned `ViewSubscription` for
  that connection. This vertical uses `TimelineReceipts` for bounded compact event
  summaries and `ReceiptReaders` for a full-reader window. Both use the same registry,
  delivery, dependency invalidation and final projection; neither embeds a new
  compact cache in TimelineItem. `ReceiptReaders` contains
  TimelineKey, the observed InitialItems projection request ID, public timeline
  generation, event identity and requested bounded window. `actor.rs:818` already
  keeps a stable `projection_request_id` across replay. The host does not invent
  or guess the private actor-generation gate value; Core resolves the observed
  source reference to that live owner and retains the private generation fence.
  `TimelineReceipts` omits the individual event ID and selects a bounded validated
  event window against the observed display revision. Do not declare unused future
  surface variants. Other required app-wide scopes
  later extend this same contract.
- `ViewSubscription::next_delivery()` yields a bounded complete reader-window
  model or a terminal retirement reason. Core gates additional MODEL delivery
  while an earlier model is unacknowledged, but terminal retirement bypasses that
  gate. The adapter keeps polling this method while awaiting ACK, so revoked
  content is not trapped behind model backpressure. It never yields an unbounded
  full reader collection. Subscription identity is opaque to the renderer and bound to its
  origin connection plus runtime lifetime; do not accept caller-created owner IDs.
- `ack_model(scope, revision)` acknowledges installation only. It is not visibility
  or permission to mark read/request images. Duplicate/older ACKs do not release a
  newer delivery; future/unissued revisions are rejected.
- `observe_view(scope, revision, visible_range_or_ids)` accepts geometry-derived
  membership only against the installed, still-current model. Rust derives demand
  and bounded prefetch from its own row identities. A scope/window request alone
  does not prove any row is visible.
- `request_window(scope, revision, range_or_anchor)` replaces the desired window;
  it does not enqueue all intermediate windows. Revisions and source identity
  fence late results. Keep the last installed model visible until a replacement
  is available unless its content/permissions are revoked or the scope terminates
  under the approved [bounded failure amendment](2026-09-06-scoped-capacity-amendment.md):
  required-work Capacity, CounterExhausted, or ProducerFailed. These distinct
  terminal reasons bypass ACK, clear only the derived view, and require a localized
  announcement; no automatic reopen, durable-data eviction, or ordinary-coalescing
  retirement is permitted.
- Explicit close and Drop retire the lease synchronously in the common registry
  and wake owners using latest-only notification, independent of data queue
  capacity. A retired scope cannot admit new work. Runtime/account/source actor
  retirement also invalidates it. The owned subscription retains its originating
  logical view-consumer context when moved to a task; dropping the original
  CoreConnection value alone does not invalidate that transferred subscription.
  Explicit consumer retirement closes all its scopes even if handles survive.
  Runtime shutdown closes every context. The registry must not hold strong lease
  references that prevent final-handle Drop. No implicit inference from the
  separate legacy event receiver is valid.

## Wire and host map proposal

Scope IDs are opaque canonical decimal strings minted by a checked process-wide
monotonic counter, so replacement CoreRuntime instances in the same host process
cannot reuse a stale scope ID. An external out-of-process transport must additionally
retire its connection on process replacement; IDs are not durable reconnect tokens.
Model revisions also use canonical decimal strings on JSON wire, avoiding JS number
precision as identity. Rust compares numeric counters internally. Range counts are
bounded by actual retained/window limits before conversion to native indices.

Subscribe failure distinguishes capacity, unavailable source, inactive session,
retired consumer and counter exhaustion without printing account/event/user data.
Window/ACK/observation commands look up the owning subscription, not a caller's
claim about its connection. Tauri's map is keyed by the authenticated webview label
and scope ID; the label comes from the injected window context, never an IPC string
argument. Each window's view-consumer context is independently retired. The map
owns forwarding handles/leases only, not an extra model or profile cache.

## Bounded handoff

Each scope retains its latest desired window, latest produced model and at most
one immutable unacknowledged model delivery. Separately retain a bounded trusted
identity/resource-lease mapping for the ACK-installed model; installed M1,
in-flight M2 and pending M3 are distinct states. ACK of M2 atomically replaces the
installed mapping and releases M1's retained resource leases. Account for that
mapping, in-flight and latest models, accepted/pending bounded raw windows, control
records and builder reservations in aggregate count/byte budgets; no artifact is
exempt because it is not encoded on the wire. New data replaces latest; it never
appends to a history. The first reader consumer can use complete bounded windows rather than
unused delta variants. The common count/byte caps apply before accepting a new
scope/window and before retaining a replacement. Capacity failure is an explicit
result; do not silently truncate reader totals or evict durable product data.

The Tauri adapter owns the subscription/forwarding task, emits only to the owning
window, and holds no second product cache. It must not emit another MODEL
while the prior one is unacknowledged, but continuously polls priority terminal
control. Retirement immediately invalidates observations/work admission and drops
retained model/raw/membership payloads in Core. The host clears its model on that
terminal signal independently of model ACK and acknowledges retirement separately.
Retired-but-unacknowledged control tombstones still consume the common scope-slot
budget, preventing close/reopen churn from building an unbounded host queue. A
voluntary final-handle Drop relinquishes the view; it needs no terminal delivery
to a receiver that no longer exists. Consumer/window retirement clears its entire
owned registry partition. Destroying that window aborts the task and drops the
subscription. A toolkit-independent native consumer uses the same owned
subscription directly. Transfer does not change Core ownership or authorization.

## Projection boundary to review, not an assumed shared profile cache

TimelineActor owns SDK receipt endpoints, derived order, count, and bounded raw
reader-window preparation. It already performs SDK profile lookup and emits the
resulting profile actions through a generation fence. There is no actor-local
ProfileState cache demonstrated in the current code.

A coherent final scoped model needs Rust's current room-context profile, alias,
locale and avatar readiness projection. Prefer the AppActor's borrowed current
state for final row enrichment, using bounded raw drafts in the common scope
mailbox, rather than adding a copied profile database to TimelineActor. Profile
mutation identities already exist after #849. Track all actual enrichment inputs:
room-member profile entries keyed by room/user, global and own profile entries,
local user aliases, avatar readiness/source references, and Rust locale/display
format policy. Mutation owners directly advance the affected scopes' projection-
dependency revision (global locale changes affect every live scope using it).
Do not classify action names or invalidate all scopes for one unrelated cached user.
Each final model records the dependency revision resolved with its source/window
revision; advancing a relevant dependency revision requires a new final model.
Only scopes referencing changed identities/context are invalidated. No full AppState clone is needed for this projection.
This introduces a real projection handoff, NOT a second receipt-order owner or a
new SDK profile-fetch route. SDK profile preparation can keep its existing path.

Raw draft and profile actions are independently scheduled today: do NOT claim
that a generation-fenced enqueue gives an atomic commit across both paths.
A final view must identify the accepted source/order revision and its resolved
profile projection revision, be produced without an intervening await, and be
replaced when a relevant profile update is applied. Source retirement is checked
again before final publication. Proposed exact scheduling: each raw draft carries TimelineKey, actor generation,
index revision and the scope's desired-window sequence. A newer desired-window
sequence rejects an older preparation result. Within the same actor/source and
window sequence, an older index revision also cannot replace an already accepted
or newer pending revision. Revision high-water marks survive relay recovery within
that owner; if an owner is replaced, retire its scopes rather than reset counters
under the same identity. AppActor accepts a pending raw draft
only while that source generation and lease remain live, resolves rows from its
current borrowed state without awaiting, then increments a per-scope final model
revision if the bounded model changed. It retains the accepted raw window for
reprojection after relevant profile mutations; it does not retain all readers.
Every relevant projection dependency above invalidates this scope directly in the
mutation/effect path, not through a lossy broadcast receiver. The complete resolved
dependency stamp is captured in the same await-free publication step. Profile preparation actions arriving
later may legitimately replace a fallback label with a resolved label in a new
revision; there is no promise of atomicity with the earlier SDK enqueue. A late raw
draft must always resolve against current state, never restore captured old aliases.
Pending drafts coalesce per scope and are processed in bounded fair turns; do not
rebuild all scopes in one unbounded actor turn. This scheduling still needs a
reviewer verdict before code; an SDK batch ID alone is insufficient.

## Required tests before enabling this consumer

Exercise the actual Core/host boundary: two connections to the same event;
Room/Thread/Focused identity separation; one lease closes while another remains;
window replacement during profile preparation; old ACK, stale observation and
no-ACK backpressure; full data mailbox during close; account/source replacement;
Tauri window destruction; source/model changes before a visibility observation.
Preserve keyboard/focus/locale reader coverage in the same vertical. These are
changed-boundary checks, not a Cartesian product or a new generic fault framework.

## Implementation verification obligations

1. Concrete Rust/DTO declarations and their integration with the reviewed lifecycle,
   including tested resource accounting and priority retirement transport.
2. Actual viewport sizing/keyboard/focus behavior and source retirement UX.
3. Full vertical design verdict covering those implementation choices; the narrow
   lifecycle review is not permission to begin the entire vertical.
