# Issue #759 ordered state transport and renderer-independent settlement

Status: proposed; implementation is blocked until the Fireworks-backed `reviewer-flash` records `CORRECT-TO-MERGE`.

## Outcome

Land one atomic protocol migration that leaves exactly one Rust-authored, ordered,
versioned application-state update path:

1. `StateDelta` is the normal update; a versioned full snapshot is used only for
   initial attach or explicit gap/lag resync.
2. Normal product commands return typed settlement/result DTOs, never a full
   application snapshot.
3. React applies state only from the initial/resync snapshot or the ordered update
   stream. Command promises cannot repair or replace product state.
4. Core navigation, replay and gap-repair progress never waits for DOM paint or a
   mounted renderer acknowledgement.
5. DOM measurement, anchoring, virtualization and viewport observations remain
   renderer-local presentation evidence.

This intentionally changes the desktop wire contract. It preserves Matrix/product
behavior and current serde casing except for the explicitly versioned state-update
and command-settlement envelopes introduced here.

## Measured current boundary

At `origin/main` `2591e106`:

- `CoreEvent` still exposes both `StateDelta(StateDelta)` and
  `StateChanged(AppStateSnapshot)` (`crates/koushi-core/src/event.rs`).
- `publish_state_delta` sends the versioned watch snapshot, emits the delta, then
  emits a legacy full `StateChanged`; snapshot-only Core changes also emit
  `StateChanged` at the current generation (`runtime.rs`).
- Tauri forwards normal deltas on `koushi-desktop://event`, emits a second
  `koushi-desktop://state` wake for session changes and lag, and React debounces
  that wake before calling `get_snapshot` (`core_event_forwarder.rs`, `App.tsx`).
- `appStore` accepts deltas but treats full command responses/refreshes as a second
  normal authority. The source explicitly documents newer command-response full
  snapshots as normal (`appStore.ts`).
- Fifteen Tauri command modules contain handlers returning
  `FrontendDesktopSnapshot`; App contains roughly 140 `setSnapshot` call sites,
  mostly applying command results. Composite submission/draft DTOs also embed a
  full snapshot.
- Focused navigation remains pending until
  `AcknowledgeTimelineProjection` returns browser target-presence evidence even
  though the TimelineActor independently computes the same fact from its
  authoritative display items.
- Gap repair stores `awaiting_projection` and will not inspect/continue until
  `AcknowledgeTimelineBatchRendered` arrives or a timeout requeues work.
- React owns a retry/backoff `timelineAcknowledgementDelivery` solely to keep
  those Core product paths moving across WebView command delivery.

#738 already supplies AppActor commit-point publication, generation-bearing
settlement, stale viewport epochs and flake measurement. #755 already supplies
Core-owned request outcome settlement. This plan reuses both; it does not add
another settlement framework.

## Normative protocol

### Core state ownership

`CoreConnection` retains its latest-wins
`watch::Receiver<VersionedAppStateSnapshot>` for initial attach, explicit resync
and Core-native consumers. The reliable normal state stream is
`CoreEvent::StateDelta` with contiguous `generation`.

Delete `CoreEvent::StateChanged`. Snapshot-only internal fields may still wake the
watch receiver at the current state generation, but they do not create a WebView
state event or fabricate a delta. Any field consumed by the frontend must be in
`StateDeltaChangedSlices`; the exhaustiveness audit remains the guard.

Core/headless tests and waiters that currently observe `StateChanged` must use the
versioned snapshot watch/predicate or the relevant typed event. No compatibility
variant or duplicate full-state broadcast remains.

### Desktop state-update envelope

Use one desktop state listener and one explicitly versioned envelope:

```ts
type StateUpdateEnvelope =
  | { protocol_version: 1; kind: "delta"; generation: number; changed: StateDeltaChangedSlices }
  | { protocol_version: 1; kind: "snapshot"; generation: number; snapshot: DesktopSnapshot;
      reason: "initial" | "gap" | "lag" | "settlement" };
```

The Tauri adapter serializes Core deltas into `delta`. Initial attach obtains one
`snapshot`. On event-broadcast lag the adapter emits one `snapshot` with the exact
`CoreConnection::versioned_snapshot()` generation on the same state-update lane,
then resets and requests timeline replay. A frontend-detected generation gap must
perform the same atomic recovery: admit one exact versioned snapshot, reset the
timeline projection cache, request `ReplaySubscribed`, and resume only from the
resync generation. AppState and timeline events share one lossy delivery lane, so
a state-generation gap is also evidence that adjacent timeline diffs may be lost;
state-only recovery is forbidden.

Delete `koushi-desktop://state`, `listenStateChanges`, the debounce timer and the
session-specific full-refresh side lane. Timeline replay/reset remains a separate
typed timeline event because timeline items are intentionally outside AppState;
it cannot carry or repair AppState.

The listener must be installed before initial snapshot admission. Deltas arriving
around attach are ordered by generation: stale/duplicate updates are ignored and
a genuine gap requests one resync. Unmount/disposal may drop all updates without
blocking Core.

### Command result contract

Add two small, closed wire DTOs rather than labeling queue acceptance as terminal:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontendCommandSettlement {
    protocol_version: u8, // exactly 1
    published_generation: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontendCommandAdmission {
    protocol_version: u8, // exactly 1
    admitted_generation: u64,
}
```

`Settlement` means the existing Core outcome predicate is terminal and visible at
that generation. `Admission` means Core accepted/routed the command and any
synchronous pending/admission projection is visible at that generation; later
actor work remains observable only through the state/event stream. Neither means
that the WebView has painted or even received the matching delta.

Classify every desktop command, without a fallback union:

1. **Initial/resync read** — returns `FrontendDesktopSnapshot`; only
   `get_snapshot`/the explicitly named state-resync read is allowed.
2. **Core-terminal state mutation/query** — returns
   `FrontendCommandSettlement` from an existing closed
   `RequestOutcomeExpectation` or specific Core API and its exact outcome
   generation. React may use rejection for fixed transport presentation but
   never applies state from the promise.
3. **Core-admitted asynchronous mutation** — returns
   `FrontendCommandAdmission` from a Core-owned command-envelope admission path.
   AppActor completes it only after routing and publishing any synchronous
   pending/admission projection. The actor's later terminal remains stream-only.
4. **Typed non-AppState result** — returns its existing result plus exact
   `settlement` or `admission` as classified below, while embedded `snapshot`
   fields are removed. Examples include submission IDs/outcomes, composer
   accepted revision, media bytes/save outcome, OIDC authorization and
   diagnostics.
5. **High-frequency observation/native/pure effect** — remains `void` or its
   existing tiny result; it does not read a snapshot merely to acknowledge queue
   acceptance.

A command that currently performs only `submit_core_command` followed by
`current_snapshot` must not invent a terminal predicate. It uses category 3
unless the table assigns an existing Core expectation/API. The admission oneshot
belongs to the Core command envelope/AppActor route, not a Tauri waiter. Queue
acceptance is never called settlement.

Adapter validation cannot fabricate a settlement/admission generation. Invalid
blank or empty command inputs that previously short-circuited by returning the
current full snapshot now reject with a fixed validation error before dispatch.
This deliberately makes those local no-ops explicit errors while preserving all
valid-input Core behavior; tests pin the error and prove no command is sent.

Command IPC and state events are separate delivery channels. A command promise
therefore cannot imply renderer visibility. One frontend helper consumes a
settlement/admission generation as a watermark: if appStore already reached it,
return; otherwise perform one explicit versioned snapshot resync with reason
`settlement`, apply it monotonically, and ignore any later stale delta. It does
not wait on a timer, retry/backoff, or DOM paint. Tests must cover both orders —
delta before promise and promise before delta — and prove the same final state.
A genuine observed generation gap uses the stronger state+timeline replay path
above.

Delete normal `current_snapshot` use from Tauri handlers and add a structure guard
whose allowlist contains only initial/resync entry points. Update
`SubmissionResponse` and `ComposerDraftAcceptanceResponse` to remove embedded
snapshots.

### Exhaustive DesktopApi migration map

The checked-in `DesktopApi` interface is the inventory; its structure test fails
when a method is absent from exactly one row below.

| Category | Methods | Core contract |
| --- | --- | --- |
| initial/resync snapshot | `getSnapshot`, `settlementSnapshot`, `resyncSnapshot` | versioned watch snapshot; settlement resync is state-only, while gap resync also resets/replays timelines |
| existing exact settlement | `discoverLoginMethods`, `completeOidcLogin`, `submitLogin`, `submitSoftLogoutReauth`, `switchAccount`, `logout`, `selectRoom`, `openTimelineAtTimestamp`, `closeFocusedContext`, `openActivityEvent`, `openPinnedEvent`, `selectSearchResult`, `createRoom`, `createSpace`, `startDirectMessage`, `joinRoom`, `acceptInvite`, `declineInvite`, `openInviteWorkflow`, `closeInviteWorkflow`, `searchInviteTargets`, `setInviteScope`, `selectInviteTarget`, `removeInviteTarget`, `inviteTargets`, `queryDirectory`, `previewJoinTarget`, `joinDirectoryRoom`, `submitSearch`, `closeSearch`, `loadRoomSettings`, `loadSpaceMembers`, `inviteUserToSpace`, `cancelSpaceInvite`, `updateRoomSetting`, `moderateRoomMember`, `updateRoomMemberRole`, `updateSpaceMemberRole`, `inviteUser`, `setRoomTag`, `removeRoomTag`, `pinEvent`, `unpinEvent`, `forceRotateOutboundSession`, `stageUploadBytes`, `selectStagedUploadOutput`, `retryStagedUploadPreparation`, `useOriginalStagedUpload`, `updateStagedUploadCaption`, `updateStagedUploadCompression`, `clearUploadStaging` | reuse the matching current `RequestOutcomeExpectation`/specific Core API: AuthDiscovery, Authenticated, SignedOut, RoomSelected, FocusedContext, MainTimelineAnchor, RoomCreated, SpaceCreated, DirectMessageStarted, RoomJoined, InviteWorkflow/InviteBatch, Directory, Search, RoomOperation, UploadStaging, ComposerAccepted, Submission, PreparedMediaQueued; add no generic string predicate |
| Core admission generation | `retrySlidingSyncCapability`, `changeHomeserver`, `submitRecovery`, `startDeviceCleanup`, `submitDeviceCleanupUia`, `eraseLocalDataAnyway`, `restartSync`, `updateSettings`, `rebuildSearchIndex`, `setRoomUrlPreviewOverride`, `dismissDirectoryPreview`, `selectRoomListFilter`, `markRoomAsRead`, `markRoomAsUnread`, `setRoomNotificationMode`, `refreshCurrentSessionStatus`, `submitAccountManagementUia`, `loadAccountManagementCapabilities`, `changePassword`, `deactivateAccount`, `probeLocalEncryptionHealth`, `resetLocalData`, `bootstrapCrossSigning`, `enableKeyBackup`, `exportRoomKeys`, `importRoomKeys`, `bootstrapSecureBackup`, `recoverSecureBackup`, `retrySecureBackupInspection`, `changeSecureBackupPassphrase`, `acceptVerification`, `startOwnUserSas`, `retryCurrentDeviceTrustDiscovery`, `mismatchSasVerification`, `startSessionBootstrap`, `confirmSessionBootstrapSaved`, `confirmSasVerification`, `cancelVerification`, `resetIdentity`, `cancelIdentityReset`, `submitIdentityResetPassword`, `submitIdentityResetOAuth`, `selectSpace`, `reorderSpaces`, `cancelScheduledSend`, `rescheduleScheduledSend`, `retrySend`, `cancelSend`, `sendReaction`, `redactReaction`, `setPresence`, `setDisplayName`, `setLocalUserAlias`, `ignoreUser`, `unignoreUser`, `reportUser`, `reportContent`, `reportRoom`, `setAvatar`, `editMessage`, `redactMessage`, `loadMessageSource`, `requestRoomKey`, `requestLateDecryption`, `forwardMessage`, `loadLinkPreviews`, `hideLinkPreview`, `leaveRoom`, `forgetRoom`, `openActivity`, `closeActivity`, `setActivityTab`, `paginateActivity`, `retryActivityResolution`, `markActivityRead`, `setComposerDraft`, `openThread`, `closeThread`, `openThreadsList`, `closeThreadsList`, `paginateThreadsList`, `openFilesView`, `closeFilesView`, `setThreadComposerDraft`, `startRoomCrawl`, `stopRoomCrawl`, `repairRoomTimeline`, `setSpaceChild`, `setComposerReplyTarget`, `cancelComposerReply` | Core envelope admission oneshot completed after route plus synchronous publication; terminal actor work stays stream-only |
| typed result + settlement/admission | `startOidcLogin` (typed OIDC result + existing OidcAuthorization settlement), `sendText`/`sendReply`/`sendThreadReply` (submission result + settlement), `scheduleSend`/`sendPreparedUploads` (accepted revision/result + settlement) | remove embedded snapshot; preserve existing typed payload and exact Core outcome |
| pure/native/observation | `getDiagnosticSnapshot`, `listSavedSessions`, `observeViewportSync`, `resolveComposerKeyAction`, `beginComposerDraftRendererGeneration`, `acquireComposerDraftLease`, `releaseComposerDraftLease`, `preparedUploadPreview`, `sendReadReceipt`, `setFullyRead`, `setTyping`, `queryMentionCandidates`, `setRoomListProjection`, `subscribeReceiptReader`, `receiveReceiptReader`, `updateReceiptReaderWindow`, `ackReceiptReader`, `readReceiptReaderResource`, `closeReceiptReader` | existing typed value or `void`; no state snapshot and no fabricated generation |
| removed renderer ACK | `acknowledgeTimelineProjection`, `acknowledgeTimelineBatchRendered` | delete from interface/client/fake/Tauri/Core; replaced only by the two named internal Core commit signals |

Each method appears in exactly one row. Typed-result rows determine both the wire
return shape and the exact existing outcome expectation. Any implementation discovery that a
listed expectation lacks the command's exact account/target/request guard stops
that slice for a design amendment; it may not broaden an expectation silently.

### Browser fake and tests

The browser fake must implement the same update contract rather than remain a
second snapshot-return protocol. It retains its private Rust-shaped fake state.
At a fake command terminal it advances one generation and emits a `delta`
envelope; emitting all changed top-level slices is acceptable for the fake and is
simpler than a speculative generic JSON differ. Initial attach emits one
`snapshot`. Tests may inject explicit delta/snapshot envelopes through the same
port.

No production or test helper may call `setAppStoreSnapshot` in response to a
normal command return. Test harness helpers named `pushStateChanged` are renamed
to the versioned envelope they actually emit.

## Renderer-independent timeline progress

### Focused navigation

The TimelineActor already owns authoritative projected items and computes
`target_present`, but no reverse actor/manager/AppActor commit route exists today;
the WebView ACK round-trip is the only route. Add one narrow internal
`FocusedProjectionCommitted` signal from TimelineActor through its existing
manager/account ownership chain to AppActor when the matching `InitialItems`
generation is accepted. It carries only request/key/actor/timeline generation,
item count and target-present facts, and is reliable within the actor tree.
AppActor settles `pending_focused_navigation` from this Core-internal projection
result:

- exact request/key/actor/timeline generation and target present → anchored state;
- exact accepted projection with target absent → existing live fallback;
- stale actor/request/generation → ignored;
- replacement/close/shutdown → existing typed terminal.

The emitted timeline rows may arrive at zero renderers. That has no effect on
navigation settlement. Delete browser `item_count`/`target_present` command input
as product evidence. Renderer-local diagnostics may compare DOM rows to the
projected event but cannot send an admission signal back to Core.

### Gap repair

A repaired batch becomes observable for Core scheduling when the Core
relay/display projection accepts and emits the exact actor/timeline/repair/batch
generation. No success signal currently returns from relay to the repair
scheduler, so add one narrow `GapProjectionRelayed` TimelineActor message at the
existing relay acceptance point. Fence it by actor/timeline/repair/minimum batch
generation, then continue bounded inspection from that internal settlement.
Do not store `awaiting_projection`, schedule a render-settlement timeout, or wait
for `AcknowledgeTimelineBatchRendered`.

Viewport observations remain typed demand facts: current visible range, visible
gaps and at-bottom state may prioritize automatic work. Missing observations mean
no renderer-specific foreground demand; they never strand already admitted
manual/live-edge/committed repair. Existing network/batch/no-progress limits stay
unchanged.

Delete end to end:

- `AppCommand::AcknowledgeTimelineProjection` and
  `AcknowledgeTimelineBatchRendered`;
- account/timeline actor acknowledgement messages and Tauri commands;
- `DesktopApi` acknowledgement methods;
- `timelineAcknowledgementDelivery` and its retry/backoff tests;
- TimelineView/App acknowledgement submission and generation identity state.

Keep local projection/layout settlement state that controls anchoring,
virtualization, pagination admission or DOM measurement; it simply has no Core
command side effect.

## Verify-first phases

### Phase A — state stream RED, then Core/Tauri cutover

Amend `docs/architecture/overview.md`, `docs/architecture/state-machine.md`,
`docs/architecture/frontend-ownership-inventory.md`,
`docs/agents/state-ownership.md` and `docs/policies/engineering-rules.md` first so the new state-update, command-watermark
and renderer-independent projection contracts are canon before production edits.
Then write failing tests:

- Core action publication emits exactly one contiguous delta and no full-state
  event.
- snapshot-only Core watch refresh does not fabricate a generation/delta.
- Tauri lag emits exact generation-bearing snapshot on the state-update lane and
  timeline replay/reset once.
- listener-before-initial ordering accepts stale/duplicate, detects a genuine gap,
  resyncs once and resumes contiguously.
- a frontend-detected state gap that also drops a timeline diff admits one state
  snapshot, resets the timeline store and requests one timeline replay.

Then delete `StateChanged`, the state wake URI/listener/timer and session refresh.
Update Core/headless consumers to the watch or typed event.

### Phase B — command contract RED, then atomic API cutover

Add a structure test that fails while any normal Tauri handler returns or builds
`FrontendDesktopSnapshot`, with only initial/resync allowlisted. Add TS contract
checks that normal `DesktopApi` methods do not return `DesktopSnapshot`, require
every interface method in the exhaustive map above, and pin every existing
`RequestOutcomeExpectation` mapping/guard/lag policy used by category 2/4. DTO
wire tests pin protocol version/casing. Add appStore/command-watermark tests for
both delivery orders: delta-before-promise and promise-before-delta; the latter
performs exactly one monotone settlement resync and a later delta is stale.

Migrate Tauri handlers, TS client/interface, composite result DTOs, browser fake,
App handlers and focused tests. Remove all normal command-result `setSnapshot`
paths and stale/latest-click gates whose only purpose was deciding whether a
returned snapshot could enter appStore. Keep presentation epochs that gate dialog
opening, chooser results or fixed local error display.

### Phase C — focused navigation RED, then internal settlement

Before changing production code, prove:

- accepted Core projection settles anchored navigation with no renderer attached;
- target absence settles live fallback with no renderer attached;
- stale/reordered internal projection cannot settle a newer navigation;
- dropping the desktop event consumer cannot delay the published AppState.

Create and generation-fence the new reliable `FocusedProjectionCommitted`
reverse signal, move settlement to it, then delete the projection ACK route. The
RED test must fail before that signal exists; event-broadcast observation is not
a substitute.

### Phase D — gap repair RED, then remove render gate

Before changing production code, prove:

- admitted repair continues after exact relay/display projection with no renderer;
- stale relay/batch generations cannot advance repair;
- lag/replay and bounded no-progress behavior remain deterministic;
- viewport replacement/unmount cannot strand an admitted repair.

Create and generation-fence the new `GapProjectionRelayed` scheduler signal,
then delete render ACK state/timeouts/routes and the frontend delivery controller.
Preserve current network, batch, actor-generation and causal relay fences.

### Phase E — canon, deletion audit and integrated gates

Update generated wire artifacts and verify the Phase A canon against the final
implementation. Record the intentional v1 envelope/camelCase evidence. Add source
structure guards for:

- no `CoreEvent::StateChanged`;
- no normal snapshot-return command;
- no `koushi-desktop://state`/`listenStateChanges`;
- no timeline projection/render acknowledgement command or frontend retry owner;
- no normal command-result `setSnapshot`.

Run focused RED→GREEN checks per phase before broad gates. Final verification is
CI-equivalent: Rust workspace, Tauri, wasm, QA binary, DTO/wire goldens,
structure-checker tests, typecheck, lint, full Vitest, build, Playwright, secret
and boundary checks, cargo-deny/machete, rustfmt and `git diff --check`. Hosted
macOS/Windows and both homeserver lanes remain merge gates.

## Atomicity and compatibility decision

This is one PR because producer, adapter, consumer and fake must agree on one
wire at every commit. Temporary old/new listeners, union return types, fallback
snapshot application, feature flags and adapter shims would preserve the exact
duplicate authorities this issue removes.

Wire protocol v1 deliberately uses the repository's current Rust/Tauri DTO field
casing (`snake_case` for existing state DTO fields, explicit `camelCase` only on
the new command-settlement DTO where pinned by its Rust serde contract). Existing
Core event variant payload casing is unchanged. No global casing cleanup is
included.

## Explicit exclusions

- No generic event bus, command framework, persisted cursor, replay log or second
  state store.
- No automatic retry for required CI and no timeout inflation.
- No movement of DOM measurement, anchoring, virtualization, focus or animation
  into Rust.
- No removal of viewport observations that are genuine current renderer facts.
- No Matrix behavior, SDK sync ownership, settings semantics (#761), public
  protocol crate extraction (#763), or remaining leaf-crate cleanup (#765).
- No compatibility shim or TODO left for removed state/ACK paths.

## Acceptance mapping

- **One ordered stream / snapshots initial-resync only:** Phase A envelope and
  deletion guards.
- **Missing/stale/unmounted renderer cannot delay Core:** Phase C and D headless
  no-renderer tests.
- **Gap/lag/lost/reordered and viewport behavior:** appStore generation tests,
  Tauri lag replay tests and Core stale-fence tests.
- **No browser retry/backoff or WebView settlement vocabulary:** ACK stack and
  delivery-controller deletion.
- **No DOM paint or Tauri URI semantics in Core protocol:** internal projection
  settlement plus adapter-only state envelope.
- **No incidental wire break:** explicit v1 DTO artifacts and preserved existing
  casing.
