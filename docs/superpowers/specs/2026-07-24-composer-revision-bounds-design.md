# Composer Revision Bounds and Overflow-Safe Wire Design

**Date:** 2026-07-24

**Status:** Approved after main frontier-model review

**Issue:** #294

**Depends on:** merged #293 / PR #295

## Context

Issue #293 added a causal fence that prevents a delayed composer-draft write
from restoring text after an accepted main, reply, thread, scheduled, or
prepared-upload submission. Rust now retains a monotonic revision for every
main-room and thread-root `ComposerTarget`, and the frontend keeps a matching
target-keyed revision coordinator.

That fence is correct for the tested completion orders, but its long-lived
representation has three gaps:

- `ComposerDraftStore.room_revisions` and `thread_revisions` grow for every
  touched target until the room disappears or the account is cleared. The
  persistence copy has room/thread count limits, but those limits use ordered
  map selection and do not bound the live state.
- The frontend revision coordinator, main/thread local overlay maps, debounce
  timer maps, and main/thread IME clear-epoch maps have no per-target retirement
  contract.
- Rust exposes revisions as `u64`, Tauri serializes them as JSON numbers, and
  TypeScript stores them as `number`. Values above
  `Number.MAX_SAFE_INTEGER` lose precision. The reducer's
  `saturating_add(1)` also turns `u64::MAX` into a fence that can never advance.

This design preserves #293's accepted-send semantics. It changes only revision
representation and the lifecycle of the live and persisted revision
tombstones.

## Goals

- Bound quiescent empty revision tombstones in live Rust state and in the
  frontend.
- Never collect a target that still has non-empty local content, is visible and
  active, or can produce or receive an unsettled revision-bearing operation.
- Give every collection decision a lifecycle proof; target name ordering is
  never evidence that a write is dead.
- Carry revisions across Rust, Tauri, snapshots, TypeScript, browser fakes, and
  encrypted persistence without JavaScript-number rounding.
- Fail closed before a Matrix or persistence side effect if the finite internal
  revision space is exhausted.
- Preserve account, main-room, and individual thread-root isolation.
- Preserve encrypted legacy-store migration and private-data-free evidence.

## Non-Goals

- Changing the accepted/rejected/unknown submission state machine from #293.
- Moving draft bodies into React product state or exposing the full draft map
  to the WebView.
- Persisting process-local lease identifiers.
- Making revision values meaningful to presentation code.
- Increasing the existing 16 KiB per-draft content limit.
- Defining a general cache framework for unrelated target-keyed frontend maps.

## Existing Ownership That Remains

- The DOM owns active composition and an unacknowledged local edit.
- Rust owns the authoritative per-account draft content, revision fence,
  accepted clear, and encrypted persistence.
- Tauri validates and transports typed commands; it does not create a second
  draft state machine.
- A draft write applies only when its revision is strictly greater than the
  target's authoritative revision.
- An accepted operation advances to
  `max(authoritative_revision, submitted_revision) + 1`. It clears content only
  when there is no newer draft and otherwise preserves the newer content at the
  advanced revision.
- Every command captures the complete account owner and exact main-room or
  `(room, thread-root)` target. Account changes are terminal fences.

## Considered Approaches

### 1. Apply the persistence quotas directly to the live ordered maps

This would take the first 128 room keys and 256 thread keys, or delete the
lexicographically first/last empty entries. It is small, but target identifiers
do not encode lifecycle. It can remove a tombstone while a debounce timer,
queued Core command, accepted submission, scheduled send, or prepared-upload
clear still carries an older revision. That delayed write would then compare
against zero and could be admitted.

This approach is rejected.

### 2. Replace revisions with random UUIDs or a resettable epoch

Opaque random values are wire-safe, but equality alone cannot decide whether a
delayed write is older or newer. Adding parent links or a retained epoch table
recreates an unbounded per-target history. Resetting an epoch after collection
also needs the same proof that no old producer remains.

This approach is rejected. A lifecycle proof is required regardless of token
shape.

### 3. Arbitrary-precision decimal revisions plus protected LRU collection

An arbitrary-precision integer avoids exhaustion. It also adds a big-integer
dependency to Rust and permits untrusted wire input to request unbounded parse
and allocation work. Imposing an input digit limit returns to a checked finite
space.

This is viable but not selected because it adds complexity without a practical
product benefit.

### 4. Canonical decimal-string `u128` revisions plus lease-protected LRU
collection

Rust uses a checked `u128` newtype. The wire and encrypted store use canonical
decimal strings. A target is collectible only after its content is empty and
all active and unsettled producer leases have ended. Only those quiescent
tombstones enter bounded oldest-to-newest queues.

This is the recommended contract.

## Recommended Revision Contract

### Type and grammar

Introduce one Rust newtype, `ComposerDraftRevision`, and one branded TypeScript
type with the same wire grammar:

```text
ComposerDraftRevision := "0" | [1-9][0-9]{0,38}
```

The parsed value must be in `0..=u128::MAX`. Leading zeroes, signs, whitespace,
fractions, exponents, empty strings, and values above `u128::MAX` are invalid.
The wire value is opaque outside the shared revision helper; components must
not compare strings or convert them to `number`.

Rust compares the `u128` newtype. TypeScript compares a temporary `bigint`
created only by the shared helper. TypeScript never stores the parsed value in
a snapshot, command payload, log, or JSON fixture because JSON has no `bigint`
representation.

`"0"` remains the absence/default revision. A fresh target's first edit is
`"1"`.

### Checked successor

All revision creation uses one operation:

```text
checked_successor(max(authoritative, submitted_or_local))
```

The operation returns either the next `ComposerDraftRevision` or the typed
failure `composer_revision_exhausted`. No reducer, fake backend, Tauri helper,
test harness, or compatibility action may use wrapping or saturating addition.

Exhaustion fails before any of these effects:

- changing authoritative draft state;
- clearing a composer;
- enqueuing a Matrix send;
- creating a scheduled send;
- sending a prepared upload;
- reporting an accepted submission; or
- writing an encrypted payload.

The DOM retains its current unacknowledged value and the existing Rust draft is
unchanged. The UI may show the existing coarse local-persistence failure
surface. It must not include the target or revision in the error. Automatic
target rebasing is not allowed: rebasing without a quiescence proof would
re-admit an old write. The existing account-local-data reset is the explicit
recovery for a synthetically or corruptly exhausted store.

The `u128` limit is intentionally finite. It provides a strict 39-digit input
bound and an end-to-end checked failure while making natural exhaustion
infeasible.

### Snapshot and IPC changes

Every `draft_revision`, `submitted_revision`, `accepted_revision`, and revision
argument becomes the canonical string type. This includes:

- `ComposerState` and active thread composer projections;
- Rust actions, commands, effects, scheduled-send and upload acceptance paths;
- Tauri command parameters and response DTOs;
- TypeScript `DesktopSnapshot`, backend client, browser fake, app harness, and
  IPC mock;
- checked-in CoreEvent and frontend-state contract artifacts.

Because this is a wire type change, implementation bumps
`SNAPSHOT_SCHEMA_VERSION` from 2 to 3 in both
`apps/desktop/src-tauri/src/dto.rs` and
`apps/desktop/src/domain/types.ts`, then regenerates the checked contract
artifact. A numeric revision at the version 3 WebView boundary is a schema
mismatch, not a value to coerce. Legacy numeric handling exists only inside
the encrypted-store migration described below.

## Target Scope and Isolation

All revision and lease APIs take one structured scope:

```text
ComposerDraftScope {
    account: (homeserver, user_id, device_id),
    target:
        Main(room_id)
        | Thread(room_id, root_event_id)
}
```

The frontend registry must not rely on a delimiter-concatenated target string
or on `reset()` alone for isolation. It uses structured/nested keys or a
canonical tuple helper that includes the full account owner, target kind, room,
and optional root. Main and thread targets in the same room are different;
two roots in the same room are different; the same target identifiers under
two accounts are different.

Tauri, `AppActor`, and `AccountActor` retain #293's complete-owner validation.
A command whose account owner, target, renderer generation, or lease does not
match fails before mutation. Logout, lock, local-data reset, and account switch
invalidate the old renderer generation, drain or reject its admitted commands,
flush the old account's ordered store barrier, and only then allow the new
account scope to become current.

## Lifecycle Lease Model

### What needs a lease

A target is capable of producing or receiving a stale revision-bearing write
from the first moment any of the following exists:

- a local edit waiting for its debounce deadline;
- a draft IPC request waiting for admission or completion;
- a plain/reply/thread submission with a captured draft revision;
- a scheduled-send acceptance with a captured draft revision;
- a prepared-upload acceptance/clear with a captured draft revision;
- a queued or executing Core command that carries a revision; or
- an ordered encrypted-store write/barrier that has not settled.

That whole interval owns a `ComposerTargetLease`. Leases are scoped by account,
target, and renderer/Core producer generation. Lease identifiers and counts
are process-local, opaque, and absent from `AppStateSnapshot`, `Debug`, logs,
and persistence.

### Frontend-to-Core bridge

The frontend cannot prove Rust quiescence using a React `Map` alone. The
backend therefore exposes an acquire/release lifecycle around the active main
composer and the one open thread composer:

1. On target activation, acquire a Core lease and hydrate the authoritative
   revision before assigning a revision to queued local input.
2. Keep the lease while the target is active, has a local overlay or debounce
   timer, or has an unsettled draft/submission/schedule/upload promise.
3. On navigation away, first cancel an empty timer or flush the latest
   non-empty overlay, wait for every operation admitted under the lease to
   settle, then release it.
4. A fresh activation after release obtains a fresh lease and rehydrates the
   current authoritative revision. It never reuses a retired coordinator
   value without observing Rust.

Activation establishes a new lease-generation baseline. The authoritative
revision returned by Rust replaces, rather than `max`-merges with, a retired
frontend entry. Moving backwards is safe only at this boundary: the retired
generation is already incapable of delivering a command. Within one live
lease generation revisions remain strictly monotonic.

Input that occurs while lease acquisition is pending remains DOM-owned and is
coalesced as the latest local value. It is sent only after acquisition
completes. This avoids a window in which a frontend timer exists but Rust
believes the target is quiescent.

The Core command inbox carries a lease permit with every revision-bearing
command. Permit acquisition and command admission are atomic with respect to
Rust collection. The App actor retains the permit through reducer handling and
the ordered store barrier, then releases it. Direct Core/headless callers use
the same admission path; there is no test-only bypass that can send a revision
without a permit.

Renderer reload or crash revokes its generation. Core releases that
generation's long-lived activation leases only after already-admitted command
permits have settled or been rejected. A JavaScript promise that resolves
after revocation cannot apply a snapshot. Account teardown provides the same
barrier for all producer generations under that account.

### Rust target states

For collection purposes each stored target is in exactly one state:

- `Content`: draft content is non-empty. Protected.
- `ActiveEmpty`: content is empty and the main/thread composer is active.
  Protected.
- `LeasedEmpty`: content is empty and at least one producer lease or command
  permit exists. Protected.
- `QuiescentTombstone`: content is empty, target is inactive, and its lease
  count is zero. Collectible.
- `Absent`: neither content nor a revision fence is retained.

The important transitions are:

```text
Absent/Quiescent -> ActiveEmpty/Content: acquire and hydrate
Content/ActiveEmpty -> LeasedEmpty: accepted current submission clears content
LeasedEmpty -> QuiescentTombstone: final lease releases while inactive
QuiescentTombstone -> ActiveEmpty: target reopens before collection
QuiescentTombstone -> Absent: bounded queue evicts oldest eligible target
Any -> Absent: account teardown or authoritative room removal after its barrier
```

An edit that makes content non-empty removes the target from the collectible
queue before applying the edit. A lease acquisition removes it before the
caller can create a delayed operation. A final lease release inserts an empty,
inactive target at the newest end. These mutations and victim selection are
serialized by the Rust owner.

### Eviction proof

The safety argument is:

1. Every producer that can later deliver a revision-bearing operation must own
   the exact account/target/generation lease before it can create a timer or
   enter the Core command inbox.
2. Lease acquisition, command admission, final release, and Rust victim
   selection are serialized. Collection observes either the lease or its
   completed release; there is no state in which the producer exists but the
   target appears unleased.
3. The collector accepts only empty, inactive, zero-lease entries. Therefore it
   cannot remove content, an active composer fence, or a fence named by an
   admitted producer.
4. Assume an old write can arrive after its tombstone was collected. Its
   producer either existed at collection, contradicting steps 1–3, or tries to
   acquire after collection with a retired generation, which is rejected. A
   fresh generation is not the old producer: it first rehydrates the new
   authoritative baseline.
5. Account and target are part of both the lease and lookup key, so a lease for
   another account, main composer, room, or thread root cannot satisfy any of
   the guards above.

Liveness is also explicit: after content becomes empty, the target becomes
inactive, and the last lease/store barrier settles, the owner must enqueue the
tombstone. Continued churn therefore eventually collects it; protected entries
do not become permanent tombstones accidentally.

## Bounded Rust Store

Replace the parallel content and revision maps conceptually with target
entries:

```text
ComposerDraftEntry {
    content: Option<String>,
    revision: ComposerDraftRevision,
    last_accepted_clear_revision: ComposerDraftRevision
}
```

The exact Rust container may remain room/thread maps, but content, current
revision, and accepted-clear token must have one lifecycle owner so they cannot
be truncated independently. `last_accepted_clear_revision <= revision` is an
entry invariant. A normal draft write never changes the clear token.

Maintain two unique oldest-to-newest queues:

- at most 128 quiescent main-room tombstones;
- at most 256 quiescent thread-root tombstones.

Only `QuiescentTombstone` targets appear in these queues. Refreshing a
quiescent target moves it to the newest end. The queue may use a small
`VecDeque` with duplicate removal; it must not use an ever-growing touch-log or
an unchecked serial counter.

When a queue exceeds its quota, pop its oldest target and remove the matching
revision entry only if the target is still empty, inactive, and unleased.
Otherwise remove the stale queue node without removing the entry. Collection
continues until the queue is within quota or no eligible node remains.

The bound is deliberately stated as:

```text
live revision targets
    <= non-empty targets
     + active or leased empty targets
     + quiescent tombstone quota
```

This is the only bound compatible with the requirement not to discard
non-empty content or unsettled work. The 128/256 limits bound retained history,
not user content or active operations. Protected excess is expected to shrink
after its leases settle or its content becomes empty; tests must prove that
transition. A fixed total target cap would require either silent content loss
or refusal of an active edit and is not part of this design.

Room removal may clear its entries only after the same owner/command barrier
proves no admitted operation can still address it. Account teardown clears the
whole current-account store after invalidating its generation. Neither path
uses the LRU as a substitute for lifecycle invalidation.

## Frontend Registry and IME Synchronization

Replace the independent revision coordinator, local draft revision records,
timer records, and per-target clear-epoch records with one
`ComposerDraftLifecycleRegistry`. Each entry owns:

- the structured account/target scope;
- the latest observed canonical revision;
- whether the last authoritative observation had non-empty content;
- optional DOM-owned local overlay;
- optional debounce handle;
- active and lease state;
- unsettled operation count; and
- oldest/newest position only while it is a quiescent frontend tombstone.

Its revision methods accept and return `ComposerDraftRevision`; they use the
shared checked `bigint` helper and never `Math.max`, `+ 1`, or a numeric
fallback.

Frontend collection mirrors Rust:

- never collect an active target;
- never collect a target whose last authoritative observation or local overlay
  is non-empty;
- never collect a target with a timer, IPC request, submission, schedule, or
  upload promise;
- after authoritative acknowledgement, release the redundant local overlay;
- after target release, retain at most 128 main and 256 thread quiescent
  coordinator tombstones, oldest first.

The frontend has the same protected-plus-quota bound as Rust. Promise
completion uses the captured account/target/renderer generation and lease;
after collection or generation revocation it cannot recreate an entry or apply
its snapshot. A local overlay exists only for an active or deactivating leased
target. Quiescent entries never form a React-owned cross-target draft-body map.
If an inactive target was last observed with authoritative content, the
frontend retains only its revision and the coarse content-present bit until the
target is next activated and rehydrated; the body remains Rust-owned.

The two unbounded IME clear-epoch maps are removed. Rust projects
`last_accepted_clear_revision` for the active composer. It changes only when an
accepted operation actually clears the current content; preserving newer input
does not change it. The IME `syncKey` is:

```text
(account scope, target kind, target identity, last_accepted_clear_revision)
```

This gives the CoreEvent-before-command-response ordering from #293 a
Rust-owned semantic reset token without changing the key on ordinary draft
persistence. Only active main and active thread keys are rendered, so no
per-target IME epoch history remains in React.

## Persistence and Migration

### Version 2 encrypted payload

The encrypted payload continues to be account-scoped and written atomically.
Its logical version 2 representation contains:

- non-empty room/thread draft entries with canonical string revisions;
- empty revision tombstones with canonical string revisions;
- `last_accepted_clear_revision`;
- room and thread quiescent-tombstone order, oldest to newest; and
- any empty targets protected at the save boundary, without their lease
  identities; and
- an explicit payload schema version.

Process-local leases, renderer generations, timers, and pending counts are not
serialized.

Persistence receives the lifecycle-compacted store and its protected set. It
must retain non-empty entries and protected empty entries. It retains at most
128/256 additional quiescent tombstones in recorded lifecycle order. Ordered
map keys are a serialization detail, never the victim policy.

The existing per-draft 16 KiB truncation remains. The existing 128/256 target
constants become quiescent-tombstone history quotas rather than hard total
entry caps. A payload may exceed those counts only by its non-empty or
currently protected targets, following the same protected-plus-quota invariant
as live state. Silently truncating a non-empty entry to enforce a hard total
count is not allowed. A future aggregate content-admission policy, if needed,
must be designed separately and must not masquerade as tombstone collection.

After a full restart, formerly protected empty targets become quiescent
together. The loader appends them after the persisted quiescent order and uses
canonical target order only to break that simultaneous-retirement tie. It then
enforces the 128/256 quiescent quotas. This is safe because their old producer
generations no longer exist; it is not a normal runtime victim policy.

### Legacy input

The loader accepts these encrypted legacy shapes:

- draft content with no revision maps: assign revision `"0"` and accepted-clear
  revision `"0"`;
- numeric `u64` room/thread revisions from #293: convert losslessly to their
  canonical decimal strings;
- empty revision tombstones without lifecycle order: treat them as quiescent
  because no process-local producer survives a process restart.

The #293 payload was already limited to 128/256 persisted targets. Its unknown
relative tombstone age is initialized once in canonical target order solely as
a deterministic tie-break among equally old migrated entries. No migrated
non-empty content is removed, and normal post-migration collection uses only
recorded lifecycle order.

Malformed, negative, fractional, over-`u128`, duplicate-order, or
content/order-inconsistent values fail closed as store corruption. They are not
coerced to zero. The next successful mutation writes only version 2; plaintext
is never written during migration. Loading one account cannot inspect or merge
another account's payload.

### Restart semantics

A full process restart has no surviving frontend timer, promise, renderer
generation, Core command permit, or StoreActor write. Persisted empty
tombstones therefore start quiescent. A renderer-only reload against a living
Core is different: its generation-revocation barrier must settle admitted
permits before those targets can become quiescent.

The persisted LRU order is authoritative after restart. Churning one additional
eligible target must evict the same oldest tombstone that would have been
evicted without restart.

## Deterministic Verification Contract

Implementation is test-first. The tests below must first fail against #293 and
then pass without fixed sleeps, log assertions, or private identifiers in
evidence.

### Rust state and lifecycle tests

- Fill the main tombstone queue to 128, touch a middle target, add another, and
  prove the oldest untouched target is removed while the touched target moves
  to newest. Use target names whose lexical order conflicts with lifecycle
  order.
- Repeat independently for 256 thread roots and prove two roots in one room do
  not share a fence.
- Churn beyond both quotas while protecting one non-empty target, one active
  empty target, and one leased empty target. Prove all three survive and the
  only excess equals the protected set.
- Release/close the protected empty targets, churn once more, and prove they
  become eligible and the empty-target excess disappears. The non-empty target
  remains as the only protected entry above the fixed tombstone quota.
- Acquire a lease for revision `N`, accept a clear at `N + 1`, churn beyond the
  quota, then deliver the delayed `N` write. Prove the target was not collected
  and the write is rejected.
- After lease release and collection, reuse of the retired lease/generation is
  rejected. A fresh activation rehydrates the retained revision or starts at
  zero only after the old producer has been made incapable of delivery.
- Prove main, thread-root, room, and complete account scopes cannot advance or
  clear one another.

### Revision and wire tests

- Round-trip `"9007199254740993"` and `u128::MAX` through Rust state, Tauri DTO,
  checked-in JSON contract artifacts, and the TypeScript parser without
  rounding.
- Increment `"9007199254740993"` exactly to `"9007199254740994"` in Rust and
  TypeScript.
- Reject non-canonical strings and all numeric current-schema IPC values.
- At `u128::MAX`, prove draft, plain/reply/thread, scheduled, and prepared-upload
  paths return `composer_revision_exhausted` before state, store, queue, or
  Matrix side effects. The draft remains present.
- Retain command-redaction tests so neither the new revision type nor lease
  type exposes target IDs or draft bodies through `Debug`.

### TypeScript coordinator and IME tests

- Prove both deferred completion orders from #293 with string revisions above
  `Number.MAX_SAFE_INTEGER`.
- Fill and churn both frontend tombstone queues with lexical order opposed to
  lifecycle order.
- Keep active, local-non-empty, timer-pending, IPC-pending, and
  submission-pending targets across churn; settle them and prove later
  collection.
- Switch room and thread while a delayed write exists. Prove release waits for
  cancellation/settlement and that the delayed response cannot recreate a
  retired entry or apply a snapshot.
- Switch account while old promises are unresolved and prove the full owner and
  renderer generation reject every completion.
- Prove ordinary persistence revisions do not change the IME `syncKey`, an
  accepted current clear changes it once, and an accepted operation that
  preserves newer input does not force a reset.
- Assert the old main/thread clear-epoch records no longer exist and registry
  size obeys the protected-plus-quota invariant.

### Encrypted store, restart, and delayed-write tests

- Load an encrypted pre-#293 payload with no revisions, assign zero, mutate it,
  and restart from the version 2 string payload.
- Load an encrypted #293 payload containing numeric `u64` revisions including a
  value above `Number.MAX_SAFE_INTEGER`, convert it exactly, write version 2,
  and restart without changing the value.
- Persist a known lifecycle order, restart, add one tombstone, and prove the
  same oldest eligible target is evicted.
- Hold a deterministic Core command/store barrier around a delayed stale write,
  churn the cache, accept the send, release the barrier, and prove the stale
  write cannot restore content before or after restart.
- Exercise renderer-generation revocation separately from a full process
  restart; only the full restart may assume all process-local producers are
  gone.
- Store two synthetic accounts with the same room/root strings and prove
  migration, collection, reset, and restart of one do not affect the other.

Browser fake, app harness, Tauri IPC mock, and Playwright fixtures use
Rust-shaped string revisions and the same lifecycle rules. They must not keep a
numeric compatibility path that lets the GUI tier pass while production Tauri
rejects the payload.

## Diagnostics and Privacy

Snapshots expose only the active composer's opaque revision and
accepted-clear token. They never expose the full room/thread revision registry,
LRU order, lease identifiers, lease owners, pending target set, or draft map.

Diagnostics and QA evidence may report:

- main/thread tombstone counts;
- protected and evicted counts;
- coarse lifecycle outcomes such as `protected`, `retired`, `evicted`, and
  `revision_exhausted`; and
- migration payload version and success/failure tokens.

They must not report room IDs, thread root/event IDs, user IDs, homeservers,
device IDs, draft/message bodies, revision values, lease values, transaction
IDs, encrypted bytes, filesystem paths, or raw errors. Tests use synthetic
identifiers and assert state/DOM/typed outcomes rather than logs.

`Debug` for the draft store and lifecycle registries reports counts only.

## Canon and Implementation Boundaries

Implementation must update, in the same change:

- `docs/architecture/overview.md` with the protected-plus-quota ownership and
  string wire contract;
- `docs/architecture/state-machine.md` with lease, quiescence, collection, and
  checked-exhaustion guards;
- `docs/policies/engineering-rules.md` with the no-lexical-eviction and
  private-data-free lifecycle rules;
- all Rust/Tauri/TypeScript DTO mirrors and checked-in contract artifacts; and
- browser fake, app harness, IPC mock, and deterministic tests.

Rust/Core owns the lease and victim decision. `App.tsx` may hold the DOM overlay
and render the Rust-projected accepted-clear token, but it must not infer that
a Core target is quiescent or locally delete an authoritative Rust tombstone.

The expected implementation surfaces are:

- `crates/koushi-state/src/state/timeline.rs`,
  `crates/koushi-state/src/action.rs`, and
  `crates/koushi-state/src/reducer/{mod,timeline,thread}.rs`;
- `crates/koushi-core/src/{command,runtime,store,account,timeline}.rs` and the
  focused state/core store, restart, scheduled-send, and timeline tests;
- `apps/desktop/src-tauri/src/{dto,commands/mod,commands/timeline}.rs` and its
  serialization/golden tests;
- `apps/desktop/src/domain/{types,composerDraftRevision}.ts`,
  `apps/desktop/src/App.tsx`,
  `apps/desktop/src/backend/{client,browserFakeApi}.ts`, the app harness, Tauri
  IPC mock, and focused Vitest/Playwright coverage; and
- the checked-in CoreEvent/frontend snapshot artifacts plus the three canon
  documents listed above.

This list identifies the known #293 mirrors; implementation must use the
compiler and contract tests to find any additional numeric revision fixture.

## Acceptance Criteria

- Live Rust and frontend quiescent tombstones never exceed 128 main and 256
  thread targets.
- Non-empty, active, timer-pending, command-pending, submission-pending,
  schedule-pending, upload-pending, and store-pending targets are never
  collected.
- Collection order is lifecycle LRU, with lexical ordering used only as the
  documented one-time tie-break for equally old legacy tombstones.
- A delayed pre-acceptance write cannot restore content during churn,
  navigation, renderer reload, account switch, or restart.
- All current wire revisions are canonical strings and remain exact above
  `Number.MAX_SAFE_INTEGER`.
- Exhaustion fails before product or Matrix side effects and never wraps or
  saturates.
- Legacy encrypted stores migrate losslessly and remain account/target
  isolated.
- Snapshots, logs, `Debug`, fixtures, and QA evidence remain
  private-data-free.
