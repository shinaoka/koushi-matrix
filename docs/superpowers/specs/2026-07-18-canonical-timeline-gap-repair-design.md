# Canonical Timeline Projection and Demand-Driven Gap Repair

**Date:** 2026-07-18

## Status and Scope

This design supersedes the historical-gap follow-up in the active live-tail
freshness design. It retains room-scoped live-tail refresh, causal fencing,
room priority, cancellation, and privacy-safe diagnostics.

It addresses two production failures without assuming they have one exclusive
cause:

1. a sent event appears twice, once as a local echo and once as the confirmed
   remote event;
2. a visibly rendered persisted gap repeatedly fails to repair.

The duplicate is a confirmed indexed-diff contract failure. The gap failure
has a separately confirmed dead repair path. Timeline display projection can
also affect viewport-to-gap mapping, but that interaction remains a hypothesis
until the regression test below proves or disproves it.

## Production Evidence

### Confirmed: indexed diff divergence

The SDK/Core timeline emitted a `Set` and then a `Remove` at raw index `1699`.
The UI-owned item vector contained about `1609` entries because it had silently
deduplicated stable IDs. The UI ignored the out-of-range operations and then
accepted the confirmed event's `PushBack`, leaving both the local echo and the
remote event visible.

An indexed diff is meaningful only in the exact vector that produced it. A
consumer may not change that vector's length and continue applying its indices.

### Confirmed: automatic repair dead path

The diagnostic record contains two remaining gaps and 24 repetitions of:

```text
started -> incomplete -> offscreen
```

There is no successful repair, and `batches_processed` remains fixed at `32`.
Automatic repair passes `cached_chunk_limit=0`; when the selected persisted gap
is not resident in the in-memory linked chunk, the SDK returns `Deferred { 0 }`
before loading the descriptor-bearing chunk. Core then classifies that result
as offscreen. The room-lifetime batch count prevents fresh user demand from
receiving a fresh budget.

### Unproven: thread display projection contribution

`latestReply` moves a thread root to a reply's `activity_event_id`. The UI
reports viewport bounds using activity IDs, and Core maps those IDs back into
its canonical items. A summary-only activity absent from the canonical vector,
or a projection crossing a gap, can make that reverse mapping incomplete or
non-representative. This is plausible but must not be treated as established
until a focused test or mapping diagnostic demonstrates it.

## Invariants

1. There is exactly one indexed display vector for each timeline generation.
2. Every emitted `Insert`, `Set`, `Remove`, and `Truncate` index refers to that
   vector and no other vector.
3. UI consumers apply snapshots and diffs verbatim. They do not deduplicate,
   reorder, clamp, or silently ignore invalid indices.
4. Stable-ID reconciliation occurs before display-space diffs are emitted.
5. A gap is not a member of the indexed event vector. It is a separate stable
   descriptor anchored to event/chunk identity.
6. Visible user demand can repair a nonresident persisted gap without depending
   on an arbitrary room-lifetime batch count.
7. Thread presentation and viewport identities are produced in the same display
   space used for gap visibility.

## Ownership Model

### SDK: raw cache and private pagination

The SDK remains authoritative for:

- linked-chunk persistence;
- private pagination tokens;
- local-to-remote echo reconciliation inputs;
- decryption and event-cache mutation;
- exact gap/chunk lookup and pagination.

The raw SDK vector is an implementation detail of Core. Its indices never cross
the Core/UI contract.

Gap inspection returns an opaque stable repair handle plus adjacent event
anchors where available. The handle identifies the persisted gap independently
of its current ordinal and does not reveal the pagination token. Repair accepts
that handle, loads its exact persisted chunk when necessary, and paginates it.

### Core: canonical display projection

Core owns the only indexed vector exported to consumers. It maintains an exact
raw SDK mirror, then projects it into display rows with:

- stable row identity;
- content event identity;
- activity event identity;
- reconciled local/remote echo identity;
- thread-root placement according to `rootEvent` or `latestReply`;
- hidden/date-divider presentation rules required by every UI consumer.

Core compares the prior and next display vectors and emits display-space diffs.
Structural fallbacks may use `Reset`, but ordinary append, replacement, and
removal must remain incremental so live messages do not reset a long room.

The existing TypeScript thread projection is removed after equivalent Rust
projection tests pass. There must never be two independently mutable display
projections.

### UI: strict consumer and renderer

The UI stores the exact Core display vector. Applying an invalid index is a
contract failure: development/test builds throw immediately, while production
records a privacy-safe invariant violation and requests a generation-fenced
snapshot reset. It never continues from silently corrupted state.

Virtualization, measurement, and DOM rendering may derive transient windows,
but those windows cannot become a target for Core diffs.

## Gap Descriptor and Rendering

A projected gap has stable identity rather than a raw array position:

```text
GapDescriptor {
    repair_handle,
    generation,
    older_event_anchor?,
    newer_event_anchor?,
    topology_revision
}
```

The SDK handle drives repair. Event anchors drive display placement and
viewport intersection. An open boundary is represented explicitly rather than
inventing an index. If both anchors disappear in a new topology revision, Core
invalidates and re-inspects the descriptor.

Core emits gap descriptors alongside the canonical display vector. The UI
weaves a gap row into render output without inserting it into the diff target.
The gap row therefore cannot shift subsequent diff indices.

`topology_revision` remains a full-range `u64` inside Rust. Across the
Serde/Tauri/TypeScript boundary it is encoded as a canonical unsigned decimal
string. JSON numbers are rejected rather than rounded, and TypeScript never
parses the value through `number`. The DOM dataset, viewport signature, and
return command preserve the same string byte-for-byte before Rust validates
and parses it back to `u64`.

Viewport observations contain the first and last visible activity identities
plus visible gap handles. Reporting a visible gap handle directly avoids
inferring gap visibility only from two surrounding event indices. Core treats
an explicitly visible handle as foreground repair demand after validating its
generation and topology revision.

## Demand-Driven Repair State

Repair state is keyed by stable gap handle and topology revision, not only by
room:

- `Idle`
- `Queued { demand }`
- `LoadingDescriptor`
- `Paginating`
- `AwaitingProjection`
- `Complete`
- `Retryable`

Demand priority is:

1. an explicitly visible gap in the selected room;
2. selected-room live-edge freshness;
3. a selected-room near-viewport gap;
4. delayed work for inactive rooms.

Switching rooms cancels the old room's network phase and retains committed
cache state. Re-selecting the room creates new foreground demand.

Budgets are scoped to one handle/topology/demand attempt. A topology change,
newly visible handle, manual retry, or room re-selection receives a fresh
budget. The SDK loads the descriptor's exact persisted chunk; it does not use
`cached_chunk_limit=0` as an offscreen proxy. Network pagination remains bounded
per request, and further pages are scheduled only while the same validated
demand remains active.

The safety fence counts consecutive no-progress outcomes, not all batches.
`Deferred { cached_chunks_loaded > 0 }`, a positive event count, a joined
boundary, or another explicit advancement resets that counter. A batch that
loads one cached chunk therefore cannot exhaust the attempt merely because the
selected descriptor is more than 32 chunks away. Thirty-two consecutive
no-progress outcomes still fence the attempt. Total batches remain diagnostic
only and never strand a visibly demanded descriptor that continues to advance.

`LiveTailSnapshot` remains observe-only: it publishes authoritative topology
without consuming a historical token. It must not pre-consume the eligibility
of a subsequently visible gap. An unchanged viewport does not need to wake
again after a repair-capable attempt has actually been admitted; observing a
descriptor alone is not such an attempt.

## Failure and Recovery

- `Deferred` means descriptor/cache work remains and is never translated to
  `offscreen` without viewport evidence.
- `Offscreen` means the handle is not explicitly visible and its anchors are
  outside the validated display viewport.
- Invalid display diffs force a snapshot recovery rather than state mutation.
- A missing/obsolete gap handle triggers topology re-inspection.
- Retryable network failures retain the descriptor and do not consume future
  user demand permanently.

## Diagnostics

Retain the existing structured repair outcomes and add:

- raw-vector length and display-vector length;
- diff operation, requested index, and in-range boolean;
- local/remote reconciliation outcome;
- viewport first/last identity resolved booleans and resolved index buckets;
- visible gap handle count and whether the requested handle validated;
- descriptor residency and exact-chunk load outcome;
- per-handle attempt number and reset reason;
- topology, ordinal, and demand revision change booleans;
- consecutive no-progress count and remaining safety budget;
- cached chunks loaded for the completed batch.

Diagnostics must not contain room IDs, event IDs, gap handles, tokens, message
content, or raw server errors.

## TDD Regression Sequence

Implementation starts with these failing tests, in this order:

1. **Nonresident visible gap:** persist a gap outside the in-memory linked chunk,
   open a viewport containing its rendered gap row, and assert that the exact
   persisted chunk loads and the repair succeeds. Assert no `Deferred { 0 } ->
   offscreen` loop.
2. **Echo index translation:** seed a raw vector whose normalized display vector
   is shorter, then apply the production-shape `Set/Remove(N)` followed by the
   confirmed `PushBack`. Assert valid display-space diffs and one final row.
3. **Strict UI contract:** feed an out-of-range display diff and assert invariant
   failure plus snapshot recovery, never silent continuation.
4. **Thread modes and visible gap:** run the same gap fixture with `rootEvent`
   and `latestReply`, including a reply activity across the gap and a
   summary-only root. Assert the visible handle selects the same repair.
5. **Budget reset:** exhaust one handle/topology attempt, make the handle visible
   under a new topology revision, and assert a new bounded attempt starts.
6. **Full-range wire identity:** round-trip a topology revision greater than
   `2^53` from Rust through JSON, TypeScript, the DOM dataset, the viewport
   command, and back into Rust. Assert that numeric JSON is rejected.
7. **Long cache path:** return at least 33 consecutive
   `Deferred { cached_chunks_loaded: 1 }` outcomes for the same visible demand
   and assert repair remains admitted. Return 32 true no-progress outcomes and
   assert the safety fence stops the attempt.
8. **Live movement:** append new events so a visible gap moves into history,
   scroll back to it, and assert explicit visible-handle demand resumes repair.
9. **More than 128 missing:** refresh a detached live tail, render the latest
   events immediately, and repair the persisted boundary through the same
   handle-driven path.
10. **Room switch:** cancel the old room's network phase, prioritize the new
   room, then re-select the old room and assert a fresh foreground attempt.
11. **Thread projection crossing:** move a `latestReply` thread root across the
    gap and assert that the projected gap identity and visible demand remain
    unchanged relative to `rootEvent` mode.

At least one test must cross SDK fixture, Core actor, serialized desktop event,
TypeScript store, thread projection setting, DOM viewport observation, and the
repair command. Component tests alone are not sufficient release evidence.

## Migration and Rollback

Remove:

- UI stable-ID deduplication while applying indexed diffs;
- silent out-of-range `Set`/`Remove` handling;
- raw `before_item_index` gap insertion into the UI diff target;
- automatic `cached_chunk_limit=0` offscreen classification;
- room-lifetime `batches_processed=32` as a permanent repair fence;
- observe-only projection recording a candidate as already repaired.

Retain:

- token-free selected-room live-tail refresh;
- causal generation and projection fences;
- room priority and phase-aware cancellation;
- stable-ID deduplication semantics, moved upstream into Core projection;
- privacy-safe `started`, `incomplete`, `deferred`, `offscreen`, `repaired`, and
  budget diagnostics;
- observe-only `LiveTailSnapshot` semantics.

The migration is complete only after the production-shape vertical tests pass
in both thread display modes and a real persisted-cache replay closes the gap
without duplicating a sent event.

## Rejected Alternatives

- **Patch the current UI index map:** preserves two mutable index spaces and
  requires permanent raw-to-display translation in every UI consumer.
- **Treat the SDK raw vector as the UI vector:** simpler, but spreads thread and
  dedup projection responsibility across consumers and does not provide one
  viewport/gap coordinate system.
- **Increase the global 32-batch limit:** delays the same dead path and cannot
  represent new user demand.
- **Repair the newest ordinal:** ordinals change as chunks load and do not prove
  that the selected gap is visible.
- **Add another timer/scheduler wake:** cannot make a nonresident descriptor
  loadable and repeats the failed symptom-level strategy.
