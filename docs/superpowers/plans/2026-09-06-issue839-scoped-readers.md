# Scoped receipt-reader vertical — implementation worklog

Status: in progress; no final review, PR or completion claim.
Branch: `feat/issue839-scoped-readers`, based on merged #853 (`2132fc98`).

## Approved design

Sol's full vertical re-review: **Correct-to-implement**, after resolving compact
transport, same-owner recovery, explicit anchor responses and focus lifecycle.
The approved [vertical](../specs/2026-09-06-receipt-reader-vertical.md) links the
lifecycle and surface contracts. `docs/architecture/overview.md` was amended before
production changes. Compact output is a scoped Rust model join, NOT embedded in
TimelineItem. All implementation is parent-owned; no implementation subagent.

## Current implementation and evidence

- SDK `ReadReceiptSnapshot::changes_since`: borrowed logical add/update/remove;
  compares timestamp/thread, ignores compatibility slots. Missing-method compile
  RED then GREEN. SDK library **372 passed**, receipt doctests **3 passed**.
  Logs `/tmp/issue839-sdk-changes-{red,green,suite,doc}.log`.
- Core endpoint mirror: retains SDK-position event/receipt endpoints, not message
  bodies. Coalesces first-before/last-after in accepted actor batch processing.
  Seeded from initial SDK items, replaced under the authoritative recovery lease.
  Existing SDK-flow tests migrated without dropping assertions. Initial relay SDK
  vector is released after bounded UTD seeding.
- Single-owner reader order: existing Vector dependency, timestamp descending with
  None/zero ties resolved by user ID. Only changed ordering keys are updated;
  indexed windows exclude own user without dropping retained records. No cloned
  full order vector is published. Both endpoint mirror and ordered index are used
  by the production receipt path.
- Parent review caught no-op endpoint retention: keeping an old logically-equal
  SDK root could lose sharing over later updates. Pointer-sharing regression failed
  then passed after adopting latest endpoints even without a published change.
  Logs `/tmp/issue839-endpoint-retention-{red,green}.log`.
- Latest regression run: **979 passed, 8 existing ignored, 1 explicitly filtered**,
  5.38s test execution (`/tmp/issue839-reader-global-inputs-suite.log`). The filtered
  test is the separately verified outstanding compact-profile RED, not a waived gate.

- Protocol view identities/source reference: scope ID and model revision use
  canonical decimal strings; TimelineViewSource reuses RequestId/TimelineGeneration
  with lossless scoped encodings and privacy-safe Debug. Reused the existing gap-ID
  decimal codec rather than adding another parser. Both missing-type checks went
  RED→GREEN; protocol **30 tests + 3 doctests passed**
  (`/tmp/issue839-view-source-green.log`). These types are not yet a working scope
  subscription or host transport.
- Added the event-qualified ReceiptSourceRef and installed-revision-qualified
  ReaderWindowRequest. Window capacity has one Rust validation path (1–256), shared
  by typed construction and deserialization. Targets explicitly distinguish index
  from installed anchor identity. Request parsing does not claim owner/anchor/ACK
  validation: those checks still belong to the pending registry. Missing-type REDs
  then **31 protocol tests + 8 doctests passed**
  (`/tmp/issue839-window-request-green.log`).

- Implemented indexed anchor resolution against at most 256 installed identities:
  same reader, first surviving installed successor, nearest installed predecessor,
  then explicit NoSurvivingInstalledRow. A non-installed anchor or oversized
  installed set is rejected; newly arrived readers never become fallback anchors.
  Test covers reordered successors, own-excluded ranks, predecessor preference,
  empty survival and invalid membership. Compile RED then GREEN, with 31 protocol
  tests and 9 doctests passing. This helper is not yet wired to an owned window
  subscription; UI positioning and end-to-end anchor acceptance remain outstanding.

- Added concrete protocol ReaderRow, ReceiptCompactSummary, ReaderWindow,
  ViewModel, ViewRetirement and ViewDelivery. Loading/retirement cannot be confused
  with a zero-reader ready window. Source/window/dependency revisions and embedded
  avatar request IDs round-trip losslessly; the existing Rust portable avatar type
  is reused via scoped serde encoding. Debug omits private row/source information.
  Missing-type RED then **32 protocol tests + 15 doctests passed**
  (`/tmp/issue839-view-model-regressions.log`). Output shape/byte enforcement,
  publication/retirement scheduling and host decoders are not implemented by DTOs.

- Added the common private admission ledger: 64 scope slots and 256 MiB of charged
  data, with RAII reservations. Slot/control-byte admission is atomic; failed resize
  preserves the old charge, and transferred/shared artifacts retain their charge
  until final release. Missing-type RED then GREEN
  (`/tmp/issue839-view-budget-{red,green}.log`). This is not heap measurement or
  active publisher enforcement yet: registry artifacts must retain the permits,
  model shape/per-model limits and builder/raw/installed accounting still need wiring.

- Added the private owned-consumer/control registry kernel using those permits.
  Process-wide IDs are checked and never reused across registry instances. A scope
  retains its original consumer/connection through an actual spawned-task transfer;
  independent consumers are not retired together. Source retirement holds its scope
  slot until authorized retirement ACK, while partition retirement/shutdown clear
  their records. Retained control handles keep their byte reservations. Drop removes
  an abandoned scope. RED then GREEN: `/tmp/issue839-scope-control-{red,green}.log`.
  This kernel has no model mailbox yet, and is not connected to CoreConnection or
  source/session admission. Its retirement wake test is not a model-ACK-backpressure
  or whole-host lifecycle acceptance claim.

- Added structural model admission checks for exact compact count/overflow shape,
  matching timeline sources, duplicate events/readers, bounded window/event counts,
  valid numeric ranges and an anchor actually contained in the delivered window.
  RED→GREEN `/tmp/issue839-view-shape-{red,green}.log`; tests retain the 1,500 total
  while checking compact size, 256/257 window boundaries and invalid offsets.
  This check is not yet connected to the model mailbox and does not substitute for
  source/owner/revision validation or encoded-byte admission.

- Connected structural validation and capped encoded-payload counting to retained
  PreparedModel reservations (64 MiB/model). Installed event/user/ready-source-ref
  metadata has its own reservation; ACK retains that metadata, not an entire old
  model. Builder/raw reservations and actual avatar resource leases remain pending.
- Connected latest-only models and one in-flight model to the control registry.
  Retirement clears mailbox/installed data and bypasses ACK waits. Removed the
  separate retirement receiver; task-transfer coverage now uses next_delivery.
  Model coalescing/ACK/retirement test: RED then GREEN
  (`/tmp/issue839-mailbox-{red,green}.log`). Parent found an unissued old revision
  incorrectly accepted by the broad older-ACK check; exact-installed equality
  replaced it after a runtime RED (`/tmp/issue839-unissued-ack-{red,green}.log`).
  This remains an internal registry: no CoreConnection/AppActor/host consumer yet.

- Connected bounded raw receipt reads through the existing CoreConnection command
  lane → AppActor → AccountActor → TimelineManager → TimelineActor, with a oneshot
  reply. No additional request broker or AppState snapshot read is introduced.
  Actor validates the observed timeline key/request/generation, reads its own index,
  excludes own user, and returns exact total plus at most the validated window limit.
  Missing/stale sources do not become empty ready models. RED→GREEN tests cover
  generation/request/Room-vs-Thread mismatch and the command-lane request/reply shape
  (`/tmp/issue839-actor-window-{red,green,final}.log`,
  `/tmp/issue839-window-route-{red,green}.log`). These are boundary tests, not a real
  SDK-to-host subscription test. Profile preparation/finalization, source revision
  fencing, ongoing invalidations and UI consumption are still pending. The raw-read
  method is crate-private and must not become a raw product-data host API.

- Actor window reads now prepare only their selected readers' SDK profiles, reusing
  the same local-profile lookup helper and diagnostic as the legacy receipt path.
  Profiles travel in the bounded raw reply, not a second cache or a truncated legacy
  receipt-map action. The reply reacquires the private actor-generation lease after
  preparation, rejecting replacement before handoff. Missing-field/helper RED then
  GREEN: 1,500 total with three selected rows requests exactly three profiles
  (`/tmp/issue839-scoped-profiles-{red,green}.log`). This uses the real SDK cached
  lookup in a bounded-window fixture, not the complete compact/UI path. The original
  architectural RED remains unchanged and unresolved until that path cuts over.

- Existing Core consumer integration remains intact after the internal command-lane
  addition: `cargo test -p koushi-core-testkit --test request_outcome` passed all
  **10 tests** (`/tmp/issue839-request-outcome-regression.log`), including lag,
  generation fencing and final deadline/disconnect snapshot checks. This does not
  test the new reader subscription end to end.

- Connected the bounded reply back through the same command lane to AppActor for
  await-free enrichment from borrowed current ProfileState. Existing receipt
  enrichment was exported/reused, not duplicated. SDK profiles remain bounded
  hints; no second profile database or partial legacy receipt-map action is made.
  Session/account checks reject inactive context; full private source/revision
  validation still belongs to the pending scoped publication integration.
  CoreConnection's private read path now returns after this AppActor step. A real
  AppActor fixture proves a current alias wins over captured text, total 1,500 is
  retained, and no global snapshot is published
  (`/tmp/issue839-app-actor-profile-resolution.log`). Existing live-signal tests
  (4) and the new public enrichment doctest pass. Two compile-only deadline hits
  were triaged with no surviving processes and completed separately; test execution
  was not timed out. Row time/initials formatting and public view/UI delivery remain.

- Reused oneshot cancellation: AppActor drops abandoned read/resolve requests before
  forwarding/enrichment, and TimelineActor checks again before index/profile work.
  An actual AppActor fixture first failed because a cancelled request reached the
  account lane, then passed with the existing-channel guards
  (`/tmp/issue839-cancelled-window-{red,green}.log`). This does not claim cancellation
  of an already-running SDK lookup or avatar HTTP requests.

- Raw replies now carry the existing private actor-generation gate/key/generation
  witness. AppActor reacquires that existing lease around its await-free enrichment;
  an unbound or replaced actor result is rejected. No lease is held across the
  command-channel awaits, so a queued result does not prevent actor retirement.
  Added a gate-invalidated-after-read RED→GREEN test
  (`/tmp/issue839-source-handoff-{red,green}.log`). This guards actor replacement,
  not yet same-owner receipt/index changes or final scoped publication revisions.

- Fixed stale SDK profile hints resurrecting an avatar removed in current state.
  The actual AppActor fixture first failed with the captured old MXC restored;
  current known room/global/own profiles now suppress captured hints, including
  absent fields. Current alias and original global label are retained. Existing
  state enrichment policy is unchanged; only scoped raw hint admission changed.
  Evidence: `/tmp/issue839-stale-sdk-hints-{red,green,regressions}.log`.

- Each receipt index now owns a small lifetime token; raw windows retain only its
  Weak reference. Logical receipt changes (including None→zero timestamps), index
  removal and authoritative replacement invalidate prepared windows; logical no-ops
  retain validity. No SDK data or ordering vector is shared through this witness.
  AppActor checks it with the actor gate and again after profile projection. Index
  RED→GREEN and an actual AppActor rejection check pass
  (`/tmp/issue839-window-epoch-{red,green,regressions}.log`). These are preparation
  fences, not an atomic cross-actor commit or a substitute for installed revisions
  and live scoped invalidation, which remain pending.

- Verified present events with zero readers are **not** missing sources: actual
  endpoint_from_sdk retains their event identity and empty snapshot. Extended the
  existing window boundary test through last-reader removal (valid empty window)
  and event removal (None), including expiration of both prior windows. It passes
  (`/tmp/issue839-empty-reader-source.log`); no production fix was needed. Earlier
  concern about omission of initially empty endpoints was inaccurate for this code.
- Adopted the [formatting amendment](../specs/2026-09-06-receipt-formatting-amendment.md)
  after Sol's **Correct-to-implement** recheck of two concrete wire/locale findings.
  Rust owns bounded timestamp data and final en/ja policy; adapters use existing
  native medium-date/short-time formatting. Updated the authoritative surface and
  vertical before code. The unapproved formatter-port/ICU/timezone investigation
  is [superseded](../specs/2026-09-06-receipt-row-formatting.md); no new product
  dependency, port or observer was introduced, and isolated build outputs are gone.
- Implemented the protocol timestamp type and migrated ReaderRow's field. Boundary
  RED→GREEN covers lossless decimal strings, the native Date range, null/invalid
  SDK input, malformed wire, and Rust pseudo→en mapping. Protocol **33 tests +18
  doctests** pass; Core **960 pass/8 existing ignored/1 known compact RED filtered**,
  5.39s. Logs: `/tmp/issue839-timestamp-{contract-red,contract-green,docs,protocol-suite,core-suite}.log`.
  Adapter decoding/formatting and actual ReaderRow producer/consumer remain to wire.

- AppActor now consumes enriched raw rows into private ResolvedReceiptWindow with
  actual protocol ReaderRows. SDK profile hints and raw MXCs are not returned;
  current Rust catalog policy produces typed timestamps, and bounded initials use
  the existing ASCII rule with valid Unicode-scalar fallback. No raw/finished row
  copies are retained together. Actor and index witnesses remain private and the
  final epoch check follows conversion. Actual AppActor RED→GREEN verifies current
  alias, original label, removed-avatar suppression, ja timestamp policy, initials,
  total 1,500 and no global snapshot; CJK/emoji/RTL initials are covered.
  Evidence: `/tmp/issue839-reader-rows-{red,green,final}.log`; shared catalog resolver
  doctest and domain dependency gates pass. This is not a published ReaderWindow:
  future scope finalization must bind avatar resource leases **before** raw MXC
  disposal and supply installed/window revisions, rather than expose this private
  preparation route as a host API.
- Verification triage: compile58.56s consumed the combined60s limit. Direct binary
  execution then omitted Cargo's RUST_MIN_STACK=4194304 and aborted; the exact
  affected test passed with that setting, followed by the full Cargo invocation
  above. No surviving test process or repository core dump was found. Failed and
  timed-out invocations are not passing evidence; shared targets were retained.

- CoreRuntime now shares its actual scope registry with attached CoreConnections;
  each private view_consumer call creates an independent existing logical context,
  bound to the connection's real identity. Dropping the CoreConnection does not
  retire transferred scopes (as required by the approved lifecycle). A runtime-task
  lifetime guard shuts down the registry on completion/cancellation, including
  cancellation before first poll. An actual file-credential runtime test covers
  two contexts on one connection, another connection, selective retirement,
  connection Drop, runtime Drop and rejected post-shutdown creation. RED→GREEN:
  `/tmp/issue839-connection-ownership-{red,green}.log`. Compilation was separated
  (37.07s) from the passing execution above. This is internal lifecycle integration,
  not a public facade or source-admitted ReaderReady delivery yet.

- Resource-path reconnaissance found an unnecessary full byte clone in existing
  renderable-cache insertion: the copied entry was returned and discarded by its
  caller. Changed that private insertion to move bytes and return unit. A retained
  allocation-identity check failed then passed, while byte accounting and lookup
  content remain checked (`/tmp/issue839-thumbnail-move-{red,green}.log`). All 10
  cache tests and the Core suite above pass. This is a copy removal, not a lease,
  request-cancellation or network-demand acceptance claim. Existing account avatar
  single-flight/session fencing can be reused, but it has no scoped cancellation
  and its waiter/cache metadata is not yet the final bounded demand owner.

- Bound the private reader preparation request to its logical ViewConsumer.
  Admission verifies origin connection, registry identity (connection numbers can
  repeat across runtimes), consumer liveness and runtime liveness. Rechecks after
  raw preparation and final resolution discard retired-consumer results. Extended
  the existing command-lane test: foreign-runtime/same-number and retired contexts
  enqueue nothing; valid preparation preserves total/start; retirement before final
  handoff yields None. RED→GREEN and suite logs:
  `/tmp/issue839-reader-consumer-{red,green,suite}.log`. This is admission/handoff
  fencing, not cancellation of already-running SDK/HTTP work or a public scope API.

- Reader preparation now stops waiting on logical-consumer retirement or runtime
  shutdown, even when the SDK reply sender remains alive. Existing registry/context
  control uses latest-only Notify wakeups, not another data queue. The private
  connection request races that control against its two-stage work and drops its
  oneshot receiver on cancellation. An actual command-boundary RED timed out with
  a held raw reply; GREEN proves both consumer and runtime retirement return None
  and close that reply receiver. Logs:
  `/tmp/issue839-reader-retirement-{red,green,suite}.log`. This does not claim that
  already-running SDK storage or avatar HTTP work has itself been cancelled.

- TimelineActor's existing profile preparation now selects its oneshot sender's
  closed notification against the SDK lookup future, returning immediately when
  the requester is gone. No new task runner or cancellation broker was introduced.
  The real-SDK selected-profile test still records exactly 3 lookups for 1,500 total
  readers, and additionally proves an already-closed reply starts no lookup.
  RED→GREEN: `/tmp/issue839-profile-cancel-{red,green}.log`; final suite above.
  Cancellation of the Core wait is wired; cancellation of any SDK-internal storage
  job or actual avatar HTTP request is not established by this test.

- Added the private byte-lease layer after Sol's **Correct-to-implement** verdict
  on [the resource design](../specs/2026-09-06-receipt-resource-leases.md). Existing
  cache entries are Arc-owned; insertion still moves bytes and transport lookup
  still makes its existing one owned copy. A charged lease survives LRU eviction
  without restoring global discoverability. Its clones share one charge, and the
  final clone releases it even after cache clear. Independent acquisitions are
  conservatively capped at 96 MiB alongside the unchanged 32-MiB LRU; admission
  and capture occur under the cache lock, with checked arithmetic and separate
  missing/capacity errors. No uncharged entry Arc is exposed.
  RED→GREEN and all 12 cache tests pass
  (`/tmp/issue839-byte-lease-{red,green}.log`), followed by the Core suite above.
  **Not yet connected** to prepared/installed models or authorized host resource
  reads; this is not public-delivery or actual-network acceptance.

- Connected byte leases to the actual AppActor row-conversion path. Before the
  raw AvatarImage is consumed, ResolvedReceiptWindow retains a private per-reader
  MXC binding and a charged lease (or its explicit unavailable/capacity result).
  Host ReaderRows still contain no MXC. A Ready reference without an acquired
  lease becomes NotRequested in this Rust producer, preserving its private demand
  identity instead of advertising unavailable bytes. Existing terminal/non-ready
  thumbnail states remain unchanged. The production conversion test proves ready
  bytes survive cache clear, missing bytes preserve MXC with Unavailable, and
  exhausted lease admission preserves MXC with Capacity without emitting Ready.
  RED→GREEN: `/tmp/issue839-reader-binding-{red,green}.log`; full suite above.
  These bindings are still private preparation: installed ownership, authenticated
  host reads and visibility-driven scheduling remain required before exposure.

- Reproduced a stale-source handoff after AppActor resolution: while the consumer
  remained live, dropping the receipt epoch before the connection received the
  reply still returned rows. The existing command-lane test now binds a real
  preparation owner/epoch and includes that case, preserving its valid-result and
  consumer/runtime cancellation cases. CoreConnection reacquires the preparation
  generation lease and checks the receipt epoch at handoff; stale results drop.
  RED→GREEN: `/tmp/issue839-source-delivery-{red,green}.log`; full suite above.
  This closes the observed handoff gap, **not** the remaining atomic publication/
  installed-revision race: a future scope publisher must still fence its commit.

- Common model publication now takes the producer's private avatar resources and
  retains them in installed metadata. Their identities/control data are charged to
  the existing view ledger; image bytes retain their separate lease charge. Model
  admission rejects missing, duplicate, unreferenced or mismatched bindings, and
  Ready requires a lease for the exact opaque reference. Existing mailbox ownership
  transfers the resource set through ACK without copying bytes. A producer→registry→
  delivery→ACK test proves installed ownership survives cache clear, retirement
  releases it, and a retained client delivery remains charged until its release.
  It also rejects unbound and wrong-reference Ready models. RED→GREEN:
  `/tmp/issue839-installed-resources-{red,green}.log`; full suite above.
  Public source admission/atomic commit and authorized host reads remain unwired.

- Added private consumer resource authorization against the exact installed
  revision. It checks runtime/consumer/scope liveness and logical ownership under
  the existing locks, then returns a charged lease clone; transport byte copying
  occurs after those locks are released. The existing lifecycle test now rejects
  pre-ACK, foreign-consumer (same connection number), wrong-revision, unknown-ref
  and retired-consumer access, and reads installed bytes after cache clear.
  RED→GREEN: `/tmp/issue839-resource-auth-{red,green}.log`; full suite above.
  This is the private Core authorization boundary, not authenticated host adapter
  wiring, visibility authorization for new downloads, or actual HTTP acceptance.

- Connected installed-resource authorization to a private CoreConnection entry
  point. It validates the actual registry identity, origin connection and consumer
  liveness before delegating to installed-revision authorization. The boundary test
  rejects another runtime's consumer with the same connection number, uninstalled
  revisions and retired consumers. RED→GREEN:
  `/tmp/issue839-resource-port-{red,green}.log`; full suite above. Host identity
  binding/transport remains unwired; this is not public adapter acceptance.

- Sol reviewed the [source commit-fence design](../specs/2026-09-06-receipt-commit-fence.md)
  and returned Correct-to-implement for the private validity-cell fence, requiring
  a prepared-only synchronous commit and source→registry lock order. Split the
  existing publication into preparation and commit_prepared without changing its
  behavior. Existing coalescing/ACK/retirement tests and the Core suite pass
  (`/tmp/issue839-prepared-commit-{green,suite}.log`). The epoch mutex and actual
  fenced commit are **not yet implemented**. Before adding the guard, also ensure
  replacement of an old mailbox payload cannot run large destructors under it;
  serialization being outside the guard alone does not establish timing bounds.

- Removed replaced-payload destruction from the prepared commit. Mailbox publish
  now returns the old Arc; the outer publisher drops it after commit guards leave.
  The new payload Arc is also allocated/owned outside commit and borrowed by it,
  so rejection cannot destroy that payload under the commit locks. A runtime RED
  showed the replaced model was destroyed immediately; the same check now proves
  it survives until the returned replacement is dropped.
  `/tmp/issue839-commit-drop-{red,green,suite}.log`. This is a prerequisite for the
  approved source fence, not evidence that the fence or timing budget is complete.

- Implemented the approved private validity cell: index update invalidates its old
  Mutex<bool> cell before the first logical mutation; Drop invalidates it too.
  True no-ops preserve the cell. Raw/resolved checks read validity rather than
  treating an upgraded old Arc as current. The existing endpoint regression was
  RED while holding upgraded witnesses across update/removal and is now GREEN.
  ResolvedReceiptWindow::commit_if_current holds the actor-generation lease and
  validity cell across a synchronous callback. A controlled-thread test makes
  actual index Drop wait for that callback, then rejects the upgraded old witness.
  `/tmp/issue839-epoch-cell-{red,green,final,suite}.log` (9 focused tests).
  The primitive is verified; actual source/window/dependency admission and its
  wiring around prepared scope publication remain unfinished, not waived.

- Connected the private source fence to prepared mailbox publication:
  publish_current prepares outside the fence, commits inside it, and releases any
  replaced payload afterward. The unguarded entry is now test-only for mailbox
  fixtures. The existing producer/resource/ACK lifecycle test binds a real source
  witness and uses the fenced entry; an invalidated witness cannot publish into a
  separate still-live scope. RED→GREEN:
  `/tmp/issue839-fenced-publication-{red,green}.log`; full suite above. This proves
  the provided source witness fences commit, not complete scope admission: actual
  subscription routing, model/source/window/dependency matching and invalidation
  delivery still require integration before exposing host subscriptions.

- Bound the private reader publisher to the source actually observed by
  TimelineActor. The preparation owner retains the complete ReceiptSourceRef
  (without a duplicate TimelineKey). Before preparing/publication, ReaderReady
  must match that reference and the prepared total/start. An existing lifecycle
  test reproduced successful publication of a different event using a valid
  witness; it now rejects it. `/tmp/issue839-model-source-{red,green,suite}.log`.
  This entry is specifically a single-reader-window publisher; compact multi-event
  publication still needs its own complete admission. Requested-window/dependency
  revisions, subscription routing and host delivery remain unfinished.

- Ran current CI boundary/portability gates: domain dependency and leaf boundaries
  pass. Rust test structure initially failed because receipt_endpoints' inline
  tests exceeded 200 lines; moved the module unchanged into
  `timeline/receipt_endpoints/tests.rs`, retaining names/assertions. Structure now
  passes (`/tmp/issue839-test-structure-green.log`) and all 970 Core tests above
  remain green apart from the explicitly filtered architectural RED.
  Actual CI Wasm command for State/Search/Protocol passes in 14.23s
  (`/tmp/issue839-portability-check.log`); this does not prove Core Wasm support.
  Native non-test Core check first hit its 60s deadline while checking dependencies;
  no surviving cargo/rustc processes remained. Narrow SDK check passed in 55.25s,
  then Core library check passed in 9.46s
  (`/tmp/issue839-{sdk-library-check,core-library-check-final}.log`). Compiler
  warnings remain, including unconnected scoped-publication paths; not a lint or
  full vertical-integration pass. Initial timed-out check is not a pass.

- Next integration investigation: a bounded Flash consultation confirmed that
  publication still lacks actual subscription callers and identified the existing
  AppActor command seam. Parent rejected inaccurate advice about strong-count-only
  fencing, unavailable/Loading semantics and awaiting SDK replies inside AppActor.
  Recorded the concrete remaining flow and open decisions in
  [subscription routing](../specs/2026-09-06-receipt-subscription-routing.md).
  Confirmed existing runtime AbortOnDrop task ownership can be reused; no new
  broker or reset-counter CoreConnection clone is needed. The scheduling proposal
  is **not approved or implemented**: registry-owned raw context, direct mutation
  hooks and ordered dependency/window counters must be specified/reviewed first.
  No tests were rerun for this documentation-only investigation.

- Traced direct mutation hooks for routing: the synchronous post-reducer seam can
  consume existing profile/alias identity effects, but room-local profile changes
  currently lose room/user identities and avatar changes lose the MXC identity.
  Those typed identities are required; empty ProfileChanged must not mean "all".
  Receipt net changes are already available in relay before legacy materialization.
  Updated the routing proposal with exact paths and the remaining ownership races.
- Closed the successful-cache initial-registration case in the actual row producer:
  a NotRequested SDK hint now reuses the existing avatar cache key and acquires a
  charged lease when bytes have already arrived. It constructs Ready metadata from
  that lease without copying bytes or making HTTP requests. Loading/Failed states
  remain unchanged. The existing conversion test was RED despite cached bytes and
  is now GREEN (`/tmp/issue839-cached-hint-{red,green}.log`); full Core suite above
  and Rust test structure pass. This does not substitute for subsequent typed MXC
  invalidations or actual demand/network acceptance.

- Expanded verification beyond Core's internal tests on the current worktree:
  **Core testkit 225 passed**, no failures/ignored/filtered tests
  (`/tmp/issue839-current-testkit-{compile,suite}.log`). **State 786 tests plus
  3 doctests passed**, no failures/ignored/filtered tests
  (`/tmp/issue839-current-state-{compile,suite}.log`). Compilation and execution
  were invoked separately with bounded deadlines. These cover existing external
  runtime and State contracts; they do not cover a newly wired subscription/UI,
  actual avatar HTTP demand, or resolve the outstanding compact architectural RED.

- Completed the routing-mechanics design gate in targeted Sol rounds. Ownership,
  registration and counters passed initially; fixed the dirty FIFO phase ambiguity
  with atomic Idle/Queued/Running transitions through completion handling. Initial
  capacity failure is immediate, not an indefinite wait. The remaining conflict
  with keep-last-installed was explicitly resolved by the
  [capacity amendment](../specs/2026-09-06-scoped-capacity-amendment.md), reviewed
  **Correct-to-adopt** and adopted into lifecycle canon before code. Normal
  coalescing preserves the installed model; hard Capacity, CounterExhausted and
  ProducerFailed are explicit derived-scope terminal exceptions with no auto-reopen
  or durable-state eviction. Host localized clearing remains an enablement gate.
- Added those three closed protocol retirement reasons. Extended the existing
  ACK/backpressure test across the old SourceUnavailable case and all three new
  reasons, preserving its assertions; terminal delivery still bypasses unacked
  models. RED→GREEN: `/tmp/issue839-capacity-reasons-{red,green}.log`.
  Protocol **33 tests +18 doctests**, Core suite above, Rust test structure and
  State/Search/Protocol Wasm check pass (`...-protocol.log`, `...-wasm.log`).
  Routing contexts/tasks and authoritative Failed/Loading resource reads are not
  implemented merely because their design mechanics/reason vocabulary are ready.

- Added the reviewed receipt source revision to the existing validity cell, not
  another ordering owner. Actor generation is an owner-replacement fence and cannot
  represent receipt changes. A checked process-wide counter stamps new logical
  epochs, including replacement; no-ops retain their stamp. Exhaustion yields no
  revision and an invalid cell, without wrapping or panicking or removing legacy
  receipt data. Resolved windows expose the stamp and the fenced publisher rejects
  mismatched ReaderReady source_revision; absent minted stamps report CounterExhausted
  at that private publication boundary. Tests cover no-op/change/replacement,
  allocator exhaustion and wrong-revision publication.
  `/tmp/issue839-source-revisions-{red,green,compile,suite}.log`.
  Existing optional raw-read replies still need the routing's typed failure
  propagation; this does not establish end-to-end exhaustion/retirement behavior.
  Source/window/dependency context ownership and actual subscription callers remain
  the next integration work. Test structure, formatting and diff checks pass.

- Replaced private read/resolve Option replies with typed results throughout the
  existing connection→AppActor→AccountActor→TimelineActor route. CounterExhausted
  is rejected before SDK profile preparation and preserved to the caller;
  InactiveSession, SourceUnavailable, Closed and NotOwned remain distinct causes.
  AppActor's post-profile check now reads validity through the generation/epoch
  guard rather than the remaining strong-count shortcut. Existing tests preserve
  valid totals/offsets and cancellation assertions, add exact counter-error
  propagation, and prove inactive sessions enqueue no AccountActor preparation.
  `/tmp/issue839-typed-preparation-{red,green,compile,suite}.log`; Core suite above,
  formatting, test structure and diff checks pass. This supports the reviewed
  subscription failure contract; actual scope context/producer routing remains
  unimplemented and is not replaced by this private one-shot read API.

- Connected the existing runtime AbortOnDrop wrapper to common scope Control.
  Each scope admits at most one owned producer; retirement detaches/aborts its
  handle outside retirement/producer locks. A weak completion guard moves with the
  eventual envelope and is disarmed only after handling, not on send. Dropping an
  unhandled guard retires ProducerFailed; it cannot keep the scope alive or
  overwrite an earlier retirement. New tests exercise explicit retirement,
  rejection of concurrent producers, task captures dropped before first poll,
  completion-held running lifetime and unexpected exit. A conservative reservation
  retained by an artifact remains charged after task abort until its last external
  owner drops. `/tmp/issue839-owned-producer-{red,green,suite}.log`; three focused
  tests and Core suite above pass, as do formatting/structure/diff checks.
  This is actual common lifecycle ownership, but spawn_producer currently has
  only test callers: SDK raw work, context/FIFO scheduling and AppActor completion
  envelopes still require integration. Synthetic pending-task cancellation is
  not HTTP cancellation evidence. Account avatar cache/inflight remain the existing
  unbounded maps; no readiness/demand migration is claimed by this slice.

- Fixed a scope-disposal bug before connecting weak context observers: removing
  the registry entry alone did not abort its producer if a temporarily upgraded
  observer kept Control allocated. Extended the pre-first-poll test to hold that
  observer; it failed on the still-present producer. OwnedViewScope::drop now
  removes admission, releases the registry lock, and explicitly retires Control
  before dropping the entry. The new ScopeClosed reason distinguishes this from
  consumer retirement; the test verifies the consumer can still open another scope.
  `/tmp/issue839-scope-disposal-{red,green,suite,protocol}.log`: Core suite above,
  Protocol 33 tests +18 doctests, formatting/structure/diff checks pass.
  This implements the existing synchronous close/Drop contract, not a new
  subscription or a proof of actual network cancellation.

- Added registry-Control-owned initial reader request admission (source, target,
  limit, initial desired sequence zero and dependency revision one). The private
  open_reader operation accounts retained source data with the existing capped
  serialization counter; retirement clears it independently of held Control
  observers. ReaderReady commit now requires this registration and verifies source,
  clamped target, exact bounded row count and both request/dependency stamps under
  the commit locks. No unregistered production ReaderReady fallback remains.
  Existing resource/publication tests now register the request and additionally
  reject unregistered scopes, a different requested target, and mismatched window
  and dependency revisions while preserving source/lease checks.
  `/tmp/issue839-reader-admission-{red,green,compile,suite}.log`; Core suite above,
  formatting/test structure/diff checks pass. This is initial request admission,
  not live subscription: dynamic window changes, dependency increments, source
  high-water/owner retirement, accepted raw ownership, FIFO and AppActor callers
  still need integration before host enablement.

- Added the reviewed registry-locked Idle/Queued/Running reader FIFO. Initial
  admission queues source work; source and dependency dirtiness coalesce while
  queued/running, dependency changes advance their checked stamp, and completion
  requeues at the tail only when newer dirtiness remains. Popping reserves the
  conservative 64-MiB builder allowance before cloning work inputs. Capacity and
  counter failure retire outside the registry lock rather than waiting/spinning.
  Work owns a weak failure guard and checked run identity: dropping unhandled
  running work retires ProducerFailed; an old work token cannot complete or retire
  a later run. Closing/consumer retirement removes queued IDs, preventing churn.
  Three tests cover ordering/coalescing/stale completion, terminal admission and
  abandonment, and 1,000 open/drop cycles without queued-ID accumulation.
  `/tmp/issue839-reader-queue-{red,green,suite}.log`; the first focused invocation
  exposed an invalid new JSON fixture (null RequestId), corrected to the actual
  protocol shape before GREEN. Core suite above and structure/diff checks pass.
  The queue is now part of open_reader, but AppActor does not consume it yet.
  Actual raw-slot ownership, producer/completion integration and direct indexed
  invalidation inputs remain next; this is not live subscription acceptance or
  network/performance evidence. AppActor must finish producer ownership before
  completing/requeuing a work token in the same completion-handling turn.

- Added charged accepted raw ownership to ReaderRequest. Measured receipt/profile
  and source data transfers out of the admitted builder reservation with no
  release/reacquire gap; work metadata remains reserved and oversized transfer
  fails without truncation. Dependency-only work clones the accepted Arc, not its
  payload; source-dirty work does not reuse it. Raw input can be cloned for current
  profile projection without mutating the retained SDK hints. Acceptance verifies
  live work/window identity and prevents source revision regression; old accepted
  payload drops outside registry locks. Tests cover original-hint preservation,
  exact Arc reuse, narrower-window rejection, transfer under budget pressure and
  scope close with an in-flight last owner.
  `/tmp/issue839-owned-raw-{red,green,compile,suite}.log`; Core suite above and
  formatting/structure/diff checks pass. This supplies the owned input needed by
  AppActor routing but does not connect AppActor yet. Completion must integrate
  raw acceptance with fenced model admission, recheck stale cached source before
  reuse, retain actor-owner replacement fencing, and reserve projection work before
  cloning. Direct invalidation, host readiness, UI/HTTP/performance remain pending.

- Connected the common registry to AppActor and its actual select loop. One
  queued reader job per turn spawns through the existing scope-owned task wrapper;
  SDK read and completion both use the existing bounded command lane. AppActor
  stores only a weak command sender, preserving command-channel lifetime behavior.
  Workers capture no consumer/CoreConnection/strong registry. Fresh raw and cached
  raw inputs reach an await-free profile/locale projection and fenced publication;
  measured raw is accepted before the same turn completes/requeues work. Projection
  scratch is admitted before cloning; cached-source validity is checked before
  reuse and again on finalization. Typed terminal failures retire; superseded work
  preserves pending dirtiness rather than silently becoming idle.
  The existing AppActor alias fixture now drives real internal read/completion
  envelopes: initial model has total 1500 and current alias, ACK then a manually
  signalled dependency update produces the new alias without another AccountActor
  SDK request or global state generation increment. It retains all previous
  one-shot assertions. `/tmp/issue839-app-reader-routing-{red,green,suite}.log`;
  Core suite above and formatting/structure/diff checks pass.
  **This is now a production queue/publication caller**, not only lifecycle test
  scaffolding. It is not host enablement: direct indexed source/profile/MXC/locale
  mutation hooks, initial subscription/host admission, actor-owner retirement,
  authoritative hint readiness, visibility demand, UI and HTTP/performance evidence
  remain pending. The fixture signals dependency dirtiness explicitly and uses a
  controlled AccountActor reply; it is not actual HTTP or full application evidence.

- Verified automatic queue consumption with a real file-credential CoreRuntime,
  without manually pumping AppActor. An inactive-session reader settles as
  SessionRetired within the bounded wait even after its originating connection
  drops; its logical consumer remains usable and retirement ACK succeeds.
  `/tmp/issue839-reader-runtime-loop.log` (0.08s execution) and the Core suite above
  pass, as do formatting/structure/diff checks. This adds runtime-loop/lifetime
  coverage to the controlled successful-publication fixture; it is not a real
  homeserver or HTTP-demand test and does not establish active-session acceptance.

- Fixed a reproducible private-owner rebind: previously a new actor with the same
  public ReceiptSourceRef could replace accepted raw. ChargedRaw now records the
  private actor generation; AppActor checks it before projection/publication and
  raw acceptance checks it again. SourceRetired is distinct from a stale result,
  so pending dirtiness cannot turn a permanent owner replacement into retries.
  The AppActor fixture now proves replacement retires SourceUnavailable even with
  unacked model data and more dirtiness. `/tmp/issue839-reader-owner-{red,green}.log`
  and latest `...-final-{compile,suite}.log` cover this.
- Also corrected stale source classification: expired source witnesses now return
  SourceUnavailable rather than Closed, preserving the existing superseded-work
  retry branch. `/tmp/issue839-reader-stale-source-red.log` contains the expected
  assertion plus shared-cache-lock poison follow-on failures; the fresh full suite
  above passes after the fix. Formatting/structure/diff checks pass.
  These are refresh-time fences, not immediate source/actor invalidation hooks.
  Registration-time ownership, direct notifications, host/UI/HTTP/performance and
  the remaining #840/#846 work are still required.

- Connected session and locale mutations directly at the synchronous
  reduce_app_action_state boundary, before deferred effects. A changed existing
  SessionKeyId retires current Matrix view scopes as SessionRetired while keeping
  host consumers alive. Receipt locale comparison reuses the row policy via
  ReceiptTimestampLocale::from(CatalogLocale), so En/Pseudo share En and unrelated
  regional tags do not invalidate the same resolved locale. A true change dirties
  only the bounded reader registrations; it does not scan profile caches.
  The AppActor fixture failed before each hook: logout left its scope live, and
  locale mutation scheduled no projection. It now proves synchronous retirement,
  automatic cached-input Ja→En projection without another AccountActor request,
  and no work for En→En region changes. Existing assertions remain.
  `/tmp/issue839-reader-session-{red,green}.log`,
  `/tmp/issue839-reader-locale-red.log`,
  `/tmp/issue839-reader-global-inputs-{green,suite,protocol,wasm}.log`:
  Core suite above, Protocol **33 tests +19 doctests**, State/Search/Protocol Wasm
  (3.49s), formatting/structure/diff checks pass. This is not Core Wasm or HTTP
  evidence. Per-source and indexed user/room/MXC hooks, host admission/readiness,
  visible avatar demand, UI and remaining #840/#846 acceptance still remain.

The room-local profile hook is now implemented as a concrete continuation slice:
accepted reader dependencies retain a room+user index, `LiveRoomProfilesObserved`
compares only the affected identities at the reducer boundary, and only matching
scopes are dirtied for dependency-only reprojection. Registry coverage proves
room/user selectivity (`/tmp/umbrella-room-profile-registry-green.log`), and the
real AppActor receipt fixture proves the room-profile projection runs without a
new AccountActor SDK request (`/tmp/umbrella-room-profile-runtime-green-4.log`).
The complete workspace rerun after this change passes 2,582 tests / 12 ignored
across 132 suites with no failures (`/tmp/umbrella-final-workspace-room-profile.log`).
The next identity slice adds a private MXC-to-reader index from accepted raw
profile/avatar bindings and a reducer hook for successful `AvatarThumbnailUpdated`
state mutations; it dirties only scopes that retain that exact source reference.
Room/user/MXC registry coverage is green (6 reader tests,
`/tmp/umbrella-avatar-index-readers-suite-3.log`), and the existing AppActor
receipt route remains green (`/tmp/umbrella-avatar-index-runtime-green.log`).
A further publication slice now splits receipt-only live-signal changes by
room/event: `live_signals_receipts_by_room_event` replaces the prior full
`RoomLiveSignals` clone when ordering/typing/fully-read metadata is unchanged,
while room additions/removals and non-receipt metadata retain the explicit full
room fallback. Core nested-delta coverage passes and the appStore applies/removes
only affected events while preserving unrelated room identity
(`/tmp/umbrella-live-receipt-event-core-2.log`,
`/tmp/umbrella-live-receipt-event-vitest.log`). The subsequent workspace rerun
passes 2,583 tests / 12 ignored across 132 suites with no failures, and desktop
typecheck/lint (including IME, agents-doc, and semantic-owner checks) pass
(`/tmp/umbrella-final-workspace-live-receipt-event-2.log`,
`/tmp/umbrella-live-receipt-event-typecheck.log`,
`/tmp/umbrella-live-receipt-event-lint.log`). The receipt-only delta was then
threaded through the Tauri frontend DTO boundary; its serialization test passes
(`/tmp/umbrella-live-delta-tauri-2.log`). The full Tauri library suite also
passes 134/134 after the DTO change
(`/tmp/umbrella-final-tauri-live-receipt-event.log`). A deterministic ignored reference
measurement now seeds 100 rooms and 10,990 receipt events, including 1,500
active-room readers, then moves one receipt between two existing events. Debug
line-table execution records 12.14 ms setup, 16.85 ms delta-build wall time, a
1,773-byte encoded delta, and exactly two touched events. The scoped payload
invariants pass, but the measured build slice exceeds the proposed 8 ms CPU
budget and does not yet provide retained-heap, HTTP-request, browser-task, or
p95 evidence (`/tmp/umbrella-publication-perf-measurement.log`). The measurement
now repeats the build 25 times: debug line-table first/p95 are 13.78/16.62 ms,
while an explicit single-Cargo-job optimized rerun (`CARGO_BUILD_JOBS=1`, test
threads=1) records 4.97 ms setup, 3.35/4.39 ms first/p95 build time, 1,813
encoded bytes, and two touched events. The optimized comparison is below the
8 ms CPU target; debug remains above it and no retained-heap, HTTP-request, or
browser-task measurement exists (`/tmp/umbrella-publication-perf-p95-debug.log`,
`/tmp/umbrella-publication-perf-p95-release.log`). A direct single-test binary
run with `RUST_MIN_STACK=4194304`, `--test-threads=1`, and `/usr/bin/time -v`
records 0.37 s wall time and 39,476 KiB maximum RSS; this is process-level
measurement including the test binary, not a retained projection-heap
attribution (`/tmp/umbrella-publication-perf-direct-memory.log`). Final workspace and frontend reruns
remain green: 2,584 Rust tests / 12 ignored across 132 suites and 1,271 Vitest
tests across 107 files (`/tmp/umbrella-final-workspace-live-receipt-event-3.log`,
`/tmp/umbrella-final-vitest-live-receipt-event.log`). The final workspace rerun
including the new ignored performance characterization remains green at 2,584
passed / 13 ignored across 132 suites
(`/tmp/umbrella-final-workspace-receipt-event-and-perf.log`). Final Rust
format, diff, and desktop lint/IME/docs/semantic-owner checks also pass
(`/tmp/umbrella-final-fmt-receipt-event.log`,
`/tmp/umbrella-final-diff-receipt-event.log`,
`/tmp/umbrella-final-lint-receipt-event.log`). A follow-up split moves
non-receipt room metadata (`fully_read_event_id`, typing IDs/users) into
`live_signals_room_metadata_by_id`, so typing/read-marker changes also avoid
cloning a room's receipt map. Core and Tauri boundary tests pass
(`/tmp/umbrella-live-metadata-core-2.log`,
`/tmp/umbrella-live-metadata-tauri-3.log`), and the appStore suite is 37/37
(`/tmp/umbrella-live-metadata-vitest.log`). The post-metadata workspace run
passes 2,586 tests / 13 ignored across 132 suites with no failures
(`/tmp/umbrella-final-workspace-live-metadata.log`). Desktop typecheck and the
focused appStore suite remain green after the metadata contract
(`/tmp/umbrella-final-typecheck-live-metadata.log`,
`/tmp/umbrella-final-vitest-live-metadata.log`). The full post-metadata frontend
suite passes 1,272 tests across 107 files
(`/tmp/umbrella-final-vitest-live-metadata-full.log`). The production frontend
build remains green (with the existing dynamic-import/chunk-size warnings)
(`/tmp/umbrella-final-build-live-metadata.log`). A fresh `test:ui-headless` attempt with the documented 180-second outer
limit stopped after 141 passing cases; its orphaned Vite child was inspected
and terminated, so that attempt is retained only as a timeout. After triage,
the complete current-tree lane was rerun with an explicit 600-second acceptance
budget and passed all 289 cases in 6.2 minutes, with no surviving Vite or
Playwright processes (`/tmp/umbrella-final-browser-live-metadata-full.log`).
Final Rust format, diff, and test-structure checks also pass
(`/tmp/umbrella-final-fmt-current.log`,
`/tmp/umbrella-final-diff-current.log`,
`/tmp/umbrella-final-structure-current.log`,
`/tmp/umbrella-final-lint-current.log`). Final Tauri adapter,
domain/leaf boundaries, agents-doc, and State/Search/Protocol Wasm checks pass
(`/tmp/umbrella-final-tauri-boundary-live-metadata.log`,
`/tmp/umbrella-final-domain-boundaries-live-metadata.log`,
`/tmp/umbrella-final-agents-live-metadata.log`,
`/tmp/umbrella-final-wasm-live-metadata.log`). Current dependency checks also
show `cargo deny` and `cargo machete` green, while `cargo audit` still exits 1
for inherited quick-xml vulnerabilities RUSTSEC-2026-0194/0195 (the current
scan also reports an allowed lru unsoundness warning and yanked chacha20)
(`/tmp/umbrella-final-cargo-deny-live-metadata.log`,
`/tmp/umbrella-final-cargo-machete-live-metadata.log`,
`/tmp/umbrella-final-cargo-audit-live-metadata.log`). A dry-run
`cargo update -p quick-xml --dry-run` locks zero packages and leaves the lockfile
unchanged, so no compatible advisory fix is available through a speculative
lockfile update (`/tmp/umbrella-quick-xml-update-dry-run.log`). A fresh real Tuwunel
Core live-signals lane also passes receipt, fully-read, typing, presence,
timeline, and cleanup tokens on the current tree
(`/tmp/umbrella-live-signals-tuwunel-current.log`). The equivalent fresh
Synapse lane also passes the same CoreCommand/CoreEvent live-signals,
timeline, and cleanup tokens (`/tmp/umbrella-live-signals-synapse-current.log`). A current target/lifecycle
inventory confirms this checkout has only `apps/desktop/package.json` and no
native iOS/Android project or mobile lifecycle harness. Per the clarified
acceptance, mobile is a backend-neutrality contract and its evidence is the
shared Rust unit/toolkit-independent consumer coverage, not a mobile UI
lifecycle run (`/tmp/umbrella-mobile-lifecycle-audit-current.log`). The fresh
backend-neutral toolkit-independent runtime consumer check passes 3/3,
including the large-account room-list storm and missing-room fence
(`/tmp/umbrella-native-mobile-neutrality-current.log`). A fresh Tuwunel
invitation/DM Core lane passes invite receive/accept/decline, member-list,
DM-space scope, restore, and cleanup tokens
(`/tmp/umbrella-invites-tuwunel-current.log`). The equivalent fresh Synapse
invitation/DM lane also passes all invite, member-list, DM-space-scope,
restore, and cleanup tokens (`/tmp/umbrella-invites-synapse-current.log`). The current combined Core
headless gate with `CARGO_BUILD_JOBS=1` also passes login/sync, directory,
timeline reconnect/gap repair, and send-queue FIFO/failure/resend/cancel
scenarios on both Tuwunel and Synapse; all `display_projection_reset_fallbacks`
remain zero (`/tmp/umbrella-headless-core-both-current.log`). The SDK WIP was also checked from its manifest: an initial
`--test read_receipts` selector was invalid because the package exposes only
the `integration` test target; after listing targets, the exact
`read_receipt_snapshot` unit tests pass 2/2 and the integration read-receipt
group passes 13/13, with inherited qualification warnings only
(`/tmp/umbrella-sdk-receipt-snapshot-current.log`,
`/tmp/umbrella-sdk-receipt-current.log`). The rebuilt current Linux
Tuwunel GUI receipt-reader lane also passes subscribe/close, DBus, and
window-state tokens, with artifacts under
`artifacts/linux-gui-current-receipt-readers/`
(`/tmp/umbrella-linux-gui-current-receipt-readers.log`). The process audit found
no child from this run; an unrelated 27-day-old `/tmp/qa460` Tuwunel process
was left untouched (`/tmp/umbrella-linux-gui-process-audit-current.log`; the
mobile target inventory is separate).

## Outstanding at the pre-2026-09-07 checkpoint (historical)

The following list records the state before the continuation entries below; it is
not the current completion checklist.

- Connect the existing internal registry/budget/mailbox to owned subscriptions;
  finish raw/build accounting, terminal lifecycle and public source/window fences.
- Finish compact preparation and AppActor ReaderRow/model publication with direct
  dependency invalidation; connect Tauri/native ownership and revision-qualified
  timeline observations. The current continuation now bounds the legacy compact
  `LiveEventReceiptSummary` to newest three rows with exact totals, and the
  ReceiptReaders UI closes/reopens its full-reader scope on source identity
  changes. These are verified by RED/GREEN focused tests, but do not prove the
  complete compact ViewModel cutover.
- Bounded compact/avatar demand and full-reader UI; remove the old full room-map
  publication in the same cutover.
- Actual HTTP cancellation/dedup/visibility evidence on both supported servers,
  accessibility/IME/locale/browser gates and required performance/retention checks.
- Full-diff parent audit, one cross-model final review, fetchable SDK commit,
  final-head CI and merge.

At that point the legacy receipt publication still materialized all readers;
the current continuation has since removed that action and uses the bounded
summary action for live updates, while the explicit window-reconciliation action
retains full data only for an opened authoritative window. The focused Core
1,500-reader profile-lookup regression is green with at most three profile
requests. This still does not prove complete bounded end-to-end work, and the
unrelated profile-publication RED remains preserved in the integration
worktree. No claim of completion of #839/#840/#846 is justified yet.

## 2026-09-07 execution update

The current WIP was rechecked without resetting or cleaning the worktree. The
the preserved WIP remains based at `2132fc98ff8b1f19d543f70cc8a9d4e7859eb360`
on branch `feat/umbrella-completion`; the parent worktree and SDK
submodule remain dirty and were not reverted. `node scripts/check-agents-docs.mjs`
and `git diff --check` passed.

The following minimal fixes were applied and verified:

- `view_scope_lifecycle/readers/tests.rs`: test callers now pass mutable
  `ReaderWork` as required by the existing profile-registration path.
- `view_scope_lifecycle/readers/raw.rs`: reader guards are scoped before the
  profile index replacement, preserving the existing owner/revision checks.
- `runtime/reducer_support.rs`: `ProfileChanged` effects now synchronously
  dirty only indexed reader scopes at the post-reducer boundary.
- `timeline/item_projection.rs`: live receipt observation profile lookup
  deduplicates and limits IDs to the compact three-reader window; authoritative
  windows retain their requested IDs.

Evidence from the current tree:

- `timeout 120 cargo test -p koushi-core --lib --no-run`: exit 0.
- `cargo test -p koushi-core --lib`: exit 0.
- `compact_receipt_profile_lookup_is_bounded_for_1500_readers`: exit 0.
- `receipt_resolution_borrows_current_alias_without_publishing_global_state`:
  exit 0.
- `view_scope_lifecycle` and `account::profile::tests`: exit 0.

The umbrella continuation on branch `feat/umbrella-completion` recorded a
compact-summary RED/GREEN pair (`/tmp/umbrella-839-compact-summary-red.log`,
`/tmp/umbrella-839-compact-summary-green.log`), state package GREEN (5 focused
live-signals tests and full package tests), and ReceiptReaders source-switch
RED/GREEN (`/tmp/umbrella-839-receipt-source-test-red.log`,
`/tmp/umbrella-839-receipt-source-test-green-final.log`). The old implementation
failed with `overflow_count=0`; the fixed one keeps 1,500 total readers but
publishes 3 compact rows and 1,497 overflow. The source-switch test failed
without the identity dependency because the old scope was not closed, then
passed after the fix. No reader or umbrella completion is claimed.

The continuation also closed a Tauri lifecycle race in the existing reader API:
`receive_receipt_reader` no longer holds the subscription-map mutex across its
pending delivery await. Each map entry now has an independently cancellable
subscription; close signals the pending receiver, while ACK/window operations
lock only the individual subscription. `runtime_stack.rs` verifies the Core
close handle at a 2 MiB worker stack and `koushi-desktop` tests pass (127).
This prevents a popup unmount/source switch from leaving a reader scope and
its producer stranded behind a map lock. The adapter path still needs a real
Tauri IPC artifact and the broader avatar/resource-demand migration remains
open.

A focused UI lifecycle test was added at
`apps/desktop/src/components/timeline/ReceiptReaders.test.tsx`: it fails on the
old `[open]`-only effect because a changed source leaves the old scope open, and
passes with the full source identity dependency. It also verifies the source
snapshot used for the new subscription. The focused Vitest pair (ReceiptReaders
and avatar-thumbnails) is green (6 tests), and desktop typecheck/lint remain
green. The bounded-window UI test also passes: a 300-reader model with a
256-row delivery ACKs revision `7` and sends exactly one next-window request at
`start=256`, preserving the installed revision and sequence.

A first #840 publication slice is now implemented in the umbrella worktree:
room-only `LiveSignalsState` changes produce the new `live_signals_rooms` delta
map with explicit removals instead of cloning the whole live-signals slice;
account-wide presence changes still use the full slice. Rust state-delta tests,
Tauri DTO tests, and frontend `appStore` tests are green (9, 21, and 23 tests
respectively), including reference preservation for unrelated state. The
architecture overview documents the closed contract. Room, Space, and invite
lists now also use scoped `*_by_id` delta maps when ordering is unchanged, with
full ordered replacements reserved for actual order changes. Core and frontend
focused tests cover all three list families. Live receipt publication now also
uses a bounded Core action (`LiveEventReceiptSummaryUpdate`: newest three rows
plus exact total) instead of sending the full reader vector to the reducer;
authoritative reader-window actions retain their full source separately. The
fresh built Tuwunel GUI lane passes compact `3 + 2`, full popup rows, and close.
Room, Space, invite, global-profile-user, room-profile-user, notification-policy,
and room-interaction changes also cross the frontend as scoped maps when
list/order invariants permit. Room notification-policy and room-interaction maps
now use the same bounded room-id update contract.
This is not the complete application-wide publication migration.

The avatar-demand continuation removed room/Space/invite icons from the
account-wide snapshot request planner. `EntityAvatar` now reports an unrequested
avatar only when its rendered element intersects its sidebar viewport, while
`SpaceMembersPanel` keeps its existing equivalent observer; the existing Core
AccountActor remains the dedupe/retry/terminal owner. Shell/WorkspaceRail/Sidebar
pass the same callback without adding a renderer state machine. Timeline sender
avatars now use the same `EntityAvatar` intersection path; the old TimelineView
viewport-range avatar side effect is removed. Focused tests cover the offscreen
snapshot rejection and intersection-triggered request;
typecheck, lint, and 13 focused Vitest tests pass. Full demand cancellation and
shared queue accounting remain open.

The new disposable Linux GUI lane `local-receipt-readers` now drives five real
helper users and a real `m.read` receipt through the Tauri/WebView path. Its
`--skip-build`-free run on Tuwunel passed with
`gui_local_reader_subscribe=ok`, `gui_local_reader_close=ok`, DBus and
window-state tokens (`/tmp/umbrella-gui-reader-real-retry.log`). The first
attempt exposed an incorrect HTTP method in the QA helper (405); POST is now
used and the successful retry is the only green evidence. The five-reader retry
initially proved only one reader because all five marker writes arrived before
the next sliding-sync observation. The lane now uses an observation barrier after
each marker (the next marker is sent only after `Read by N` is rendered), with no
sleep or retry; the fresh Tuwunel run passed compact `3 + 2`, popup reader rows,
and close tokens (`/tmp/umbrella-gui-reader-five-final.log`). The lane is listed
in `docs/agents/qa-lanes.md` and registry checks pass.

This update does not close Phase 1 or Phase 2. Full diff-origin classification,
resource readiness and demand termination integration, real HTTP CoreRuntime
proof, and the remaining host/UI work are still required. The historical RED
sentence above describes the pre-fix state; the focused bounded lookup check is
now green, while end-to-end bounded publication remains unproven.

The documented real-homeserver `live_signals` Core lane was then retried with
its required 240-second lane timeout (the earlier 90-second wrapper timeout was
not a valid lane result). Both commands completed with exit 0:

```text
--server=tuwunel --scenario=live_signals --core --timeout-ms=240000
--server=synapse --scenario=live_signals --core --timeout-ms=240000
```

Logs: `/tmp/issue839-live-signals-240.log` and
`/tmp/issue839-live-signals-synapse.log`. Additional contract checks completed
with exit 0: `cargo test -p koushi-protocol --lib` and
`cargo test -p koushi-state --lib` (`/tmp/issue839-protocol.log`,
`/tmp/issue839-state.log`). The five WIP files reported by the formatter were
formatted with Rust edition 2024 (without resetting other files); the complete
`cargo fmt --all -- --check` then passed. The core suite, agents-docs check,
and `git diff --check` were rerun successfully (`/tmp/issue839-core-after-format.log`,
`/tmp/issue839-docs-after-format.log`). The earlier
`/tmp/issue839-live-signals.log` remains a timeout and is not used as green
evidence.

### Phase 1 diff classification

The current `git status --short`, tracked diff, untracked list, and SDK
submodule diff were inspected. The scoped-reader implementation files,
protocol view DTOs, timeline receipt/index changes, runtime lifecycle changes,
resource lease changes, and their tests/specs are retained as required for this
vertical. `Cargo.lock`, existing core/state files, and the SDK
`read_receipt_snapshot.rs` change are retained as user/WIP inputs and are not
silently reverted. No path was classified for deletion solely because it was
untracked or large; deletion remains conditional on an equivalent replacement
and the same checks. The branch still contains unrelated pre-existing dirty
files, so the tree is not ready for commit or merge without a final owner-by-
owner diff review. At that checkpoint the inventory contained 27 tracked and
29 untracked paths; the current worktree audit is recorded separately below.
All untracked Rust/protocol files were included in the successful Core/Protocol
compilation and tests; the 12 untracked specification/plan documents were read
or referenced by the worklog.
A scan of the scoped implementation files found no `TODO`, `FIXME`,
`unimplemented!`, or `todo!` placeholders. This is an inventory/audit result,
not permission to delete or commit any user/WIP path.

### Current continuation — search-crawler scoped delta

The application-wide state-delta slice carries search-crawler room replacements
as `search_crawler_rooms_by_id` and the single `last_active` record as
`search_crawler_last_active`; changing one no longer clones the unrelated
crawler map. The protocol, Tauri DTO, and frontend appStore merge preserve
unrelated references and explicit removals. Fresh focused evidence:
`cargo test -p koushi-core state_delta` (22 passed),
`cargo test -p koushi-desktop search_crawler --lib` (2 passed), and
`vitest run src/domain/appStore.test.ts` (33 passed); desktop typecheck passed
(`/tmp/umbrella-search-crawler-core.log`,
`/tmp/umbrella-search-crawler-tauri.log`,
`/tmp/umbrella-search-crawler-vitest.log`,
`/tmp/umbrella-search-crawler-typecheck.log`). This remains a partial #840
migration; ordered activity/room-list slices and other explicitly classified
whole-slice state still require their own contract/evidence review.

A second audit found the account-level `live_signals.presence` map had the same
unrelated-clone problem. It now publishes `live_signals_presence_by_user`, with
explicit user removal and frontend reference-preserving merge coverage. Core
state-delta coverage is 24/24, Tauri presence/crawler boundary coverage is 3/3,
and appStore coverage is 34/34; the initial TypeScript run caught and fixed a
local-vs-imported `PresenceKind` name collision, then typecheck passed
(`/tmp/umbrella-live-presence-core.log`,
`/tmp/umbrella-live-presence-tauri-2.log`,
`/tmp/umbrella-live-presence-tauri-3.log`,
`/tmp/umbrella-live-presence-vitest.log`,
`/tmp/umbrella-live-presence-typecheck-2.log`). The current builder now has
no `changed.search_crawler` or `changed.live_signals` whole-slice assignment;
remaining full assignments are limited to ordered-list/stream-shape fallbacks
that the contract classifies as structural replacement. A follow-up profile audit
also removed the final routine whole-`ProfileState` fallback: own profile,
local aliases, ignored-user membership, and each update status now have
independent delta fields, including simultaneous global and room-user changes.
Core state-delta coverage is 26/26, appStore coverage is 35/35, desktop
profile DTO coverage is 3/3, and typecheck passes
(`/tmp/umbrella-profile-scalars-core.log`,
`/tmp/umbrella-profile-scalars-vitest.log`,
`/tmp/umbrella-profile-scalars-tauri.log`,
`/tmp/umbrella-profile-scalars-typecheck.log`,
`/tmp/umbrella-profile-dto-tests.log`). The architecture overview was updated
for the new profile/search-crawler/presence delta fields. A source audit confirms no
whole-slice assignment remains for `profile`, `search_crawler`, or
`live_signals` in `build_state_delta`; only the corresponding scoped fields are
populated (`/tmp/umbrella-scoped-assignment-audit.log`). Frontend lint, IME, docs, and
semantic-owner checks remain green (`/tmp/umbrella-profile-scalars-lint.log`).

The receipt consumer was then cut over from renderer-created `details` strings
to Rust-shaped `ReaderRow` models. Compact summaries no longer format dates or
carry a legacy details prop; opened windows format only delivered, validated
rows using the locale supplied in each `ReceiptTimestamp`. Reader avatars now
reuse the existing visibility-triggered `EntityAvatar` demand path, and the
opened list is an accessible `dialog` with pointer scrolling and Escape/blur
closure instead of an inert tooltip. Focused ReceiptReaders/Timeline live-state
Vitest and desktop typecheck passed. A fresh built Tuwunel Linux GUI lane also
passed with `gui_local_reader_subscribe=ok`, `gui_local_reader_close=ok`,
`notification_dbus=ok`, and `window_state_path_contract=ok` in
`/tmp/umbrella-gui-receipt-dialog.log` and
`artifacts/linux-gui-local-receipt-readers-dialog/`. Full avatar-demand
cancellation, shared queue accounting, and the remaining native/resource
requirements are still open.

The obsolete unbounded live receipt action was removed rather than retained as
an adapter path. Existing full-reader test fixtures now name the explicit
`LiveRoomReceiptsWindowReconciled` action, while production live observations
use only `LiveRoomReceiptSummariesUpdated`; the reducer and profile diagnostics
no longer consume the old action. Fresh `koushi-state` (41 tests),
`koushi-core` (989 passed, 8 ignored), and rust source-structure checks passed.
The opened receipt surface now handles `readerLoading` and terminal retirement
without ACK, with focused ReceiptReaders tests covering that failure contract.
A source audit finds no `LiveRoomReceiptsUpdated` reference in production
Rust, Tauri, or frontend sources; only the bounded summary and explicit window
paths remain.

The existing AccountActor avatar owner was tightened to match the approved
capacity contract: six active fetches, at most 256 queued distinct MXCs, and no
semaphore-waiting JoinSet task per queued URI. Duplicate waiters remain
single-flight; session teardown clears queued metadata and active accounting;
late generation results remain rejected. Saturated demand settles with the
Rust-owned typed `capacity` thumbnail failure. Real `MatrixMockServer` actor
coverage now fills six delayed active requests plus the 256-entry distinct
pending queue and proves the next request settles with `capacity`; a second
real-session test replaces the session and proves a generation-0 late result is
not cached, with the current request reaching the expected network terminal.
The focused `koushi-core` tests pass (`/tmp/umbrella-avatar-deterministic.log`,
`/tmp/umbrella-avatar-late.log`). End-to-end shared-scope demand evidence and
platform-specific resource checks remain open. The first workspace run exposed
an ordering-sensitive avatar test fixture that shut down before all six delayed
requests had reached the mock server; the test now waits on the observed six
requests with a bounded condition (no sleep/retry), and the focused capacity
case passes.

The existing toolkit-independent `koushi-core-testkit` native consumer was
also rerun against the real CoreRuntime large-account selection harness:
`runtime_room_selection_scale` passes 3/3 for 110 non-DM rooms, 57 DMs, and
five spaces, including the background room-list storm and missing-room fence
(`/tmp/umbrella-native-110-room.log`). This supplies current second-consumer
and 100-room-class runtime evidence, satisfying the clarified mobile
backend-neutrality scope but not the macOS GUI recurrence proof. The macOS
viewport parser/source contract tests pass 51/51 (`/tmp/umbrella-macos-viewport-contract.log`), but this Linux
environment cannot produce the native macOS recurrence artifact. The mobile
acceptance clarification means the 3/3 toolkit-independent consumer/unit
result is sufficient for backend neutrality; the mobile upper layer may remain
a thin adapter and no mobile UI lifecycle artifact is required. The same
clarification is explicit in the architecture overview and scoped-publication
contract; changed-document links, diff, and agents-doc
checks pass (`/tmp/umbrella-doc-links-mobile-scope.log`,
`/tmp/umbrella-diff-mobile-scope.log`,
`/tmp/umbrella-agents-mobile-scope.log`). A disposable `/tmp` candidate screen was kept separate from acceptance. After
triaging the initial Virtua container and generic scroller setup failures, the
corrected toy schedule runs all three modes: Virtua `shift=true` moves the
anchor -40px, Virtua `shift=false` moves it +240px, while React Virtuoso with
`firstItemIndex` adjustment holds it at 0px. This is preliminary synthetic-row
screening only—not production-row, Rust-batch, native-motion, accessibility,
or engine-selection evidence. No repository dependency or production code
changed (`/tmp/umbrella-viewport-screen-summary.md`,
`/tmp/umbrella-viewport-screen-results-static-scroll4.log`). A post-screen
status comparison confirms the repository tree is unchanged (116 tracked
modifications, 26 untracked paths, one dirty SDK submodule) and `git diff --check`
remains green (`/tmp/umbrella-status-post-viewport-screen.log`). The post-screen
format/diff and agents-doc checks are also green
(`/tmp/umbrella-final-diff-viewport-screen.log`,
`/tmp/umbrella-final-agents-viewport-screen.log`).

The existing timeline viewport engine received a 100,000-row characterization
at its calculation boundary. It uses one virtualized list window and asserts
that the rendered range remains bounded with positive off-window padding;
`vitest run src/components/timeline/TimelineViewportVirtualization.test.ts`
passed (2 tests, `/tmp/umbrella-100k-viewport.log`). This proves the current
single-list range calculation only, not a 100,000-event homeserver run or the
remaining #846 replacement/portable-consumer evidence.

Activity publication now also has a narrow scoped-row path. When both open
activity streams retain their order and pagination/resolution metadata, Rust
publishes only changed rows keyed by event/room identity through
`activity_recent_rows_by_id` and `activity_unread_rows_by_id`; stream shape,
ordering, tab, mark-read, or metadata changes retain the full activity slice.
The frontend merges these row maps without replacing stream metadata. Fresh
state-delta tests pass 21/21, protocol tests 33/33, DTO tests 23/23, appStore
Vitest 32/32, and desktop typecheck passes (`/tmp/umbrella-activity-*.log`).
This is an additional partial #840 slice; other inventoried whole-slice paths
remain open. Scoped list maps now recognize a surviving-order subsequence, so
room/Space/invite removals use explicit null entries instead of falling back to
a full list solely because an item disappeared; insertions and reorders still
use the full ordered slice. Fresh core state-delta tests pass 21/21 and the
appStore suite passes 32/32 (`/tmp/umbrella-subsequence-*.log`). A 100-room
fixture changing one unread count publishes exactly one room replacement
(`/tmp/umbrella-100-room-delta.log`). The full desktop Vitest suite was rerun after updating the
approved DesktopApi migration map for the reader commands: 107 files and
1,266 tests passed (`/tmp/umbrella-final-vitest-resource-serial.log`). The matching Rust
forwarder contract artifact/test was updated for the scoped search-crawler
shape; fresh `koushi-desktop` lib tests pass 129/129
(`/tmp/umbrella-final-src-tauri.log`). The frontend production build also
passes after the serialized resource-read change (`/tmp/umbrella-final-build-resource-serial.log`), and
again passes after the wire/keyboard corrections (`/tmp/umbrella-reader-final-build.log`).
Fresh post-correction typecheck, lint/IME/docs/semantic-owner, Tauri-boundary,
domain/leaf-boundary, Rust-test-structure, and State/Search/Protocol wasm checks
all pass (`/tmp/umbrella-reader-keyboard-typecheck.log`,
`/tmp/umbrella-reader-keyboard-lint.log`, `/tmp/umbrella-final-tauri-boundary.log`,
`/tmp/umbrella-final-domain-boundaries.log`, `/tmp/umbrella-final-rust-test-structure.log`,
`/tmp/umbrella-final-wasm-after-reader.log`).
and the documented State/Search/Protocol wasm check passes
(`/tmp/umbrella-final-wasm.log`). Frontend lint, IME, docs and semantic-owner
checks also remain green (`/tmp/issue839-reader-resource-lint-2.log`). The
post-resource structural checks pass formatting, diff-check, Rust test structure,
agents-docs, leaf boundaries, and protocol/QA boundaries
(`/tmp/umbrella-final-resource-*.log`). Dependency
checks: `cargo deny check` and `cargo machete` pass
(`/tmp/umbrella-final-cargo-deny.log`, `/tmp/umbrella-final-cargo-machete.log`).
`cargo audit` remains non-green with its two inherited high-severity
`quick-xml 0.39.4` advisories `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195`
through Tauri's `plist` dependency (`/tmp/umbrella-final-cargo-audit.log`);
no speculative dependency upgrade was made. The SDK WIP receipt snapshot file
formats cleanly and its focused `matrix-sdk-ui` read-receipt suite passes 14/14
from the SDK manifest (`/tmp/umbrella-sdk-receipt-tests-2.log`,
`/tmp/umbrella-sdk-fmt-3.log`); the root SDK gitlink guard also passes. After
the list-map refinement, the
full `koushi-core` lib suite passes 998 with 8 ignored
(`/tmp/umbrella-core-after-all.log`).

The browser harness now mirrors the reader scope lifecycle for its intentional
Tauri test backend: subscribe creates one bounded scope from the current
compact summary, receive delivers one Rust-shaped `readerReady` model, and
ack/update/close remain explicit command calls. A source-less timeline uses
its bounded compact summary rather than a permanent full-reader loading state;
a committed source upgrades to the scope path. The two affected receipt-avatar
Playwright cases pass after using a valid image fixture and asserting the
remaining compact count (`/tmp/umbrella-browser-receipt-fixed-3.log`). The full
frontend Vitest suite remains green at 107 files / 1,266 tests
(`/tmp/umbrella-final-vitest-resource-serial.log`). The complete documented browser-headless
lane then passed 289 Playwright cases, including the reader, avatar, timeline,
IME, accessibility, and scroll suites (`/tmp/umbrella-final-browser-resource-serial.log`).
After the reader wire/keyboard/ARIA and compact-row geometry corrections, the
same complete lane was rerun and again passed all 289 cases
(`/tmp/umbrella-final-browser-keyboard.log`).
The fresh built Linux/Tuwunel GUI `local-receipt-readers` lane also passed its
subscribe/close, DBus, and window-state tokens with artifacts under
`artifacts/linux-gui-reader-final/` (`/tmp/umbrella-linux-gui-reader-final.log`).
The complete Rust workspace suite now passes after that deterministic test
fix and the scoped resource command (`/tmp/umbrella-final-workspace-resource.log`).
After the source-index, lease-admission, and reader-wire changes it was rerun
fresh: 2,571 passed / 12 ignored across 131 suites, with no failures
(`/tmp/umbrella-final-workspace-after-source-key.log`). The leaf-crate boundary check
initially caught the new Core-local
`runtime_stack.rs` target as unregistered; the target is now explicitly listed
in the boundary contract, `node scripts/check-leaf-crate-boundaries.mjs`
passes, and the three runtime-stack tests pass
(`/tmp/umbrella-runtime-stack-final.log`). Final local structural audit also
passes Rust formatting, diff-check, Rust test-structure, agents-docs, and leaf
crate boundaries (`/tmp/umbrella-final-audit-*.log`). The protocol/QA boundary
check remains green after the resource command (`/tmp/issue839-reader-resource-boundary.log`).
A focused dead-code cleanup removed the unused production `OwnedViewScope::spawn_producer`
wrapper (the test-only helper remains cfg-gated) and the redundant `ReaderWork`
`source_dirty` copy and unused installed-row metadata fields; Core remains green at
998 passed / 8 ignored (`/tmp/umbrella-warning-cleanup-core.log`). Remaining
warnings are inherited in other modules/SDK WIP and are not hidden by broad
allow attributes.

The host resource boundary is now connected for opened reader rows. Core exposes
`ReaderSubscription::resource_content`, the Tauri command validates the live
window-owned subscription, installed revision and opaque source reference before
copying bytes, and ReceiptReaders creates/revokes local object URLs only after
ACK. Avatar-thumbnail completion now also publishes the exact changed user IDs
for live-receipt copies, so registered reader scopes are dirtied directly instead
of relying only on a broad live-signal rerender; the new live-only invalidation
coverage and existing snapshot/avatar coverage pass (`/tmp/umbrella-avatar-invalidation-state-3.log`).
Receipt sources are now indexed by account/room/event in the lifecycle registry,
with conservative byte accounting for both the retained key and index membership;
accepted live receipt actions invalidate only matching scopes with a fresh raw
read. The actual AppActor source-change regression is green
(`/tmp/umbrella-source-hook-core-3.log`), as is the accounting-aware registry
coverage (`/tmp/umbrella-source-index-accounting-core.log`).
The complete State crate suite (including 3 doctests) and the dependent Core
suite remain green after this reducer change (`/tmp/umbrella-avatar-invalidation-state-full.log`,
`/tmp/umbrella-core-after-avatar-invalidation.log`). Reader scopes now register
an indexed `(account, room, event)` source key and receipt actions invalidate
only matching scopes; the focused registry test passes
(`/tmp/umbrella-source-index-core.log`). Resource lease admission now
maps an exhausted private lease budget to typed scoped `Capacity` before a
`ReaderReady` model can be published; the focused regression passes
(`/tmp/umbrella-lease-capacity-core-3.log`). The full dependent Core lib suite
now passes 1,001 / 8 ignored after the source-index, lease-admission, and
avatar-cancellation changes (`/tmp/umbrella-avatar-cancel-core-full-2.log`). Compact summaries retain the existing unscoped cache path; full reader rows
cannot silently fall back to a bare cache reference. Resource reads are serialized
per opened window, so a bounded 256-row model cannot create a burst of 256
IPC/blob allocations. The frontend `ReaderRow` contract now matches the actual
protocol wire shape: avatar MXC identities remain private and only thumbnail
state crosses the reader boundary; the browser fixture and resource test use
that exact shape (`/tmp/umbrella-reader-wire-typecheck.log`,
`/tmp/umbrella-reader-wire-vitest.log`). The affected live-signal/receipt browser
subset, including the compact single-line geometry regression, passes 4/4
(`/tmp/umbrella-reader-keyboard-playwright-2.log`). The reader dialog now exposes a bounded
ARIA list with roving row focus, Home/End/Page/arrow navigation, revision-qualified
window requests, and explicit `aria-setsize`/`aria-posinset`; the End request
regression is green (`/tmp/umbrella-reader-keyboard-vitest-5.log`). The focused client/reader
contract suite passes 32/32 (`/tmp/umbrella-reader-focused-final-2.log`). Fresh Tauri lib tests remain green at
130/130 (`/tmp/umbrella-avatar-cancel-string-tauri.log`). The full
frontend suite remains green at
107 files / 1,267 tests after this wire correction
(`/tmp/umbrella-reader-wire-vitest-full.log`). The new keyboard/ARIA regression
brings the fresh complete suite to 1,267 passing tests
(`/tmp/umbrella-reader-keyboard-full-vitest.log`). The Tauri client contract,
headless harness mirror, focused client/ReceiptReaders tests (32/32), typecheck,
Tauri tests (130/130), full Core tests (1,001 passed, 8 ignored), affected
receipt Playwright cases (2/2), and the fresh built Linux/Tuwunel GUI reader
lane are green (`/tmp/issue839-reader-resource-*.log`,
`/tmp/umbrella-resource-final-tauri.log`,
`/tmp/issue839-reader-resource-core-tests.log`,
`/tmp/issue839-reader-resource-gui.log`,
`artifacts/linux-gui-reader-resource/`).

Avatar demand cancellation is now owned end-to-end: the Core account actor
tracks abort handles for active and semaphore-queued fetches, removes only the
released waiter for shared MXCs, advances pending work when the last waiter is
released, and rejects late canceled completions. Tauri returns the opaque
request sequence and exposes a matching cancel command; the App registry
reference-counts snapshot/visible consumers, and intersection cleanup releases
its demand. Real delayed MatrixMockServer coverage proves canceling one of six
active fetches admits the seventh pending URI without publishing a terminal
event; a cross-connection target is also rejected
(`/tmp/umbrella-avatar-profile-final-2.log`); the profile avatar subset is
4/4 (`/tmp/umbrella-avatar-profile-final-2.log`), Tauri is 130/130
(`/tmp/umbrella-avatar-cancel-string-tauri.log`), and the affected frontend
observer tests are 61/61 (`/tmp/umbrella-avatar-cancel-vitest-final.log`).

The required maximum Tuwunel stress lane was attempted with the documented
10-space × 10-room × 100-message configuration. The run did not produce green
scale evidence: after the base timeline stage, the homeserver rejected a
later stress-room creation with `RoomOperationFailed { kind: Network }`
(`/tmp/umbrella-timeline-stress-100room-1msg-2.log`; the earlier 100-message
attempt instead lost the same local stress flow under load). The default
2-space × 2-room × 8-message Tuwunel lane still passes. This is retained as a
real infrastructure/scale blocker, not hidden by increasing retries or
weakening assertions. The equivalent maximum Synapse attempt also failed
under the configured scale while waiting for a stress send terminal
(`local_echo=true`, `send_completed=false`; `/tmp/umbrella-timeline-stress-100room-synapse.log`). No further blind retries were made.

The post-cancellation complete Rust workspace suite passes 2,574 tests with
12 ignored across 132 suites (`/tmp/umbrella-avatar-cancel-workspace-final.log`),
and the frontend remains green at 107 Vitest files / 1,267 tests plus a fresh
289-case Playwright lane (`/tmp/umbrella-avatar-cancel-vitest-full-final.log`,
`/tmp/umbrella-avatar-cancel-browser-final-string.log`). The post-cancellation production build also passes
(`/tmp/umbrella-avatar-cancel-build-final-2.log`). Fresh final boundary and test-structure checks also pass
(`/tmp/umbrella-avatar-cancel-tauri-boundary-final.log`,
`/tmp/umbrella-avatar-cancel-domain-boundaries-final.log`,
`/tmp/umbrella-avatar-cancel-rust-structure-final.log`,
`/tmp/umbrella-avatar-cancel-wasm-final.log`). The rebuilt Linux/Tuwunel
GUI reader scenario also passes its required tokens
(`/tmp/umbrella-avatar-cancel-gui-built.log`,
`artifacts/linux-gui-avatar-cancel-reader-built/`).

Fresh real-homeserver CoreCommand/CoreEvent evidence for the live-signals
vertical also passes on both supported servers with the current tree:
`--server=tuwunel --core --scenario=live_signals --timeout-ms=240000`
and the equivalent Synapse command both report receipt, fully-read, typing,
presence, and `live_signals=ok` (`/tmp/umbrella-live-signals-tuwunel-final.log`
and `/tmp/umbrella-live-signals-synapse-final.log`). The required invitation/DM
Core lanes were also rerun on Tuwunel and Synapse and both passed invite,
member-list, DM-space-scope, and cleanup tokens
(`/tmp/umbrella-invites-tuwunel-final.log`,
`/tmp/umbrella-invites-synapse-final.log`).

### Current completion blockers

The referenced #846 proposal was restored into this worktree at
`docs/superpowers/specs/2026-09-05-timeline-viewport-redesign.md` from its
reviewed design commit, fixing the previously broken architecture link. It
remains a proposal: no engine was selected or production viewport code changed.
The documentation/link check and `git diff --check` pass
(`/tmp/umbrella-viewport-spec-docs-check.log`). A fresh ownership/deletion scan
finds no obsolete receipt action or placeholders in active scoped Rust/Tauri/
frontend implementation paths, and no whole-slice profile/search/live-signal
assignment in the builder. Retired backend vocabulary remains only in the
intentional rejection guards, archival/history material, and older superseded
plans; active production paths are clean
(`/tmp/umbrella-ownership-deletion-audit-current.log`). A warning cleanup removed
unused request-progress payload fields while preserving request/room/generation
fencing, and documented the intentionally retained recovery outcome contract;
`cargo check -p koushi-core --lib` now reports no `koushi-core` warnings and the
recovery state-machine tests pass 11/11
(`/tmp/umbrella-core-warning-cleanup-final.log`,
`/tmp/umbrella-recovery-model-cleanup-tests.log`). The full `koushi-core` lib
suite passes 1,009/1,009 and the request-outcome testkit passes 10/10
(`/tmp/umbrella-core-warning-cleanup-corelib.log`,
`/tmp/umbrella-request-outcome-cleanup-testkit.log`). The post-cleanup diff and
agents-doc checks remain green (`/tmp/umbrella-final-diff-warning-cleanup.log`,
`/tmp/umbrella-final-agents-warning-cleanup.log`). The checked-in generated
CoreEvent wire artifact also matches Rust serialization (IPC contract test 1/1)
(`/tmp/umbrella-generated-contract-current.log`). A fresh completion audit
maps each requirement to its current evidence, characterization-only result,
or blocker; it confirms that the clarified mobile backend-neutrality scope is
satisfied but the umbrella goal is not complete
(`/tmp/issue839-goal-completion-audit-current.md`). The post-audit diff and
agents-doc checks are also green (`/tmp/umbrella-final-diff-goal-audit.log`,
`/tmp/umbrella-final-agents-goal-audit.log`).

- The maximum 10-space × 10-room × 100-message stress configuration fails under
  both Tuwunel and Synapse at the real-server send/room-load stage; the default
  lane remains green. Failure triage now distinguishes Tuwunel's completed
  `SendCompleted` without a local-echo item from Synapse's local echo without a
  completed send. A fresh reduced-cadence Tuwunel run passes 1×5×20 and then
  identifies the first failure at s0/r5/m1 for 1×10×100, while a separate
  1×6×100 run passes, and fresh 1×10×20 runs pass on both Tuwunel and
  Synapse with 201 messages; the documented state machine permits an atomically
  event-ID replacement when no local echo exists, so reconciling that contract
  with the stress lane's mandatory local-echo oracle is a design blocker. The
  exact next diagnostic is retained rather than weakening the shared waiter;
  the Core coordinator's no-local-echo contract test passes 1/1
  (`/tmp/umbrella-timeline-stress-failure-triage-current.md`,
  `/tmp/umbrella-send-no-local-echo-contract.log`). Synapse access-log
  analysis found 692 room-message PUTs: 691 completed in 0.019–0.044 s and
  one took 30.128 s before the client disconnected, matching the 30 s QA
  deadline (`/tmp/umbrella-synapse-stress-send-all-latency.log`).
  The waiter now reports the exact space/room/message coordinate on failure, and
  its QA helper is test-only; the headless-core-qa test binary passes 102/102
  (`/tmp/umbrella-stress-diagnostic-qa-tests-fixed.log`). The final diff and
agents-doc checks remain green (`/tmp/umbrella-final-diff-stress-triage.log`,
`/tmp/umbrella-final-agents-stress-triage.log`). The send-contract audit's
post-check diff and agents-doc checks are also green
(`/tmp/umbrella-final-diff-send-contract.log`,
`/tmp/umbrella-final-agents-send-contract.log`). Fresh reduced 1×10×20
Tuwunel and Synapse runs both pass 201 messages
(`/tmp/umbrella-timeline-stress-triage-tuwunel-1x10x20.log`,
`/tmp/umbrella-timeline-stress-triage-synapse-1x10x20.log`). The follow-up
diff and agents-doc checks remain green
(`/tmp/umbrella-final-diff-stress-followup-2.log`,
`/tmp/umbrella-final-agents-stress-followup-2.log`). The latest post-audit
diff and agents-doc checks also pass
(`/tmp/umbrella-final-diff-stress-followup-5.log`,
`/tmp/umbrella-final-agents-stress-followup-5.log`). The QA lane documentation now
formally distinguishes the product-compatible no-local-echo state-machine path
from the deliberately stricter `timeline_stress` diagnostic oracle: each stress
send still requires its exact local echo, matching `SendCompleted`, and event ID;
a missing echo remains a recorded failure rather than a weakened assertion
(`/tmp/umbrella-no-local-echo-contract-docs.log`,
`/tmp/umbrella-no-local-echo-contract-diff.log`).
- #846's approved design still explicitly leaves production unchanged until a
  comparative Virtua/React-Virtuoso feasibility gate and native-motion test
  select an engine. The current branch retains the existing viewport controller
  and characterization; it does not have a defensible single-engine replacement.
  Implementing or selecting one without the required macOS WKWebView/trackpad
  evidence would silently change the approved design. No assertion weakening or blind retry is allowed.
- macOS native viewport recurrence cannot be produced in this Linux environment.
  The clarified mobile requirement is backend neutrality, which is covered by
  the shared Rust/toolkit-independent unit and consumer tests; no mobile UI
  lifecycle claim is made.
- `cargo audit` now exits 0 after the compatible `plist 1.10.1` lockfile update,
  which selected `quick-xml 0.42.0` and `base64 0.23.1`; `cargo deny` and
  `cargo machete` pass. The inherited yanked `chacha20 0.10.1` remains a
  warning-level audit finding and no unreviewed dependency upgrade or exception
  was made (`/tmp/umbrella-cargo-audit-final-current.log`,
  `/tmp/umbrella-final-cargo-deny-2.log`,
  `/tmp/umbrella-final-cargo-machete-2.log`).
- The SDK submodule's public `ReadReceiptSnapshot::changes_since` API in
  `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/timeline/event_item/read_receipt_snapshot.rs`
  is committed as `e85bc9e762592fee316839908f29cd6015973578` and pushed to
  `origin/koushi/shared-receipt-snapshots`; the focused UI receipt suite passes
  2/2 (`/tmp/umbrella-sdk-receipt-publication-focused.log`). The parent gitlink
  and remote SDK ref are synchronized, and the current submitted worktree has
  no SDK or untracked diff (`node scripts/check-sdk-submodule.mjs`,
  `git status --short`).
- The ownership/deletion audit, reproducible clean submitted-tree check, and
  exact-head umbrella CI are complete; the remaining PR work is review and merge.
  Active production paths contain no obsolete receipt action or placeholder, and
  retired backend vocabulary is limited to intentional guards/history material
  (`/tmp/umbrella-ownership-deletion-audit-current.log`). The combined state-delta
  regression and full workspace evidence remain green
  (`/tmp/umbrella-final-workspace-after-combined-live-delta.log`). Final
  formatting, diff, Rust test-structure, agents-doc, SDK, and ownership checks
  pass. Current `feat/umbrella-completion` HEAD is
  `077ec490339c73b8bb91f2cb410aa872a2943f51`; worktree and remote match with no
  untracked files, and CI run `34231276865` passes all nine required jobs. PR
  #857 is open Draft with `mergeStateStatus: CLEAN` and currently has no reviews
  or comments.

A search-crawler delta audit found one remaining whole-slice path: changing
`last_active` previously cloned the complete crawler room map. The protocol,
Core builder, Tauri DTO, and appStore now publish/apply
`search_crawler_last_active` independently, while room entries continue through
`search_crawler_rooms_by_id`; explicit full snapshots and legacy direct delta
construction remain supported. Focused Core coverage is 22/22,
Tauri crawler-boundary coverage is 2/2, and the appStore file is 33/33
(`/tmp/umbrella-search-crawler-core.log`,
`/tmp/umbrella-search-crawler-tauri.log`,
`/tmp/umbrella-search-crawler-vitest.log`). TypeScript typecheck passes
(`/tmp/umbrella-search-crawler-typecheck.log`).

A warning/dead-metadata cleanup made the internal `ReaderSourceKey` visibility
consistent, removed unused production reader metadata and obsolete send/receipt
helpers, and kept test-only assertions behind test configuration without broad
warning suppression. The obsolete two-hop `ResolveReceiptWindow` envelope and
connection helper are test-only; production readers use the bounded
`ReadReceiptWindow`/`ReaderPrepared` route. Focused connection coverage is 21/21,
read-state coverage is 69/69, and the complete workspace suite remains 2,574
passed / 12 ignored (`/tmp/umbrella-warning-cleanup-tests-2.log`,
`/tmp/dead-cleanup-read-tests-2.log`,
`/tmp/resolve-dead-cleanup-runtime-tests-2.log`,
`/tmp/dead-cleanup-workspace-final-2.log`). The fresh production Core check is
at 25 warnings, with the removed scoped-path warnings absent
(`/tmp/dead-cleanup-check-11.log`); formatting and `git diff --check` also pass
(`/tmp/dead-cleanup-fmt-check-final.log`,
`/tmp/dead-cleanup-agents-final.log`,
`/tmp/dead-cleanup-diff-check-final.log`,
`/tmp/umbrella-final-rust-structure-2.log`,
`/tmp/umbrella-final-leaf-boundaries-2.log`,
`/tmp/umbrella-final-tauri-boundary-2.log`,
`/tmp/umbrella-final-domain-deps-2.log`,
`/tmp/umbrella-final-frontend-owners-2.log`).

After the profile, search-crawler, and live-presence delta changes, the default
full workspace suite was rerun without concurrent Cargo commands: 2,580 passed,
12 ignored, 0 failed (`/tmp/umbrella-final-workspace-after-scoped-deltas-default.log`).
The later DTO boundary test also passes, and the final rerun is 2,581 passed,
12 ignored across 132 suites with 0 failures
(`/tmp/umbrella-profile-dto-tests.log`,
`/tmp/umbrella-final-workspace-after-all-scoped-deltas.log`). The final Rust and frontend gates are green for this source state: the full
frontend suite is 1,270 tests across 107 files with no failures and no
React duplicate-key warning; the invite-search fixture now gives its two
candidates distinct identities. Production frontend build also passes after
the expanded state-delta contract
(`/tmp/umbrella-final-build-after-scoped-deltas.log`,
`/tmp/umbrella-final-vitest-warning-free.log`,
`/tmp/umbrella-space-member-warning-fix.log`,
`/tmp/umbrella-final-vitest-after-scoped-deltas.log`). A parallel full-workspace
run exposed a shared renderable-thumbnail test-cache race; the common cache
lock now covers renderable-thumbnail and avatar-actor tests, and the full
workspace rerun is green. The production Core warning count is down from 25
to 2 (only retained SDK-independent state-model/test-boundary metadata warnings;
SDK emits six inherited qualification warnings)
(`/tmp/umbrella-warning-cleanup-final-check-4.log`,
`/tmp/umbrella-cache-lock-renderable.log`,
`/tmp/umbrella-cache-lock-profile.log`,
`/tmp/umbrella-final-workspace-after-cache-lock2.log` (2,581 passed / 12
ignored across 132 suites, 0 failed). Final diff, agents-docs,
semantic-owner, and SDK path/gitlink guards pass
(`/tmp/umbrella-final-diff-check-5.log` (and no trailing whitespace in the
changed scoped files, `/tmp/umbrella-final-scoped-trailing-whitespace.log`),
`/tmp/umbrella-final-fmt-check-4.log`,
`/tmp/umbrella-final-rust-structure-3.log`,
`/tmp/umbrella-final-agents-3.log`,
`/tmp/umbrella-final-owners-3.log`,
`/tmp/umbrella-final-sdk-guard-3.log`,
`/tmp/umbrella-final-lint-warning-free.log`).

## Current submitted-branch follow-up

The SDK receipt snapshot WIP was committed and published at
`e85bc9e762592fee316839908f29cd6015973578` on
`origin/koushi/shared-receipt-snapshots`; the parent gitlink was updated and
committed in the umbrella branch. The umbrella changes were committed as
`5affd113`, then merged with `origin/main` (including runtime reconnect fixes)
as `7e24a15d`, and pushed to `origin/feat/umbrella-completion`. The direct-send
fix and test-teardown warning cleanup are now committed as `18a2e355` and
`932240aa`, respectively, and the current pushed head is clean.

Post-merge verification passes: the full workspace suite before the latest
send fix reported 2,562 passed, 0 failed
(`/tmp/umbrella-final-workspace-after-main.log`); after the send fix and
projection regression it reports 2,564 passed, 0 failed
(`/tmp/umbrella-final-workspace-send-all.log`). `koushi-desktop` lib tests pass
135/135; `koushi-core` runtime-stack tests pass 3/3; `koushi-sdk` lib tests pass
143/143; frontend Vitest passes 1,272/1,272;
typecheck, lint, build, Tauri/domain/leaf/SDK/secret/structure/docs checks all
pass (`/tmp/umbrella-final-*after-main.log`). The build retains only the
pre-existing Vite chunk-size and ineffective dynamic-import warnings. The first
PR #857 CI run exposed a real generated-artifact mismatch in the README
screenshot: the new Rust-demand avatar path intentionally applies deterministic
per-sender colors. Re-running `docs:screenshot` produced a stable 2560×1600
asset, and a second local generation matches it byte-for-byte; the refreshed
`assets/screenshots/koushi-main.png` is included in the next commit.

The strict send waiter exposed a concrete direct-send race. `TimelineCommand::SendText`
and `SendReply` now install a bounded manager-owned pending projection and wait
for the generation-fenced actor refresh ACK before starting the SDK enqueue.
The actor no longer retires that fallback before the terminal state is
published; direct projections remain available for recoverable failures,
pre-bind SDK local echoes are tracked until binding, and direct retained rows
are dropped when their timeline actor is gone. This preserves the strict
local-echo plus terminal oracle without sleeps or weakened expectations. The regression test
`local_echo_before_sdk_bind_preserves_fallback_until_terminal` passes; the
canonical send-state regression passes 23/23, and the fast send-queue lane
passes 7/7. Tuwunel `1×10×100` (1,001 messages) and `4×10×100` (4,001
messages) pass (`/tmp/umbrella-final-tuwunel-direct-retained.log`,
`/tmp/umbrella-timeline-stress-prebind-marker-trace-4x10x100.log`). A fresh
Synapse `1×10×20` lane passes 201 messages
(`/tmp/umbrella-timeline-stress-direct-projection-synapse-1x10x20.log`).
The latest full `10×10×100` Tuwunel attempts progressed through space 8 and then
failed creating room 9 with `RoomOperationFailed { kind: Network }`; no send
assertion was relaxed (`/tmp/umbrella-timeline-stress-max-final-current.log`,
`/tmp/umbrella-timeline-stress-max-after-retained-drop.log`). The latest run
reached the same room-creation boundary after direct-retained cleanup, with no
send-flow queue overflow.

A fresh Synapse `1×10×100` run now reaches `s0/r7/m0` with
`local_echo=true local_echo_send_state=Sending send_completed=false event_id=false`.
Its lifecycle ring shows the local echo and projection merge completed before
terminal delivery; its access log records the corresponding 30-second
long-poll requests disconnecting before response serialization. This is the
same transport/long-poll outlier as the earlier Synapse run, not an absent
local-echo projection
(`/tmp/umbrella-final-synapse-direct-retained.log`). Exact-head umbrella CI
run `34212970926` passes all 9 required jobs, including macOS Tauri check,
Windows IPC, both invitation lanes, browser/README gates, frontend, and Rust
workspace/wasm
(`https://github.com/shinaoka/koushi-matrix/actions/runs/34212970926`). The
post-fix fast send-queue integration lane passes all 7 tests, the QA binary passes 103 tests,
and the complete workspace/all-targets run passes 2,564 tests with zero
failures (`/tmp/umbrella-final-send-testkit-2.log`,
`/tmp/umbrella-final-send-qa-2.log`,
`/tmp/umbrella-final-workspace-send-all.log`). The subsequent full Core lib
run passes 1,011 tests / 9 ignored, and the two previously reported unused
`Result` warnings in search/thread teardown tests are now handled
(`/tmp/umbrella-warning-two-core-tests.log`).

Dependency-security follow-up: `cargo update -p plist` selected the latest
compatible `plist 1.10.1`, `quick-xml 0.42.0`, and its required `base64 0.23.1`
without changing the vendored SDK. `cargo audit` now exits 0 with only the
existing warning-level advisory set (including yanked `chacha20`), while
`cargo deny check`, `cargo machete`, the Core/SDK/state/desktop focused suites,
formatting, and `git diff --check` pass. The obsolete quick-xml exceptions were
removed from `deny.toml`; the change is committed as `5f06a429` and pushed for
fresh exact-head CI verification. A current `cargo audit --json` check found
that `event-listener 5.4.1` had a compatible fixed release, so the lockfile was
updated to `5.4.2`; the audit warning set no longer includes
`RUSTSEC-2026-0221`. The Core library suite passes 1,011/1,011 and the fast
send-queue lane passes 7/7 after this change (`/tmp/umbrella-cargo-audit-event-listener.json`,
`/tmp/umbrella-event-listener-core-test.log`,
`/tmp/umbrella-event-listener-send-test.log`,
`/tmp/umbrella-event-listener-deny.log`,
`/tmp/umbrella-event-listener-machete.log`). Remaining audit warnings are
transitive `bitmaps`/`glib`/`lru` unsoundness or unmaintained crates without a
safe workspace-level replacement; they remain an explicit upstream/SDK policy
blocker rather than being hidden by a broad ignore. The follow-up CI run
`34219287338` passed all
nine required jobs at submitted head `206fdd5f1ebae6d777f2341f8a82ab2c7791c8c3`.
Its only prior failure was the frontend `TimelineView.anchor-race` test timing
out under shared-runner load; the test was locally reproduced, its existing
assertions were retained, and an explicit 15-second test timeout was added in
`206fdd5f`. Focused and full frontend tests pass locally, and the current
worktree and remote branch are identical and clean. A final production-path
preflight audit traced `ReceiptReaders` → `TimelineItemRow` → `TimelineView`'s
committed `ReceiptSourceRef` through the six registered Tauri reader commands,
`CoreRuntimeState`/`ReaderSubscription`, `AppActor` bounded window/profile/resource
publication, and ACK/retirement; it also confirmed the App-level timeline store
is the sole reducer owner while TimelineView retains only viewport/side effects.
Evidence: `/tmp/umbrella-preflight-review-current.md`.

The docs-only no-local-echo contract clarification was pushed as `db673252`.
Exact-head CI `34239948809` then exposed one existing parallel-test isolation
failure in `scoped_receipt_window_prepares_only_its_selected_profiles`: four
sibling receipt-profile tests emitted the same diagnostic source without taking
the shared diagnostic test lock. They now all hold that lock, without changing
production behavior or relaxing the assertion; the focused read-state module
passes 38/38 and the full Core library suite passes 1,011/1,011
(`/tmp/umbrella-ci-342399-rust.log`,
`/tmp/umbrella-receipt-lock-module.log`,
`/tmp/umbrella-receipt-lock-core-final.log`).
Exact-head CI `34235700270` then failed only in the Synapse invitation job:
the SDK lane reached two successful join-operation traces and timed out in the
runner, while the other eight jobs passed; the uploaded SDK artifact contains no
application failure or changed assertion (`/tmp/umbrella-ci-342357-synapse.log`,
`/tmp/umbrella-ci-342357-artifact/`). This is recorded as a non-green CI
attempt, not treated as a pass or retried blindly.

The next exact-head CI `34241965442` exposed one browser-headless failure in
`timeline-scroll-anchor-drift.spec.ts`: the anchor-drift oracle timed out after
288 other browser tests passed. Local reproduction showed the helper's single
estimated-height jump could mount the overscanned row while the measured range
still moved it outside the viewport. The test now advances through bounded,
frame-separated scroll steps before asserting the real anchor, retaining the
same viewport assertion; the complete spec passes 4/4 and frontend typecheck
passes (`/tmp/umbrella-ci-342419-browser.log`,
`/tmp/umbrella-anchor-stepping-typecheck.log`). Exact-head CI
`34244924920` then passed all nine required jobs at `f29d8c03072d3ea808280402d1b2a9bad253ec07`,
including the browser-headless anchor suite and the updated event-listener lock.
The subsequent plan-only commit `fd676dfc` is also verified by exact-head CI
`34246880910`, which again passes all nine required jobs; the branch, remote,
and PR head match this commit and the worktree remains clean. This confirms
that the receipt-diagnostic serialization and anchor-stepping corrections are
stable in the full required matrix, while the native macOS, maximum-stress,
advisory-policy, review, and merge blockers above remain open. The resulting
plan commit `665f8c4e` was then verified by exact-head CI `34248745965`, again
with all nine required jobs successful.
