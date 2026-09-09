# Receipt reader vertical: concrete next-boundary proposal

Status: **Correct-to-implement**, Sol read-only full-vertical re-review after four
material corrections (scoped compact join, recovery, anchor protocol, focus).
No implementation or acceptance test is claimed. The overview canon records this
approved migration target before code changes. This is the next vertical use of
[scoped publication](2026-09-06-scoped-publication-contract.md), not a competing
subscription API or completion of #840.

## Existing seams and constraints

- `timeline/actor.rs:1394–1413` maps initial SDK items one-for-one into raw Core
  navigation items. Do not assume later display projections retain SDK positions.
- `timeline/relay.rs:1440–1606` receives initial SDK items and ordered VectorDiff
  batches. Overflow terminates that relay generation and requests resubscription;
  there is no safe continuation after dropping a positional batch.
- Current `live_receipts_action_from_sdk_diffs` sees only new SDK values, so it
  cannot derive net changed readers from old/new receipt endpoints.
- `state/live_signals.rs::normalize_receipts` currently deduplicates by user,
  excludes own user, sorts timestamp `unwrap_or_default()` descending then user ID,
  and retains ALL enriched readers. None and timestamp zero have equal sort rank.
  Later equal-timestamp duplicates replace earlier ones. Preserve these semantics.
- `ReceiptReaders.tsx` currently expands ALL preformatted details in a tooltip;
  waiting for an explicit full-reader scope must not replace that with silently
  truncated details. Hover/focus preview and an accessible full-reader action need
  explicit separate behavior.
- Avatar readiness already carries portable `source_ref` values such as
  `avatar/<hash>`; existing media ports resolve them. Do not invent a replacement
  resource-ID wrapper or expose paths/data URLs. Relevance/demand ownership is the
  missing boundary, not this existing portable reference format.

## Bounded owner consultation and remaining correction

A read-only Flash consultation traced initial hydration (`actor.rs:1465,1670`),
live diffs (`relay.rs:495,714`) and authoritative recovery
(`relay.rs:936,1041,1127`). It recommends TimelineActor for the derived order and
bounded SDK profile preparation, reusing its session and generation-fenced send.
This is a diagnostic owner recommendation, NOT approval of the scoped API or this
implementation. The earlier broad Sol consultation timed out without a verdict.

Parent corrections to the consultation: there is not yet a full SDK item mirror;
that is proposed work. `overflow_count: 0` is consistent with today's retained-full
reader representation, not by itself a defect. A fenced enqueue must not be
misreported as atomic commit of all independently emitted timeline/state events.
Do not fix the unrelated initial-send return handling without a demonstrated bug.

Most importantly, `timeline_room_id` (`item_projection.rs:1335`) accepts Room,
Thread AND Focused keys, but current receipt actions collapse all three into the
same room/event map. A new actor-local index/full-reader scope cannot preserve that
collapse as its identity: a room-only lookup can select another timeline actor's
placement or let one actor's retirement remove another's model. Bind the index,
compact projection and full-reader scope to TimelineKey + actor generation + event.
A reviewed transition must replace the room-only receipt lookup, while keeping
room-scoped typing/fully-read state separate. Profile provenance still resolves in
room context. This issue must be settled before implementing the vertical.

Authoritative recovery removes the old/new event-ID union and then installs the
new entries (`reducer/live_signals.rs:88–123`). Preserve replacement semantics at
the NEW scoped owner, not as an incremental merge or a global room deletion.
During authoritative recovery under the SAME private actor owner: surviving event
IDs replace their complete indexes atomically and advance monotonic source revisions;
removed event IDs retire dependent scopes; added IDs initialize fresh indexes.
A changed public relay generation is reflected in the next delivered source
reference, but does not by itself retire a surviving event's scope. Stale drafts
from the earlier relay generation must be rejected. Replacing the private actor
owner always retires its scopes; do not transplant them to a new actor.

## SDK endpoint changes, not another receipt stream

Add a public borrowed per-user logical change iterator to the approved opaque SDK
snapshot. Compare `Receipt.ts` and `Receipt.thread` explicitly; Ruma Receipt has no
PartialEq. Ignore ordering-slot changes caused by swap-remove. An SDK-only record
wrapper may define logical equality excluding its bookkeeping slot; explain that
choice and test it. Expose add/update/remove, not raw imbl node types or historical
journals. Unrelated reconstructed maps have an explicitly accounted full comparison.

Retain a read-only SDK-position receipt-endpoint mirror in TimelineActor's accepted
relay-batch processing, seeded from the initial SDK items. No message bodies are
retained by this mirror, and no extra endpoint payload is queued alongside the
existing SDK diff batch. Release the relay's initial SDK vector after bounded UTD
tracking is seeded. Apply SDK VectorDiffs in order; capture the first-before and
last-after endpoint for each affected event in the SAME accepted relay batch. Coalesce repeated sets, remove+
reinsert and add+remove to their net result. On overflow/drop, no rejected receipt
change is published; discard the generation with existing resubscription behavior.
This mirror is a consumer of SDK placement, never another placement authority.
Do not address SDK positions through a renderer/display-order index.

## Single-owner derived order, not a cloned AppState field

Within each TimelineKey/actor generation, for each currently retained remote event,
keep the latest cheap SDK receipt snapshot and an ordered key vector `(timestamp_or_zero, user_id)` in the Core
receipt projection owner. The snapshot itself supplies old receipt lookup, avoiding
another full by-user record database. Update order by key search/remove/insert for
net changed users. Do not clone the full ordered vector into AppState/watch/UI
snapshots. Only bounded row projections leave this owner.

The imbl Vector primitive with a single owner measured 0/256/256 key clones at
32/1500/100000 readers for one middle move plus a 20-row indexed window, versus
32/1600/8448 when retaining a prior full vector. These are algorithm probes, not
Core latency or memory acceptance. Production ownership/retention must be tested.

Compact output is all readers through four, three plus exact overflow from five.
Resolve/enrich only these identities and explicitly observed full-reader windows.
Do not query all hidden reader profiles to prepare a compact result. Preserve raw
profile provenance where required; do not feed a previously resolved alias back
as an original Matrix display name. Event eviction/replacement must reclaim its
derived index and retire dependent full-reader scopes with an explicit unavailable
result, not retain an unbounded history or fabricate a zero count. This retirement
behavior requires review before code.

## Selected compact transport: scoped Rust model join

Reject the earlier embedded-TimelineItem candidate. A bounded follow-up source
consultation established that existing `DisplayLabelsUpdated` carries only global
user labels, not room-specific receipt labels, avatars or timestamp formatting.
Embedding compacts would require a new loaded-row reprojection owner/cache or a
broader timeline publication handoff. Neither is justified for this vertical.

Instead, use a `TimelineReceipts` model in the SAME common scope registry/transport
as `ReceiptReaders`. Its source is TimelineKey + observed projection request ID +
public generation; its bounded event selection is validated against the actor's
current display identity/revision. TimelineActor prepares compact raw drafts only
for that selection. AppActor enriches and directly invalidates these bounded drafts
using the same dependency contract as full-reader windows. The renderer joins
already-projected summaries by event ID; it does not sort, count, resolve profiles
or request arbitrary MXCs. A bounded adapter leaf formats delivered timestamps
with Rust-selected locale and fixed style, as defined in
[the approved formatting amendment](2026-09-06-receipt-formatting-amendment.md). This is one owner per fact, not another
room-level cache or feature-specific bus. Do not add receipt fields to TimelineItem.

Delete the old room receipt map and its full-reader publication in the same
cutover. No embedded compact cache or legacy room-map fallback remains. Full reader
records stay only in the actor-owned SDK-derived index. The common registry retains
only the selected compact/window raw and final models under its existing budgets.
Sol's targeted full-vertical re-review approved this selection with no remaining
material finding.

`event_projection.rs` currently projects timeline display labels in
`CoreConnection::project_event` (`connection.rs:808`), using the latest AppState.
There is no demonstrated actor-local shared ProfileState cache to reuse. Therefore
do not assume TimelineActor can independently produce final alias/locale-aware rows
from SDK member data. Reuse or deliberately replace that existing Rust projection
boundary. Both scoped receipt models use AppActor finalization; the existing
CoreConnection sender-label path is not repurposed for receipts. Each receipt
model's revision changes for relevant projection dependencies, not merely order.
No new full-profile clone or second profile database is acceptable.

The timeline observation must qualify both its committed timeline display revision
and installed receipt-scope revision. Core rejects visibility/end evidence while
either relevant model is stale, including a delayed first receipt footer that
changes layout after the SDK item batch. This extends the required #846 revision
boundary; it cannot be implemented as a React unread-state repair. A receipt scope
is a product-data owner only: the one timeline engine still owns all row geometry.

The proposed concrete lifecycle and host boundary are recorded in
[scoped-view-lifecycle](2026-09-06-scoped-view-lifecycle.md). It explicitly excludes
repurposing the composer registry's single-renderer retirement semantics and the
Tauri forwarder's unrelated second connection as subscription ownership.

## One real scoped consumer first, extensible to the required app-wide migration

Use the common connection-owned scope lease/model identity for ReceiptReaders;
do not add a receipt-only parallel bus. The first concrete scope carries room/
event/actor-generation identity, model revision, total count, window origin and
bounded reader rows. Native hosts request revision-qualified ranges/anchors and
report installed-model visibility separately. No caller-supplied MXC demand lists.

Start with bounded complete window deliveries where sufficient: one coherent
latest view, one unacknowledged transport delivery per consumer, explicit model
ACK and latest replacement on gaps. Do not add unused delta variants merely to
anticipate other scopes. All mailbox and host queues must obey the common bounds;
lease retirement cannot depend on the full data queue. Subsequent required scopes
must reuse this same lifecycle, not coexist permanently with full-state publication.

Reader rows reuse portable avatar readiness and Rust-resolved labels/time display.
A full-reader surface requests only a window; visible rows plus Rust's bounded
prefetch determine profile/avatar demand. Compact timeline avatar demand must be
bound to observed timeline models too; the existing unrevisioned first/last-visible
observation is insufficient proof of current membership. Do not claim #839 complete
while other avatar surfaces still dispatch unrestricted legacy demand.

Reuse the floating-layer and existing platform media ports. Full list needs bounded
DOM, correct total/row indices, keyboard navigation, focus return, RTL/locale and
resize handling. No all-readers details array or all-readers tooltip survives.

The concrete payload and keyboard/window proposal is in
[receipt-reader-surface](2026-09-06-receipt-reader-surface.md). Its selected choices
passed the full vertical re-review; they are not passed UI/accessibility tests.

## Implementation and verification checklist (design approved)

- Concrete scope payload/command types and host ACK/lease integration seams.
- Exact revision source covering receipt/profile-induced model changes, not just
  SDK batch IDs (these alone do not prove every display-model change).
- Renderer row sizing/window/focus behavior and its existing test migration.
- Mutation and lifecycle tests including mixed SDK batches, overflow/recovery,
  event retirement, account replacement and concurrent consumers.
- Same compact-profile RED turns green with total1500 retained; actual Tuwunel/
  Synapse avatar request evidence, not just this primitive or DOM counts.
