# Frontend Semantic Ownership Inventory (#552)

Status: evidence inventory as of `origin/main` `dfaec0753461e168c273916d1f3340e0255d8a2e` (includes merged #660). This document classifies current owners; it does not itself migrate state or close #552.

Pinned epic contract: [`evidence/issue-552-contract.json`](evidence/issue-552-contract.json), SHA-256 `0371538cb18ab90b399fbd8114ec0678603ef3d24797e3f70d182898910c268f`, from <https://github.com/shinaoka/koushi-matrix/issues/552>.

## Classification key

- **Rust product/domain** — durable Matrix/product semantics or backend resource lifetime. React renders a projection and sends typed intent.
- **Renderer presentation** — DOM geometry, focus, transient overlays, immediate feedback, or view-local scheduling. Keep in React.
- **Transport/projection cache** — ordered copy of Rust snapshot/events, including gap/stale detection. Keep as a cache; it is not another semantic owner.
- **Test-backend mirror** — Browser Fake/harness behavior used to test the Rust contract. It is not production authority, but must stay equivalent and bounded.

Decisions: **keep**, **derive/delete**, **migrate leaf**, or **investigate**. A long-lived ref is not a migration target by size alone.

## Inventory

| Site / current owner | Lifetime and disappearance | Classification / authority | Settlement and duplicate semantics | Decision |
| --- | --- | --- | --- | --- |
| `domain/appStore.ts::applyAppStoreDeltas` | Page lifetime; reset on account/session snapshot clear | **Transport/projection cache**. Rust `StateDelta.generation` and snapshots are authoritative. | Drops duplicate/stale generations, requests full refresh on a forward gap; never invents domain transitions. | **Keep.** Already-correct #111 projection cache. |
| `domain/timelineStore.ts::TimelineStoreState`, `applyInitialItems`, `applyItemsUpdated` | Page lifetime, bounded inactive keys; keys clear on resync/account reset | **Transport/projection cache** for Rust `CoreEvent::Timeline`. Derived ID/timestamp indices are renderer acceleration only. | Actor/generation/batch fences reject stale diffs; pagination/gap fields copy Rust states. | **Keep.** Do not move maps/indices into Rust or reimplement the transport. |
| `TimelineView.tsx` mounted viewport controller (`pendingMeasuredHeightsRef`, anchors, range epochs, observers, frames) | Mounted timeline key; ordered teardown on key change/unmount | **Renderer presentation**. DOM measurement, virtualization, scroll anchors and visible-range facts have no backend owner. | ResizeObserver/frames/timers cancel at one key reset; Rust receives typed viewport facts. | **Keep.** #551 residual audit proves this cohesive DOM owner. |
| `TimelineView.tsx::projectionAcknowledgementRetryRef` / `repairAcknowledgementRetryRef` | Mounted timeline key; timers cancel on key reset/unmount | Mixed: DOM evidence is **renderer presentation**; retry/backoff and terminal delivery correspond to a Rust actor waiting for ACK. | Signature/in-flight fences, six-capped attempt counter with unbounded capped-delay reconstitution while mounted, command promise settlement. Owner disappears on unmount and cancels retries. | **Migrate-leaf candidate 2.** Keep evidence capture in React; investigate moving reliable retry/terminal ownership to transport/Rust. |
| `TimelineView.tsx::pendingKeyRequests`, `keyRequestEpochRef`, `keyRequestToast` | Mounted key/account; reset on timeline-key change and Rust terminal DTO | **Renderer presentation/investigate**, not product admission. Rust owns `DecryptRetryController::admit`, `begin_decrypt_retry`, `handle_request_room_key`, and `TimelineActor.key_request_states`. | Frontend Set suppresses pre-projection duplicate dispatch and handles delayed rejection/toast; Rust already coalesces same event/generation and owns terminal state. | **Keep for now / investigate.** No proven Rust semantic gap; do not migrate merely because it is a Set. |
| `TimelineView.tsx` avatar relevance/request/retry refs, App `requestedMemberAvatarMxcsRef`/`memberAvatarRetryCountsRef`, and `domain/avatarThumbnails.ts` | Mounted virtual/member window/key; clears with key/reset | **Renderer presentation** around a Rust-owned download command/cache. Relevance is DOM-window-specific. | Two-attempt request fence, retry release on typed event/failure, one shared teardown per surface. | **Keep.** #551 audit found no non-overlapping owner API. |
| `TimelineView.tsx` backfill epochs/evaluation/ref fences | Mounted key; cancels with projection/layout reset | **Renderer presentation** for when geometry warrants asking. Rust owns pagination operation/end state and SDK task. | Prevents repeated DOM-triggered requests until layout/projection settles; no Matrix history semantics synthesized. | **Keep.** Revisit only with a whole viewport-controller redesign. |
| `App.tsx::latestTextMutationQueueRef` / `applyLatestTextMutationSnapshot`, using `domain/latestAsyncResult.ts::createLatestMutationOperationQueue` | Page lifetime, keyed text mutations | **Partial migration.** Alias and main/thread caption mutations still require renderer serialization; invite and mention queries no longer use this queue. | Alias/caption A/B/A and invalidation retain latest-wins mutation admission; invite/mention dispatch every typed query and admit only Rust/appStore snapshots by their existing destination/request/generation fences. | **Migrate-leaf candidate 1 (Wave C, partial shipped).** Keep only the mutation queue; the invite/mention query semantic owner is now Rust request/generation state plus the monotone appStore fence. |
| `App.tsx` room/space settings/navigation/member request refs | Page lifetime; manually incremented on navigation/close | **Renderer presentation / transport fence** around async command responses. Rust request IDs, demand generations and StateDelta ordering are authoritative. | Delayed promise result is ignored when local request ref/selection no longer matches. | **Candidate 3: investigate derive/delete.** Prove generation admission covers each path before removing; #582 may change Space-member fields and remains unmerged. |
| `App.tsx` search debounce timer and query drafts | Dialog/view lifetime | **Renderer presentation** (typing draft and debounce of user intent). Rust owns search request/result correlation and crawler. | Timer clears on query/view changes; no durable retry or result semantics. | **Keep.** |
| `App.tsx` `pendingRoomLeave`, leave/confirm/dialog state, widths, pointer listeners, focus timers | Overlay/gesture lifetime | **Renderer presentation**. Matrix membership and operation state stay Rust-owned. | Explicit cancel/unmount cleanup; in-flight guard prevents accidental repeated UI intent. | **Keep.** Accessibility basics remain frontend-owned. |
| `App.tsx` composer overlays + debounce handles + `typingSignalRef` | Renderer/key lifetime, released on account/target/revision transitions | **Renderer presentation** over Rust `ComposerDocument`, revision, store, lease and typed-intent authority. | IME-safe local draft overlay settles only against accepted Rust revision; typing ref dedupes renderer intent; timer/overlay teardown is renderer-local. | **Keep.** Do not move DOM/input buffering. |
| `App.tsx::composerDraftLifecycleRegistryRef` | Page renderer generation; leases acquired/released through typed backend | Shared resource boundary: frontend owns renderer handle, Rust owns lease validity/account/target persistence. | Awaited acquire/release, generation replacement, #657 harness mirror cleanup. | **Keep.** One owner exists on each side of the typed lease boundary. |
| `App.tsx::submissionRegistryRef` and send overlays | Page/account/target; clears/settles from Rust submission IDs | Immediate presentation controller; Rust global submission registry/terminal state is authoritative. | Prevents local double UI settlement and preserves IME draft; terminal comes from Rust. | **Keep / audit only if duplicate transition is demonstrated.** |
| `App.tsx` State event/Core event/Tauri menu listeners + `stateRefreshTimerRef` | Page/runtime transport lifetime | **Transport resource owner** in the renderer. | Each effect/module listener has cleanup; refresh timer coalesces event gaps into authoritative snapshot fetch. | **Keep.** Backend task lifetime remains Rust-owned. |
| `App.tsx` QA send refs, diagnostics request generations, module error listeners | QA/page lifetime only | QA presentation/observability, not product state. | Reset by QA flow/page; privacy-safe diagnostics. Module error listeners overlap boot capture defensively. | **Keep; low-priority deletion audit** for duplicate error listeners, not a Rust migration. |
| `App.tsx` secure-backup retry in-flight ref and other button guards | View lifetime | **Renderer presentation** while Rust operation state is authoritative. | Avoids repeated click before snapshot; terminal/failure comes from Rust. | **Keep unless a reproducible duplicate command escapes Rust admission.** |
| `backend/browserFakeApi.ts` composer leases/draft maps/prepared bytes/submission ledger | Browser Fake instance/page | **Test-backend mirror** of Rust contracts. Not production state. | Bounded/reconciled by fake session/target generation; fixtures emulate terminal results. | **Keep as mirror; never cite as migration target.** Drift is test debt fixed against Rust. |
| `backend/browserFakeApi.ts` Activity/Space-member/search/settings local transitions | Browser Fake instance | **Test-backend mirror**, some duplicated state-machine logic intentionally required for browser tests. | Must install Rust-shaped snapshots and reproduce request/generation/failure guards. Active #570/#582 designs are unmerged and are not recorded as shipped. | **Keep and reduce duplication only in each reviewed contract migration.** |
| `apps/desktop/src/test/appHarnessMain.tsx::preparedUploadBytes`, `composerLeases`, invocation history | One Playwright harness page | **Test harness resource mirror**. #657 added snapshot/account/target reconciliation and boot history boundary. | Bytes and leases retire on authoritative replacement; invocation history has one boot boundary. | **Keep.** Already-correct reviewed lifecycle, not product state. |
| Pure dialogs, hover/focus/animation, alias drafts, media-viewer focus | Component mount/overlay | **Renderer presentation** | React cleanup and accessibility lifecycle only. | **Keep.** |

## Already-correct Rust-owned paths

The following are not migration work:

- Application/session/settings/invite/Activity/Space-member state in `koushi-state::AppState`; React consumes snapshots/deltas (`docs/agents/state-ownership.md`, “The boundary”).
- Composer persistence, revision, lease admission and send/submission terminals in Core/state. Frontend overlays are IME/render-local.
- Timeline SDK actors, pagination, repair, thread attention, read-state outbox, media tasks and room-key recovery in Core. `timelineStore` is a projection reducer, not the SDK owner.
- Room-key request admission/coalescing (`DecryptRetryController::admit`, `begin_decrypt_retry`, `handle_request_room_key`) and per-event projected request state (`TimelineActor.key_request_states`).
- Search crawler, directory, room/Space operations and current-session status. App request refs only fence stale renderer promises.
- Harness cleanup from #657, invite admission from #658, composer-load evidence from #645, KaTeX admission from #668, and transient projection/trust-loss reset contracts from #660.

## Duplicated semantics requiring evidence before change

1. **Latest text operation ordering** — TS queue vs Rust composer revision/snapshot generation. This is the selected leaf because deleting the duplicate would leave one existing Rust authority.
2. **Projection ACK retry/backoff** — frontend owns reliable-delivery policy while Rust owns actor terminal waiting. DOM evidence must remain frontend; transport retry may move.
3. **App promise request refs** — may duplicate Rust request/demand generations, but each command-response path needs an equivalence test before deletion.
4. **Browser Fake transitions** — intentional test mirror, not production duplication. Change only alongside the corresponding Rust contract.

Room-key `pendingKeyRequests` is excluded from this list until a semantic gap is proven: Rust already owns operation admission/coalescing; the Set is optimistic presentation and dispatch suppression.

## Ranked disjoint leaf candidates

### 1. Retire invite/mention query admission from the latest-text queue (Wave C leaf, partial)

- **Value:** removes the second “which async result wins” semantic owner from App for invite and mention queries while preserving the queue where it serializes unversioned mutations.
- **Proof required:** delayed invite and main/thread mention A/B/A dispatch, adversarial settlement, explicit monotone nonzero snapshot generations, rendered final projection, and account/room/dialog replacement fences. Alias/caption A/B/A serialization and invalidation stay green.
- **Scope:** `App.tsx`, `domain/latestAsyncResult.ts`, focused App/latestAsync/appStore tests, and this inventory/plan; no new dependency, Rust/Tauri API, fake semantic, or Rust abstraction.
- **Current result:** invite target search and main/thread mention query admission are migrated to existing Rust request/generation state plus `appStore`; alias and main/thread caption mutation serialization remain renderer-owned pending a separate reviewed Rust contract.
- **Disjointness:** #659 changes room-list reducer admission; #608 auth invalidation diagnostics/copy; #559 read-state local/server boundaries; #570 Activity/redaction/thread convergence. None share this query leaf.

### 2. Move projection/repair ACK retry policy to a reliable transport owner

- **Value:** a mounted view currently owns backoff/attempt terminal policy for a Rust actor resource.
- **Boundary:** React still computes committed DOM evidence and sends one typed observation. Tauri/Core owns reliable retry, cancellation and actor-generation settlement.
- **Risk:** cross-file actor/transport design; larger than candidate 1.

### 3. Retire redundant App request refs per command family

- **Value:** remove local stale-result fences already represented by Rust request IDs/generation and appStore admission.
- **Method:** one family per PR, exact delayed-result test, no generic request manager.
- **Risk:** some refs protect purely local selection/dialog lifetime and should remain.

Low priority: consolidate duplicate QA error listeners after a behavioral boot-error proof. This is deletion, not Rust migration.

## Active designs, not shipped behavior

- #570 redaction/edit convergence: umbrella split; Task A hit the redaction-before-target stop condition and is not shipped.
- #582 Space role management: design/implementation active; current main still lacks the role control.

## Disjoint issue contracts

- #659: fail closed before late room-list readiness/invites/rooms/spaces mutation.
- #608: classify UnknownToken authentication invalidation separately from E2EE trust and update locked UI copy.
- #559: split local viewed boundary from server-confirmed read state and bound persistent retry/outbox behavior.
- #570: redacted/edit convergence in Activity/unread/thread/conversation projections.

The shipped invite/mention query leaf touches none of those owners; alias/caption mutation serialization remains the separate retained owner in App.

## #552 acceptance status

| Epic criterion | Status after this inventory |
| --- | --- |
| Publish evidence-based inventory/classification | **Complete in this document** after merge. |
| Identify already-correct Rust-owned/projection-only paths | **Complete** above. |
| Identify duplicated Rust/TS semantics | **Complete as candidates**, each still needs task-level proof. |
| Migrate selected high-value leaf owners incrementally | **Partial:** invite target and main/thread mention query admission migrated; alias/caption mutation queue remains for a separately reviewed leaf. |
| One documented semantic owner per migrated subsystem | **Complete for invite/mention queries:** Rust request/generation state plus `appStore` snapshot generation; mutation fields retain their explicit renderer queue owner. |
| Async Rust owners have cancellation/awaited settlement where required | **Partially shipped** via #550/#551 audits; this leaf changes only App query dispatch and uses existing Rust ownership. |
| Remove corresponding TS semantic state after cutover | **Partial:** invite/mention keys and invalidation are removed from the mutation queue; alias/caption keys remain intentionally. |
| Frontend cleanup primarily renderer-local | **Current invariant**, verified for kept rows. |
| Preserve Tauri command/event compatibility unless separately reviewed | **Complete for this leaf:** no Rust/Tauri command or DTO changes. |
| Focused transition/teardown/projection-equivalence tests | **Complete for this leaf:** deterministic deferred App tests cover invite/main/thread queries, monotone snapshots, adversarial settlement, and replacements. |
| Compatible with Tauri UI and future native Rust renderer | **Current architecture supports it**; duplicate semantic owners remain the epic work. |

#552 stays open. One inventory and one future leaf do not satisfy the migration epic.
