# Rust-Owned State Boundaries

Per-area reference for who owns what. Open the area you are touching; you do not
need the rest. The normative architecture canon is
[docs/architecture/overview.md](../architecture/overview.md) and
[docs/architecture/state-machine.md](../architecture/state-machine.md) — this
file records the operational detail and the mistakes that have actually been
made.

## The boundary

Product state is Rust-owned. React renders Rust DTOs and dispatches typed
commands. React must not mutate product state, synthesize Matrix semantics,
repair command results locally, or infer success from a click. Visible change
happens only after a Rust-shaped snapshot or event says so.

React may own transient DOM input drafts, popup/dialog visibility, refs that
suppress duplicate command dispatches, and browser resource handles tied
strictly to one mounted presentation owner. Listeners, observers, animation
frames, and timers must be cancelled by the same effect/controller on logical
key change and unmount. Pending operations, retries/backoff, correlation,
session cleanup, SDK subscriptions, and background task ownership remain Rust
actor state.

A non-Tauri renderer consumes the same `koushi-protocol` Rust boundary: start
`CoreRuntime`, attach a connection, allocate connection-scoped request IDs,
submit typed `CoreCommand`s, observe `CoreEvent`/versioned snapshots, drop
consumers, then await `CoreRuntime::shutdown`. Tauri-only serde DTO mirrors,
native artifact paths, and custom thumbnail URI minting stay in
`apps/desktop/src-tauri`; do not move them into Core or protocol.

## Session authentication invalidation

Rust distinguishes an authenticated E2EE trust lock from Matrix authentication
invalidation. `SessionState::Locked` is paired with the Rust-owned
`session_lock_reason`: `DeviceTrust` keeps verification/recovery copy, while
`UnknownToken { soft_logout }` shows expired/revoked authentication copy and a
Sign out action. React must not infer this reason from timing, a generic trust
recheck failure, diagnostics, or sync state. The SDK classifies trust recheck
errors from structured facts; only `SessionChange::UnknownToken` dispatches the
authentication-invalidation action. The optional reason's state-delta mirror is
nested (`Option<Option<_>>`) so an explicit null clears the frontend projection.

## Snapshot and wire-contract mirrors

The Tauri snapshot is a **hand-maintained DTO**
(`apps/desktop/src-tauri/src/dto.rs`, `FrontendAppState` / `From<AppState>`), not
a passthrough of `AppState`. When `AppState` gains a field, the DTO must be
extended in the same change, or the serialized snapshot silently omits it and the
React UI crashes the moment it reads the missing field. Symptom: clicking a
control blanks the WebView and `window.onerror` reports `undefined is not an
object (evaluating 'e.state.basic_operation.kind')`. Headless tests that use the
browser fake or mock IPC will NOT catch this — they build their own snapshots.
Only the real Tauri lane or the `dto.rs` serialization-contract test does.

When an `AppState` field, DTO, or command/event variant changes, check every
applicable mirror below and update affected surfaces in the same change.
An unchanged surface needs no mechanical edit:

1. `crates/koushi-state` state/action/reducer and the public
   `crates/koushi-protocol/src/{command.rs,event.rs,state_update.rs}` contracts
2. `apps/desktop/src-tauri/src/dto.rs`, `dto/`, and serialization-contract tests
3. `apps/desktop/src/domain/types.ts`
4. `apps/desktop/src/domain/coreEvents.ts`
5. `apps/desktop/src/domain/coreEvents.generated.json`
6. Relevant explicit transport fixtures under `apps/desktop/src/backend/browser/`
7. `apps/desktop/src/test/tauriIpcMock.ts`
8. `apps/desktop/src/test/appHarnessMain.tsx`
9. Any Rust/TS fixture that constructs the changed struct

The core-event wire-contract test lives in
`apps/desktop/src-tauri/src/core_event_forwarder/tests.rs`; timeline item DTO
fields must keep it in sync with `coreEvents.ts` and `coreEvents.generated.json`.

Headless browser mocks and browser fakes do **not** inherit Rust snapshot fields
automatically. The real WebView consumes the Tauri DTO while headless tests often
consume the TypeScript fakes, so updating only one side leaves a green browser
tier and a crashing Tauri lane.

A snapshot field change can affect two checked-in artifacts, with different
update procedures (this section owns those procedures):

- `apps/desktop/src-tauri/tests/golden/frontend_app_state.json` is rewritten by
  `UPDATE_GOLDEN=1 cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib frontend_app_state_golden`.
- `apps/desktop/src/domain/coreEvents.generated.json` is regenerated from the
  Rust serialization exercised by the wire-contract test:
  `UPDATE_CORE_EVENT_GOLDEN=1 cargo test -p koushi-desktop --lib core_event_wire_format_matches_checked_in_contract_artifact`.
  Update the Rust contract cases for the intended shape first, regenerate rather
  than hand-editing JSON, inspect the resulting diff, then rerun the test without
  the update variable. The update run writes the artifact and returns before
  the equality assertion; it is not verification by itself.

Likewise, rerun the frontend snapshot golden test without `UPDATE_GOLDEN` after
regeneration. Neither update switch substitutes for checking the intended shape.

Populate the golden fixture with data that exercises the new shape. An empty
array or a `None` proves nothing a scalar field would not also satisfy, so a
list-valued or optional field needs a real value in the maximally-populated
state.

Do not hand-write a TypeScript shape that is not proven by the Rust contract
artifact.

Focused checks:

```bash
cargo test -p koushi-state --test core_batch_a_state
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
npm --prefix apps/desktop run typecheck
```

Browser tests do not own reducer guards. They use the typed Tauri transport mock,
assert the submitted command, and inject an explicit Rust-shaped snapshot/event
for the resulting state. A command receipt alone must leave the visible state
unchanged. Do not add another stateful `BrowserFakeApi`; Rust behavioral tests
cover reducer/actor guards, while frontend tests cover rendering and transport.

Focused check: `npm --prefix apps/desktop run test -- --run src/test/tauriIpcMock.test.ts`.

## Private-data-free projections

Snapshots, `Debug` output, logs, QA artifacts, screenshots, and issue comments
carry tokens and counts only. The full prohibited list is in
[qa-lanes.md](qa-lanes.md#output-must-be-private-data-free).

- `ProfileState` Debug exposes only profile/avatar presence and counts; SDK alias
  DTO Debug exposes counts only.
- `SetAvatar` may carry image bytes only through the typed command boundary.
- `RoomSendQueueUpdate::SendError` carries raw SDK errors: project only coarse
  recoverable/unrecoverable status into DTOs and QA tokens. Structured release
  diagnostics may additionally carry a closed app-owned failure-class token
  such as `secure_backup_required`, `http`, `crypto`, or `store`; they never
  carry the raw SDK error, endpoint, identifier, or response body.
- `koushi-sdk` maps SDK cross-signing/backup states into private-data-free
  `koushi-state` DTOs and redacts SDK error details in `Debug`.
- Local aliases are private "only I see this" data.
- Rust/Tauri own diagnostic records, bounded retention, dropped counts and the
  privacy-safe `FrontendDiagnosticLogSnapshot`. App's page-lifetime
  `diagnosticsOpenIntentEpochRef` is renderer presentation only: it orders
  overlapping clicks that may open one dialog because the diagnostic DTO is
  outside AppState/appStore and carries no request/state generation. Both stale
  success and stale failure must return before replacing the runtime snapshot,
  appending the fixed unavailable token or reopening a later-closed dialog. The
  epoch deliberately survives account replacement because the Rust DTO is
  global/runtime and contains no private account values; the report composes it
  with current AppState. `copyDiagnostics` is a separate stateless clipboard
  action and does not use the dialog epoch.
- Core `UploadMediaRequest` Debug output redacts filenames, captions, media
  bytes, and thumbnail bytes.

## State transport

- The #111 state-transport architecture is Rust-owned incremental slice deltas
  plus a selector-subscribed WebView projection cache. React may cache and
  subscribe to Rust snapshots for rendering, but it must not mutate product
  state, synthesize Matrix semantics, or repair command results locally.
- Runtime/background state updates enter the WebView on one ordered state-update
  lane as versioned `StateDelta` changed-slice DTOs. Full snapshots are initial
  attach or explicit gap, lag, and command-watermark resync only. Normal command
  returns carry a typed Core settlement/admission generation or their non-state
  result and may never be applied as product state. Apply deltas and resync
  snapshots through the projection store by preserving references for unchanged
  `domain`, `ui`, `sidebar`, timeline, and thread data. Hot derived arrays such
  as mention candidates and forward destinations must be memoized from Rust DTO
  input references.
- Delta generation gaps must atomically recover through a versioned full
  snapshot (`state_generation`), timeline-store reset, and timeline replay before
  later deltas apply. A command generation ahead of appStore triggers one
  deterministic snapshot resync; it is not proof of renderer receipt or paint.
- Tauri Channels are high-frequency only and measurement-gated. Keep crawler,
  typing, receipt, and presence semantics Rust-owned; a Channel transports Rust
  projections, not React-local state.

## Account work scheduler

- App-owned work that competes for the homeserver goes through the Rust-owned
  account scheduler in `crates/koushi-core/src/account_work.rs`. Call sites name
  a semantic `AccountWorkKind`; `AccountWorkKind::policy()` is the only place
  priority numbers, scheduling class, concurrency, and batch bounds live. Do not
  add another endpoint gate and do not pass raw priority numbers around.
- The three classes are normative. Interactive work (`MessageSend`,
  `UserRoomOperation`) never queues: it takes `begin_interactive` for the SDK
  enqueue only, which asks worse-priority preemptible work to yield and keeps a
  yielding job from re-contending until the enqueue completes. Foreground work
  (`VisibleGapRepair`, `ExplicitPagination`) is preemptible by better priority but
  is never deferred behind an interactive enqueue. Background work
  (`OffscreenGapRepair`, `SearchCrawl`, `Maintenance`) yields to everything better
  and waits for an interactive enqueue.
- Permit cancellation is cooperative and is not a failure. Finish the current
  bounded batch, keep the checkpoint, drop the permit, and re-enter scheduling.
  One permit means one bounded batch: gap repair acquires per batch and releases
  before local projection settlement so a send never waits for it.
- The vendored SDK exposes no cancellation argument for
  `repair_timeline_gap_with_projection` (unlike live-tail refresh), so Phase A
  yields between batches and does not abort an in-flight request. Do not patch
  vendored SDK for this without a recorded upstream-feedback decision.
- Scheduler diagnostics use source `core.account_work` with stages `queued`,
  `started`, `preempted`, `yielded`, `completed`. They carry work id, kind token,
  priority, preemptible flag, queue wait, run time, batch and item counts only.
- Focused checks: `cargo test -p koushi-core --lib account_work` and
  `cargo test -p koushi-core --lib gap_repair_work_kind_follows_reported_visibility`.

## Timeline items and the outbound send queue

- Tauri production timelines render from the CoreEvent-backed `TimelineView`
  store, not `AppState.timeline`. A local GUI lane that needs a real row/action
  must wait for DOM state such as `.message`, `data-event-id`, or
  `button[aria-label="Message actions"]`; `timeline_items=0` in the QA title can
  be normal because that token comes from the snapshot DTO.
- `send_text` must route through the SDK UI `Timeline::send` path, not a direct
  `room.send_queue().send` call. The latter can settle `SendCompleted` while
  starving the event-driven `TimelineView` of local-echo diffs in the Linux
  WebView lane.
- Core must not depend on that SDK local-echo diff for first visibility. The
  session-scoped send coordinator owns one bounded pending display projection
  from accepted client transaction through SDK/event identity convergence. The
  current `TimelineActor` combines that projection with canonical SDK slots and
  acknowledges publication before the matching composer acceptance may clear
  the draft. Actor replacement receives the same bounded snapshot; React only
  applies the resulting ordinary Rust-authored timeline diffs.
- Retry/cancel is driven by SDK `SendHandle`, not by a direct
  `RoomSendQueue::retry(transaction_id)` API. `TimelineActor` keeps its
  transaction-id keyed handle registry from `RoomSendQueue::subscribe()` local
  echoes; when that echo is missing it uses the exact handle retained by the
  manager-owned coordinator. Body, sender, and timestamp never participate in
  correlation.
- Recoverable SDK send errors disable the room send queue. `RetrySend` must call
  `room.send_queue().set_enabled(true)` before `SendHandle::unwedge()`;
  successful `CancelSend` must also re-enable the room queue after
  `SendHandle::abort()` so successors are not stranded behind a removed failed
  item.
- `TimelineItem.send_state` is a Rust-owned DTO projection. React may render it
  and dispatch `retry_send` / `cancel_send`, but must not infer send legality
  from `TimelineItemId::Transaction` or repair queue state locally.
- `TimelineItemId::Transaction` is a stable Rust-owned pending/local-echo
  identity, not a UI state. It starts with the client transaction ID, changes to
  the SDK transaction ID only through the coordinator's exact bind, and changes
  to the terminal event ID only through `SentEvent`. A transaction row without
  `send_state` must not be labeled unsent; failed/sending/cancelled affordances
  come only from `send_state`.
- Transaction timeline rows use `timelineItemDomId`, so local echoes render with
  `data-item-id="txn:<transaction_id>"`. Headless media-progress specs should
  target that canonical id instead of the raw transaction id.
- Phase B send-queue GUI tests should seed Rust-shaped CoreEvent timeline items
  in `appHarnessMain.tsx` / `composer-send-queue-upload.spec.ts`, click the visible
  controls, and then push a CoreEvent diff to prove the UI reflects Rust-owned
  state changes. Do not update React state directly after `retry_send` or
  `cancel_send`.
- A reply must target a MESSAGE event, not a state event. The timeline includes
  state events (room create, membership) that carry no body; the SDK's
  `make_reply_event` rejects them (app stderr `make_reply_event failed:
  StateEvent`, surfaced as `send=failed`). `TimelineItemRow` therefore gates the
  reply affordance on `item.body !== null`, so only message rows are replyable.
- Timeline reactions are Rust-owned projection state. React must only dispatch
  typed `SendReaction` / `RedactReaction` commands; do not implement toggle
  semantics in the UI, because `Timeline::toggle_reaction` is only an internal
  Rust delegation detail behind the typed boundary.

## Local viewed and server-confirmed read state

`TimelineActor` owns the verified local viewed boundary for Room and Thread
windows. It accepts only an at-bottom viewport observation whose last visible
canonical event is the latest eligible readable item, has exact actor-generation
and position-index evidence, and is not obscured by a visible gap. It emits the
local boundary immediately; it never calls receipt or read-marker IPC.

`TimelineManagerActor` owns the bounded automatic read-intent correlation map
and the four-slot dispatcher. Room boundaries require the private fully-read /
read-receipt bundle and, when `SettingsValues.notifications.send_read_receipts`
is enabled, the public receipt. Thread boundaries require only the matching
thread receipt when that policy is enabled. Focused timelines never create
automatic intent. Settings changes travel from the Rust settings projection
through `AccountActor` to a session-generation-fenced manager control update;
React cannot change this policy locally.

`TimelineNavigationSnapshot.read_marker_event_id` and unread counts remain the
server-confirmed boundary. `local_viewed_event_id` and
`read_marker_display_event_id` may advance while the write is pending or failed.
`read_state_sync` is the Rust status (`pending`, coarse `failed`, `synced`, or
`notRequested`) rendered by `TimelineView` as an accessible status; pending or
failed state never clears badges/counts. Browser Fake receipt methods are
transport no-ops: only an installed Rust-shaped snapshot/event may change its
read state.

Focused checks:

```bash
cargo test -p koushi-core --lib read_state::tests::
npm --prefix apps/desktop run test -- --run src/components/TimelineView.live-state.test.tsx
```

## Room-subscription residency

- `TimelineManagerActor` is the only Koushi owner that mutates the live
  `RoomListService` room-subscription set. It unions opened timeline rooms,
  successfully projected non-left room-list entries, and valid restored
  coverage into one account-session `BTreeSet`.
- Residency is uncapped and in-memory only. Timeline actor unsubscribe/rebuild,
  Thread/Focused navigation, cache eviction, and projection replay release actor
  resources but never remove room residency. Successful leave removes that room;
  logout, account switch/reset, or account deletion drops the session owner.
- Room-list and leave intents are serialized through the manager mailbox.
  Visible-range intents are fenced by the current sync generation; successful
  leave uses a manager-instance typed handle installed before room operations
  are enabled. Direct leave, invite decline, accept, direct join, and directory
  join require pointer identity between RoomActor's current
  `Arc<MatrixClientSession>` and the session atomically bound to the manager
  handle, then snapshot one manager-instance permit before awaiting the SDK.
  A replacement install→`SessionEstablished` mismatch fails before any SDK call;
  permits remain held through existing success/failure reducer/event settlement,
  then acknowledged admitted permits drain before that manager is
  retired. Session-local leave state blocks restore/visibility resurrection;
  only an admitted successful local rejoin or ordered SDK `left` then
  `joined|invited` observation clears it. Observer coalescing must preserve the
  receipt order of those membership transitions.
  Repeating the same desired set is an SDK no-op; delayed retired-generation
  observations are ignored.
- UnknownPos remains genuine coverage loss. Koushi may re-submit its in-process
  desired set but must not suppress the SDK's members-missing reload, Megolm
  rotation, checkpoint, or recovery behavior. Do not add a second
  `RoomListService`, an LRU/timer, a persistence format, or a crypto override.
- Cross-actor checks live in
  `crates/koushi-core/tests/room_subscription_residency.rs`; only pure helper
  tests may stay inline.

## Timeline navigation

- Timeline navigation semantics stay in Rust. React may report viewport facts
  through `observe_timeline_viewport` (`first_visible_event_id`,
  `last_visible_event_id`, `at_bottom`) and may scroll to returned anchors, but it
  must not compute read-marker placement, first-unread targets, unread counts, or
  jump-to-bottom counts.
- `TimelineActor` emits `TimelineEvent::NavigationUpdated` from the current
  projected item order and fully-read marker. Diff-driven navigation updates must
  be emitted after `ItemsUpdated` so GUI rows exist before a Phase B scroll action
  references them.
- Jump-to-date uses `open_timeline_at_timestamp`, which routes through
  `AppCommand::OpenTimelineAtTimestamp` and the Matrix `timestamp_to_event`
  endpoint in Rust before reusing focused context. React must not call raw Matrix
  APIs for date jumps.
- `TimelineView` renders first-unread/bottom pills only from
  `TimelineEvent::NavigationUpdated`. The date picker dispatches
  `open_timeline_at_timestamp`; it must not resolve event IDs in React.
- The read-receipt/fully-read auto dispatch is constrained to the bottom
  viewport. If the viewport is not at bottom, React reports only
  `observe_timeline_viewport` facts so Rust can keep unread navigation projection
  stable.
- In-session room re-entry freezes anchor eligibility against the first
  `InitialItems` window. A later historical prepend may not turn an initially
  absent session anchor into a restore target. When user scroll input is still
  pending and the DOM is actually at bottom, record live-edge even if the event
  matches a recent programmatic-write signature. The one-shot `timeline.scroll
  stage=room_reentry_restore` diagnostic carries session mode, age bucket,
  anchor-live verdict, and path only.
- Projection commitment is Core-internal. TimelineActor reports exact
  request/key/actor/timeline-generation and target-presence facts through the
  reliable manager/account/AppActor ownership chain; focused-navigation outcome
  never waits for App timeline-store application, Room DOM evidence, WebView
  command delivery or paint. Renderer application/DOM evidence remains local
  layout/diagnostic state and is safe to drop on unmount.
- Gap-repair continuation is released by an exact
  actor/timeline/repair/minimum-batch-fenced relay/display-projection commit
  signal inside Core. No DesktopApi projection/render acknowledgement command,
  delivery retry owner, timeout, or browser backoff participates in product
  progress. Do not reintroduce the removed acknowledgement route.

## Live catch-up and gap repair

- Room-entry live catch-up is committed-response and generation fenced. The SDK
  retains the latest token-free committed observation per room; TimelineManager
  replays it after actor registration, and TimelineActor defers only `LiveEdge`
  repair until the matching backend epoch, room, actor, and
  response/subscription generation arrive. The actor must repair the
  observation-owned gap exactly and never substitute another persisted gap.
  Explicit no-update/no-gap closes the intent; stale descriptors get one
  authoritative re-inspection.
- Relay settlement fences must be bounded and recover through authoritative
  resync while retaining queued work. Exact relay/display-projection commitment,
  not renderer paint, releases repair continuation. SDK diff projection tags from
  a superseded timeline actor generation are discarded at the relay boundary
  before `relay_received=queued`; keep `rejected_operation` for current-actor
  correlation violations rather than filling it with known-obsolete generation
  noise.
- A run must carry the SDK response sequence of its first successful response and
  reject retained observations from earlier responses plus duplicate per-room
  commit sequences.
- Focused gate: `cargo test -p koushi-core --lib timeline_actor_waits_for_current_subscription_checkpoint_before_live_edge_repair`.

## Threads and attention

- Notification, badge, sound, tray, and activation decisions are Rust-owned
  `AppState.native_attention` projections. GUI/native adapter code must render or
  dispatch from the snapshot/capability DTOs; it must not invent notification
  candidates, badge counts, dedupe, or suppression semantics.
- React attention helpers may only map `snapshot.state.native_attention` to
  window title, badge, and native adapter payloads. They must not aggregate
  `rooms`, diff previous room snapshots, or infer focused-room/muted/duplicate
  notification behavior locally.
- Keep persistent and transient effects separate. Window title, badge count,
  Windows overlay, tray count, and zero-badge clearing are snapshot-state
  mappings. Sound and activation are candidate-scoped transient effects and may
  run only from a Rust-owned notification candidate plus the Rust-owned
  capability DTO; do not trigger them from every unread/badge snapshot refresh.
- Passive native notification dispatch checks the current OS permission state
  only. It must not call permission-prompt APIs; permission prompts belong to an
  explicit user/onboarding action.
- Native notification clearing is adapter-only and best-effort. When Rust-owned
  attention state drops the badge count to zero (including logout/account clear),
  React may call the native transport clear hook, but it must not mutate Matrix
  state or synthesize read/focus semantics locally.
- Platform capability profiles are Rust-owned and resolved from the shared
  `DisplayPlatform` model before reaching React. Add macOS/Linux/Windows
  capability differences there; do not scatter platform branches through React
  components or notification helpers. Windows taskbar overlay support is the
  Rust-owned `NativeAttentionCapabilities.overlay_icon` field; React adapter code
  may call `setOverlayIcon` only from that DTO capability, never from direct OS
  sniffing.
- Notification sound policy is Rust-owned
  `SettingsValues.notifications.sound`. React may pass that DTO value into native
  adapter routing so sound is skipped, but it must not create an independent
  notification preference or mutate native attention state locally.
- Candidate projection uses private-data-minimized room labels and counts only.
  Native attention uses `RoomSummary.display_label` for its safe room label, but
  serialized candidates still must not carry room IDs, sender IDs, event IDs, or
  message bodies. Profile/alias relabeling of an existing candidate is a Rust
  reducer projection over `state.rooms`, not a React notification-policy repair.
- Space rail attention badges are Rust-owned `SidebarModel.space_rail` counts
  produced by `compose_sidebar`; `WorkspaceRail` may render the snapshot
  attributes but must not recompute child-room unread/highlight state.
- Timeline thread chips render the Rust-projected row `thread_summary` DTO.
  One session-scoped Core projection reconciles the SDK/event-cache aggregate
  with accepted live activity for canonical, Thread/Focused, and hydrated
  off-window roots. React must not fill null fields, choose between a bundled
  summary and visible replies, retain an expected latest reply, or repair a
  stale non-null summary. Replay/restart rehydrates the same Core projection
  from the SDK event cache; no frontend or first-party plaintext summary store
  exists.
- Thread-root lifecycle and placement are also Rust-owned. The session-scoped
  `ThreadRootProjectionService` retains canonical/hydrated root snapshots until
  authoritative aggregate/redaction clear, Room unsubscribe, or session teardown;
  a bounded display omission is dormant, never deletion. Rust State mirrors only
  explicit Core lifecycle actions. The Room actor's `DisplayProjectionState`
  applies root-event/latest-reply order, standalone-reply suppression, stable row
  identity and display-relative diffs. `TimelineItem` display metadata is a
  hand-maintained Rust/Tauri/TypeScript wire contract and must update every mirror
  listed in [Snapshot and wire-contract mirrors](#snapshot-and-wire-contract-mirrors).
  `timelineStore` applies the projected items/diffs and prunes only an entire
  timeline key or an explicit Rust clear; frontend code must not scan current
  items to infer projection death or choose placement. `TimelineKeyState.items`
  is the Rust display-index domain consumed in production only by `TimelineView`;
  Core's private `navigation_items` remains canonical for read state, search,
  receipts, Activity and event-cache reconciliation. `TimelineView` keeps DOM
  measurement, virtualization, date-divider rendering, anchoring, layout
  settlement, and post-subscription geometry-triggered pagination intent only.
  That pagination intent may run after Core has published an authoritative-empty
  existing thread; it does not confirm emptiness, choose projection lifetime, or
  derive the pre-subscription `InitialBackfillPolicy` from promoted pane state.
  Pane-level thread attention is Rust-owned `AppState.thread_attention`; React
  may render the DTO but must not scan visible thread rows or row chips to derive
  indicator counts. The core producer uses the authoritative own threaded receipt
  plus explicit hydration/live/backfill/replay lifecycle and stable event-ID
  deduplication. Relay batches carry the SDK event origin so a delayed
  pagination/cache batch cannot become live after ambient task state changes. It
  counts only matching remote `m.thread` replies; roots, own local/remote echoes,
  read history, and reconnect duplicates are ignored. A first-seen recovery/reset
  reply may count only when its position after the authoritative visible receipt
  proves it unread. SDK vector mutation shapes such as `PushBack` are not unread
  evidence.
- GUI thread indicators, including the Threads nav badge/markers, render only
  `AppState.thread_attention.notification_count`, `highlight_count`, and
  `live_event_marker_count`. Do not derive them from room-list totals,
  `TimelineItem.thread_summary`, or visible thread rows. Total reply count stays
  in `thread_summary.reply_count`; successful threaded read acknowledgement
  clears new/unread attention through the Rust actor/reducer snapshot path.
- Focused checks: `cargo test -p koushi-state --test attention_surface`,
  `cargo test -p koushi-sdk --test attention_surface`,
  `npm --prefix apps/desktop run test -- src/domain/desktopAttention.test.ts`, and
  `cargo test -p koushi-state --test session_state logout_clears_native_attention_state_and_notifies_ui`.

## Live signals

- `AppState.live_signals` is the Rust-owned source of truth for read receipts,
  fully-read markers, typing users, and presence. React may render it and
  dispatch typed commands only; do not add React-local receipt, marker, typing,
  or presence semantics.
- Read-receipt reader avatars are Rust-owned live-signal projection data:
  reducers resolve reader display labels and avatar DTOs from profile state,
  dedupe by reader using the newest timestamp, order readers most-recent-first,
  cap the rendered readers, and expose `overflow_count`. The existing
  `AvatarThumbnailUpdated` reducer settles every already-copied reader avatar
  with the same exact MXC URI and emits `LiveSignalsChanged` when any changes.
  Main and thread timelines consume this same room/event projection.
  `TimelineView` renders the DTO and may own only tooltip visibility through
  DOM/CSS; do not join receipts with `profile.users` in React.
- Read-receipt readers carry both `display_name` (the Rust-projected visible
  label, despite the legacy field name) and `original_display_label` for
  alias-free hover/profile context. React must not recover original names by
  looking up profiles or stripping a local alias.
- Timeline live-signal commands route through `TimelineCommand` and the
  subscribed `TimelineActor`: `SendReadReceipt`, `SetFullyRead`, and `SetTyping`.
  Account presence routes through `AccountCommand::SetPresence`. Keep SDK handles
  and sync policy in Rust actors.
- The current presence implementation records and emits the requested Rust-owned
  presence state; full network presence propagation remains a sync-backend
  decision because the `SyncService` builder exposes no direct presence setter.
- React may use refs only to suppress duplicate viewport-triggered command
  dispatches such as mark-read/read-receipt sends. The values themselves remain
  Rust-owned.

## Activity

- `AppState.activity` is the Rust-owned source of truth for account-wide
  Recent/Unread Activity. React may render `ActivityState`, switch tabs through
  `set_activity_tab`, request pagination, open focused context from the row's
  event reference, and dispatch `mark_activity_read`; it must not sort,
  synthesize unread membership, clear rows locally, or derive account-wide
  Activity from `TimelineView` DOM state.
- Activity rows are observed by room `TimelineActor`s as `ActivityRowsObserved`
  and materialized by the `AppActor`'s Activity projection cache. The projection
  fills room labels, unread flags, highlight flags, and low-priority exclusion
  from Rust-owned `AppState` facts. Keep this cache outside React and outside
  per-view browser fake state.
- Opening or paginating Activity snapshots the Rust projection into separate
  Recent and Unread streams. Viewing the Unread tab does not mark anything read.
  `MarkActivityRead` settles both room targets and the all-activity target
  through the Rust `mark_read` substate and then updates Activity streams; future
  SDK fully-read writes must stay behind the same typed command boundary.
- `ActivityStream.resolution` owns stale-unread history loading. A `RoomUnread`
  row is a transient Rust resolver input, not terminal GUI content.
  `AccountActor` resolves it from decrypted timeline cache/live updates and
  bounded backward pagination (at most 16 rooms per generation), guarded by a
  generation and the shared messages backpressure gate. Per-room success is
  retained when another room fails, and capped batches rotate across retry
  generations so persistent failures cannot starve later rooms. React renders
  resolving/failed status, hides placeholders, and dispatches only typed
  `retry_activity_resolution`.
- Browser-headless Activity GUI tests should seed Rust-shaped
  `AppState.activity` snapshots and assert that rows stay visible after
  `mark_activity_read` until a later snapshot removes them. Do not make React
  sort rows, infer low-priority exclusion, auto-clear Unread on tab view, or
  repair mark-read results locally.

### Activity Event Navigation

- Rust owns the Activity, Search, and Pinned event-navigation lifecycle and
  failure state, including the outer cancellation contract with room selection,
  thread-row navigation, date jumps, explicit return-to-live, and one another.
  The last accepted outer navigation intent wins; displaced work is benign and
  cannot change the primary view, right panel, focused subscription, failure,
  or waiter.
- Tauri and React must not orchestrate the close/select/open/wait sequence or
  own epochs, shared failure state, or promise arbitration. They dispatch typed
  intents and render only the current Rust terminal (`Anchored`, `LiveFallback`,
  or `Failed`).
- The thread inner navigation machine remains separate. Thread-row navigation
  still participates in the Rust-owned outer cancellation contract and cannot
  let a stale event-navigation completion settle it.

## Rooms, tags, and the sidebar

- `RoomSummary.tags` is the Rust-owned source of truth for Matrix `m.tag`
  favourite and low-priority state. React may render tag affordances and dispatch
  `set_room_tag` / `remove_room_tag`, but it must not keep local tag membership or
  repair room-list sections after the fact.
- Favourite and low-priority are mutually exclusive in `koushi-state`. Keep this
  reducer rule in sync with the SDK wrappers: use `koushi-sdk`'s `set_room_tag` /
  `remove_room_tag`, which delegate to `Room::set_is_favourite` and
  `Room::set_is_low_priority`; do not patch the vendored SDK for this behavior.
- Tag command success must not immediately request a room-list refresh. The SDK
  tag calls send account-data changes to the homeserver, and the local SDK room
  snapshot can remain stale until the next sync. Project the successful command
  through `RoomTagSet` / `RoomTagRemoved` reducer actions, then let the next sync
  snapshot become canonical.
- When adding fields to `RoomSummary`, update every projection and fake snapshot:
  `koushi-core::room::normalize_rooms`, `koushi-state::sidebar::RoomListItem`,
  plus the mirror list above.
- Sidebar shell affordances (section counts, unread badges, mention dots) render
  `SidebarModel` fields such as `unread_count` / `highlight_count`. When a sidebar
  projection field changes, update Rust `compose_sidebar`, the Tauri DTO
  serialization-contract test, `types.ts`, browser fake snapshots, `tauriIpcMock`,
  app harness snapshots, and browser-headless shell tests together.
- Room-list sections (Favourites / People / Rooms / Low priority / Not joined)
  and their ordering are emitted directly as `SidebarModel.sections` by
  `compose_sidebar_for_state`. React may text-filter the selected already-ordered
  vector but must not classify tags/DMs, join room/Space membership, compute
  attention or sort. Account-global invites remain the Home navigation/count and
  are not a room section.
- Room-tag GUI tests should stub `set_room_tag` / `remove_room_tag` to return the
  current snapshot first, assert the row does not move immediately, then push a
  Rust-shaped snapshot with updated `RoomSummary.tags` / sidebar room tags and
  assert the section movement. This catches accidental React-local room-list
  repair.

## Home-scoped navigation

- A sidebar entry's location has to explain its scope. Account-global views
  (Activity, Explore, Invites) render only under Home; a space sidebar is the
  room list for that space, and its space-scoped actions are the header icons. Do
  not reintroduce an account-global entry into a space sidebar.
- A room's thread list opens from the room header, and that button is
  unconditional. It used to appear only when a thread had unread attention, so
  removing the sidebar entry without ungating it would have made a quiet room's
  threads unreachable. The header counts still come from
  `AppState.thread_attention`, gated on the tracked thread belonging to the open
  room — that gate is the point, not a bug: the old sidebar badge showed a
  room-scoped count no matter which room was open.
- Account-wide and space-scoped thread aggregation does not exist. Issue #332
  owns it. `ThreadsListState` is keyed by `room_id`, so there is nothing above one
  room to filter.
- The Home rail badge renders Rust-owned `AccountHomeItem.attention_count`
  (`unread_count + invite_count`). Keep the three fields separate: the badge needs
  one number, and the accessible label names unread messages and invites
  individually through the catalog. Do not sum them in React, and do not let
  `unread_count` absorb invites. Space rail badges stay unread-only —
  `InvitePreview` carries no reliable parent-space scope.
- Both Tauri sidebar paths must pass the same account facts. `From<AppState> for
  FrontendDesktopSnapshot` composed from rooms and spaces alone, so the full
  snapshot dropped mute filtering that `koushi_core::state_delta` applies and the
  same state produced different Home badges per transport.
  `compose_sidebar_with_account_facts` takes notification settings and the invite
  count; the three-argument `compose_sidebar` wrapper is for callers that
  genuinely hold neither.
- Focused checks: `cargo test -p koushi-state --test navigation_state`,
  `cargo test -p koushi-desktop --lib`, and
  `npx playwright test e2e/home-scoped-navigation.spec.ts`.

## Invites and DMs

- `AppState.invites` is projected by `RoomActor` from SDK invited rooms; React
  must render it and dispatch typed commands (`AcceptInvite`, `DeclineInvite`,
  `StartDirectMessage`) instead of maintaining local invite lifecycle state.
- The live room-list entries adapter uses the non-left filter, and the same
  observer uses the base client's already-committed room-update broadcast as a
  bounded auxiliary wake: an invite can commit outside the visible entries head
  without changing that head. Coalesce wakes and reproject only on invite payload
  or membership changes (plus one lag recovery); do not start another sync owner
  or `RoomListService`.
- Rust owns invite target queries, candidate/status derivation, scope/history
  policy and workflow state. Tauri `open/search/close` waits at most two seconds
  for an exact Rust `InviteWorkflowState` terminal; queue acceptance alone is not
  convergence. React retains two presentation fences: the mounted Space panel's
  debounce/candidate-list epoch and one App epoch shared across room-dialog and
  Space-panel workflow lifetimes. Stale returned promises cannot directly apply a
  mismatched snapshot or candidate list; Rust StateDelta remains authoritative.
  Every convergence rejection is caught with fixed private-data-free diagnostics,
  and Space search resolves `[]` so its spinner cannot stick. Scope/target/invite
  execution settlement remains a separate audit family.

## Public directory and Explore

- Public directory semantics are Rust-owned. `AppState.directory.query` and
  `AppState.directory.join` are separate state machines; React must render those
  DTOs and dispatch typed `query_directory` / `join_directory_room` commands only.
  Do not recreate query, pagination, join success, or failure state in React.
- Directory join is alias-based. The SDK wrapper rejects bare room IDs for the
  directory flow; GUI code should pass the canonical alias and optional server
  hint from the Rust directory result.
- Explore has two sections — join by address, and search a public directory — and
  both submit through the single `resolveDirectorySubmission` classifier so the
  same string cannot be read two ways. A full address pasted into the search field
  routes to preview, because a directory text search cannot find a room addressed
  by id. Neither path joins directly; both land in the Rust-owned preview dialog.
- A bare `@user:server` classifies as `user`. Only matrix.to user links did
  before, so a pasted MXID became a directory search and returned "no public
  rooms found" — which reads as if the person did not exist.
- Give an input and its submit button different accessible names. Explore's
  search input and Search button both answered to "Search public rooms", and
  WebDriver could only tell them apart by tag name. Inputs take their field
  label, buttons take their visible text.
- Playwright specs that reach Explore or Invites must select Home first — the app
  harness boots with a space selected. `room-space-invites.spec.ts` has a
  `selectAccountHome` helper for this.

## Message interactions

- `TimelineItem.reply_quote` is a Rust-owned projection. React renders the
  `ReplyQuoteState` and optional preview only; it must not look up reply bodies,
  classify redactions, or patch quote state after a send.
- `TimelineItem.actions` is a Rust-owned action-affordance projection. React may
  render/copy only the DTO-provided body/permalink affordances; it must not build
  `matrix.to` permalinks, infer copy/forward/source eligibility from event ids,
  body/media fields, or redaction flags, or synthesize message-source / forward
  semantics locally.
- `TimelineCommand::LoadMessageSource` and `TimelineCommand::ForwardMessage` are
  the typed path for view-source/forward GUI work. The source DTO is a safe Rust
  projection, not raw Matrix JSON. Forwarding sends the Rust-projected visible
  body only; media-only rows must remain non-forwardable until a dedicated
  media-forward contract exists.
- Megolm session-change attribution in source details is Rust-owned and
  current-device-only. Core compares the SDK encryption sender device with the
  active session, queries the full room/session identity only inside the trusted
  SDK boundary, and projects a closed reason or `notRetained`. React renders the
  enum and never derives a reason from dates, fingerprints, counters, or timing.
- Message-action menus render only `TimelineItem.actions` affordances. Copy uses
  the Rust-projected row body or Rust-built permalink only; view source
  dispatches `load_message_source` and waits for `MessageSourceLoaded` before
  showing the source dialog; forward dispatches `forward_message` with
  Rust-snapshot room destinations and never copies the message body in React.
- `AppState.room_interactions` is the Rust-owned source of truth for
  `pinned_events` and `pin_operation`. GUI code dispatches typed `pin_event` /
  `unpin_event` commands and waits for Rust-shaped snapshots/events instead of
  mutating local pin lists.
- Recoverable pin/unpin failures must remain retryable in the reducer. Do not
  clear failed pin state from React; a new typed request transitions the Rust
  state from `Failed` to `Pending`. Pin/unpin command success settles the Rust
  pending state before the follow-up pinned-event reload. A reload failure may
  emit a coarse operation failure, but it must not leave the GUI stuck in
  `Pending`.
- The replacement shape of an edit is Rust-owned. The GUI submits only the new
  visible text, so `handle_edit_text` resolves the target's SDK message type and
  chooses `EditedContent::MediaCaption` for media (`m.image`, `m.file`, `m.audio`,
  `m.video`) and a plain-text replacement for everything else. A media event
  carries its attachment in the same `m.room.message` content as its caption, so a
  text replacement drops `url`/`file`/`info`/`filename` — that was issue #328. The
  msgtypes routed to the caption path must stay exactly the set that
  `message_projection_from_msgtype` projects with `TimelineItem.media`;
  `edit_replacement_caption_support_matches_media_projection` pins that equality.
  Do not let the webview pass a msgtype or reconstruct media content. Focused
  check: `cargo test -p koushi-core --lib edit_replacement_`.

## Formatted message rendering

- Received Matrix `formatted_body` is a Rust-owned security projection. Core
  sanitizes Matrix HTML into `TimelineItem.formatted` before it crosses the
  WebView boundary, including plain-text and code-block metadata. React must
  render only that Rust-owned DTO; it must never render unsanitized server HTML
  or own ad hoc Matrix HTML sanitizer policy.
- The `TimelineView` formatted-message renderer is presentation-only. It may map
  the Rust-owned sanitized HTML/code-block DTO into React nodes and copy-code
  controls, but any tag/attribute safety decision belongs in Rust. When adding
  supported tags, extend the Rust projection tests first, then the React renderer
  and browser-headless checks.
- Display preferences such as code-block line wrapping are Rust-owned
  `SettingsValues.display` product state and must persist through the settings
  store with legacy JSON backfill. React may map `code_block_wrap` to CSS and may
  omit timeline rows only from the Rust-projected `TimelineItem.is_hidden` flag.
  It must not keep a separate local display-policy store, derive redacted
  visibility from React settings state, or repair switch state after dispatch.
- Timeline mention pills are display-only rendering over Rust-owned timeline body
  text plus `ProfileState.users`; they must not become a React-owned source of
  mention semantics.

## Profiles and local aliases

- Own-profile state, per-user profile cache, room avatars, and space avatars are
  Rust-owned DTOs. React renders them and dispatches `set_display_name` /
  `set_avatar`; do not add React-local profile success/failure semantics.
- React may discover visible/not-requested avatar MXCs and submit typed thumbnail
  demand. After submission, `AccountActor` owns single-flight deduplication,
  bounded concurrency, the two-attempt network policy and the session-terminal
  Ready/Failed cache. Ready state carries an opaque cache reference. The Tauri
  link/media port may map it to `koushi-thumbnail://`; Core/state/protocol never
  mint that URI, and a native frontend may consume Core bytes directly.
  Renderer request sets may suppress duplicate transport admission only; they
  must not classify retryability or count/release retries.
- Personal local user aliases are also Rust-owned profile state. Keep alias
  set/clear/list, persistence to `app.koushi.local_aliases`, display-name
  resolution, and pending/failure state in Rust; React may render the returned
  labels and dispatch typed commands only. The SDK uses
  `app.koushi.local_aliases` only; old Kagome-era account-data migration is not
  required for this project state.
- `UserProfile.display_label`, `UserProfile.original_display_label`, and
  `UserProfile.mention_search_terms` are the Rust-owned person/mention
  projection. `display_label` may contain the local alias;
  `original_display_label` is the alias-free upstream/own-profile/MXID context
  value. GUI mention suggestions/highlighting and profile/tooltips must use the
  projected fields instead of recomputing alias precedence or stripping aliases
  in React.
- Timeline sender display is Rust-projected. `sender`, reply quote `sender`, and
  thread-summary `latest_sender` remain raw identity fields; normal TimelineView
  display must use `sender_label`, `reply_quote.sender_label`, and
  `thread_summary.latest_sender_label` when present. Do not repair missing labels
  in React by joining sender ids to `local_aliases`.
- Existing timeline rows are relabeled through the keyless Rust
  `TimelineEvent::DisplayLabelsUpdated` patch stream after profile/alias changes.
  Frontend stores may match raw identity fields and apply the supplied labels
  across loaded timelines, but React must not resolve alias precedence or
  synthesize fallback labels. When clearing an alias, keep the target user id in
  the Rust emission even if the user is absent from `profile.users`.
- Room-scoped member labels are Rust-projected too:
  `RoomMemberSummary.display_label` is resolved from
  `ProfileState.local_aliases`, nonblank room-scoped upstream `display_name`,
  profile cache / own-profile fallback, and finally MXID when room settings load,
  room settings update, or profile/alias state changes.
  `RoomMemberSummary.original_display_label` carries the alias-free context
  label. `display_name` remains the upstream/original raw value. React member
  lists, sort order, action labels, and original-name affordances must consume
  these projected fields and must not join `settings.members` with
  `local_aliases` or the global profile cache.
- Local alias GUI affordances dispatch only the typed `set_local_user_alias(user_id, alias)`
  account command. React may own dialog visibility and input draft text,
  including trimming empty input to a clear, but it must not update member rows,
  DM titles, timeline labels, receipts, or mention candidates locally. The bounded
  `alias:${userId}` mutation lane is a renderer-specific autosave transport/result
  owner, not alias state: Tauri returns a pre-terminal snapshot and browser results
  share one generation, so it serializes started input writes, skips superseded
  pending writes and applies only the latest result. Rust still exclusively owns
  durable aliases, `Saving`/failure, reconciliation and every display projection.
- Room summaries use the same Rust-owned display projection:
  `RoomSummary.display_label` is the sidebar/header/search/forward/space-child
  display value, while `RoomSummary.original_display_label` is the alias-free
  room/DM context value. `display_name` remains the upstream/original room name.
  For one-to-one DM rooms, `dm_user_ids` carries the target identity and labels
  resolve through local alias, nonblank upstream room name, profile/own-profile,
  then MXID. Non-DM rooms use trimmed upstream `display_name`, then `room_id`.
  These are caller-owned room/user data, not i18n catalog prose; do not invent
  generic English fallbacks such as `Member`, and do not infer the DM target from
  a room title in React.
- `AvatarImage.mxc_uri` is metadata, not a render URL. GUI code renders an `<img>`
  only for `AvatarThumbnailState::Ready.source_url`; otherwise it uses the
  colored-initial fallback. This keeps the current #15 media contract intact
  because timeline `download_media` emits byte counts only.
- Profile update completion settles a user-visible pending state. Actor code must
  deliver `ProfileUpdateSucceeded` / `ProfileUpdateFailed` reliably via the action
  channel, not as a best-effort notification that can leave settings controls
  stuck in a saving state.
- When adding GUI labels for alias/profile affordances, update the `MessageId`
  union, English and Japanese catalogs, and `messages.test.ts` coverage together;
  adding only the English catalog makes runtime `t(...)` calls fail before the
  typecheck catches the missing key.
- Mention candidates come from Rust-owned `AppState.mention_candidates`,
  projected from SDK room member profiles during room-list observation. React may
  insert a selected candidate only as an atomic `ComposerDocument` mention node;
  it must not keep parallel pills/mention metadata.

## Room management and moderation

- Room-management GUI work must render only `AppState.room_management`. Settings
  snapshots and permission facts are Rust-owned; React should disable controls
  from `settings.permissions` and dispatch typed commands, but it must not decide
  or repair permission, setting, or kick/ban/unban state locally. Tauri
  room-management commands wait for correlated `RoomEvent`s and must not call SDK
  wrappers directly.
- App's room/Space settings request epochs and load markers are renderer-only
  panel-demand fences, not settings authority. Rust/Core owns each correlated
  load terminal and the returned settings snapshot, while React must distinguish
  same-room People/Profile intents (including equal snapshot generations) and
  suppress duplicate mount-effect dispatch because Rust intentionally projects no
  panel-open or settings-load Pending state. People navigation opens the pane
  before its settings read settles; a newer Threads intent retires that People
  request before either focused-context closure or settings settlement, and a
  late load may refresh only still-current data. Main-timeline sender Profile
  navigation forwards the Rust-projected `(room_id, sender user_id)`, never a
  display label, and opens Profile only after an exact room-settings snapshot;
  its renderer request/navigation fences make rapid clicks and room changes
  latest-wins without visibly opening People first. A rejected effect load may release
  only its still-current request/target marker; it must not log the raw error,
  clear a newer same-target demand, or add retry/backoff. Navigation and panel
  replacement continue to fence completion before the Rust-shaped snapshot enters
  the monotone appStore.
- SDK room-setting state events can return before the SDK room cache reflects the
  just-sent state event. The success snapshot must project the submitted setting
  change or wait for a refreshed cache; do not make React patch the visible
  room-management state after a command returns.
- Member actions render from the room-scoped
  `AppState.room_management.settings.members` snapshot, not the global profile
  cache. Member display labels, roles, and power levels are Rust-projected facts;
  React may render a select and dispatch `update_room_member_role`, but it must
  wait for the returned snapshot before the visible role changes. Kick/ban success
  removes the target in the Rust reducer; React must not locally filter the member
  row after command completion.
- Permission-guard QA must observe both the `OperationFailed(Forbidden)` event and
  the failed `room_management` snapshot; event delivery can lead the connection
  snapshot by one `StateDelta` generation.

## Room and Space navigation intent

- Rust owns every submitted `SelectRoom`/`SelectSpace` command, request terminal,
  active-room/Space state and projection. React owns only the earlier view intent:
  `roomNavigationIntentEpochRef` and `spaceNavigationIntentEpochRef` are captured
  before async composer draining and prevent an older drain, promise or panel
  follow-up from applying after a newer click. Rust cannot classify an intent that
  has not yet crossed the command boundary. Keep the epochs separate because room
  and Space/Home lifetimes have distinct settings/profile and Space-mutation
  consumers. Do not rename them back to request refs, delete them, merge them into
  one generic manager or move renderer composer-drain intent into Rust.

## Room and Space member roles

- `AppState.space_members` and room-management `RoomMemberSummary` are the
  authoritative role projections. Rust owns direct-Space membership,
  power-level revision, `can_edit_roles`, room/Space `role_options`, role
  authorization, request/generation/revision fences, and role operation/failure
  state. React renders those DTOs and dispatches the typed numeric command; it
  must not synthesize the 0/50/100 ladder from role labels, child-room
  completion, or local permission guesses. Arbitrary current levels remain a
  disabled projected option until Rust supplies an allowed target.
- The Space-members panel owns one bounded load-demand record and one panel-open
  intent epoch. The demand key is the full ready account
  (homeserver/user/device), Space id and Rust generation. It coalesces only the
  pre-projection gap before Rust's `Loading` request id is visible and retains a
  loaded marker for a legitimate empty projection. Exact record identity plus
  full live/returned fences are required before applying or settling; an old
  account completion cannot mutate or clear a newer demand. Do not restore the
  former page-lifetime Map/Set or move panel-open intent into Rust. Invite
  search has separate renderer lifetimes and converged Tauri returns. Invite
  execution has no frontend latest-request epoch: Rust's first-admitted
  `Inviting(request_id, Space, user, generation)` operation owns settlement, and
  App applies only a full-fence matching authoritative snapshot. A rapid
  duplicate rejection must not suppress the accepted success. Cancellation uses
  Rust's equivalent first-admitted operation for membership settlement: the
  renderer captures Space navigation/full fences for success and keeps a
  separately named epoch only for latest transport-failure presentation. A
  resolved Rust Failed operation still drives retry UI; non-failed settlement and
  navigation clear local failure. Role updates use the same split: Rust's
  first-admitted request owns projected success/failure, Space navigation and full
  fences gate semantic application, and `spaceMembersRoleFailureEpochRef` owns only
  current local transport-failure presentation. Panel close/reopen does not reject
  a valid result; navigation does. Non-failed settlement advances the failure epoch
  before clearing it so a duplicate rejection cannot outlive accepted success.
- The Space Members panel may own only confirmation-dialog visibility and DOM
  focus. A select remains on the projected current role until a later Rust
  snapshot projects the requested role; failure/retry leaves the authoritative
  role and options intact. Incomplete child-room sync is a notice, not a local
  disablement of a directly authorized control.
- Tauri and Browser Fake paths mirror the same command shape and admission
  guards. Browser-headless tests must exercise full projection replacement,
  failure/retry, confirmation cancellation, and role-option rederivation rather
  than patching React state after an invoke.

## Media

- Media GUI rendering is DTO-only. React may display `TimelineItem.media`
  filename/mimetype/size/dimensions/encrypted flag and `MediaUploadProgress`, but
  it must not parse Matrix event content, render MXC URIs, store downloaded bytes,
  or synthesize upload/download lifecycle state.
- `TimelinePaneState` includes `staged_uploads` and `media_gallery`. GUI code must
  render these Rust projections and dispatch typed commands only; do not keep
  upload staging/gallery maps in React, synthesize a gallery from DOM rows, or
  parse Matrix media events in the webview.
- Core owns downloaded-media save policy through `MediaSaveFilesystem`: source
  emptiness/URL/absolute checks, canonical cache/source containment, symlink and
  component-prefix rejection, destination admission, parent creation ordering,
  and copy admission. Tauri resolves the app-data cache root and selected
  destination, and supplies only the native syscall port; paths never enter
  Core state, events, commands, or diagnostics. Port and policy failures expose
  closed private-safe kinds, never paths or raw filesystem errors. Linux
  coverage uses deterministic port fakes plus a real temporary symlink escape;
  Windows junction/canonicalization and short-name assumptions remain covered
  by the hosted Windows gate rather than Core path normalization.
- Selecting a file sends source bytes through `stage_upload_bytes` and shows the
  Rust-owned Upload attachments staging dialog. Send invokes
  `send_prepared_uploads`; there is no direct renderer upload command. Each staged caption is a nullable
  `ComposerDocument`, edited through the staging dialog
  (`TimelinePaneState.staged_uploads[*].caption`), not inferred from the
  ordinary Composer draft. At the media-send boundary Rust derives the
  `FormattedMessageDraft` from that document's plain body, formatted body, and
  mention intent. Rust exclusively owns staged items, caption DTOs, residency,
  and send content. The bounded main/thread `caption:*` mutation lanes own only
  mounted-editor intent ordering through the correlated Tauri terminal; their
  keys include target/item identity and clear/send invalidates them so late
  results cannot restore removed items. Browser snapshots have no caption revision,
  so do not delete these lanes without a separately reviewed Rust editor revision.
- Rust owns image upload compression end to end: authoritative
  `SettingsValues.media.image_upload_compression` policy, source/candidate bytes,
  executor-hosted pixel transforms, original-vs-selected variant metadata,
  metadata-stripped assertion, and thumbnail-refresh assertion. Core builds the
  final `UploadMediaRequest` from the selected prepared registry entry and uses
  the actual byte-vector length rather than renderer metadata. Tauri only
  serializes staging inputs, preview bytes, and settled snapshots.
- The core media tokens prove the Rust-owned upload-staging/gallery contracts
  only; codec/canvas/native transform behavior and the visible
  drag-drop/paste/gallery/viewer workflow must be covered by browser-headless plus
  Linux virtual-display evidence.
- File attachment GUI tests must not open a native file dialog. Use the
  Composer's hidden `input[type=file][aria-label="Attach file input"]` and
  Playwright `setInputFiles()` with synthetic bytes. Locate the visible button
  with `getByRole("button", { name: "Attach file", exact: true })` because
  browsers expose file inputs with button semantics and the input label contains
  the button label as a prefix.

## Desktop viewport synchronization

Rust owns native viewport synchronization in the Tauri adapter. macOS parent
NSView bounds are authoritative; the WKWebView frame is repaired only inside
one main-thread native callback when the pure policy requires it. The receipt's
monotonic generation, repair decision, and final native/DOM alignment booleans
are the evidence boundary. `native_origin_aligned` and `native_size_aligned`
are measured again after any repair, so they never describe the stale frame.

React's viewport reporter owns only finite DOM measurement and one-shot
observation dispatch after a committed density render or browser resize. It
must not cache expected geometry, resize the native window, synthesize DOM
resize events, or add retry/timer recovery. Panel transitions remain layout-only.
The optional QA title extension is published once from the Rust receipt and
contains only generation, decision, and alignment tokens; it is disabled with
normal QA-title mode and cannot change product title semantics.

## Settings, composer, and scheduled send

- Settings product state lives in `koushi-state::AppState.settings`. GUI work may
  render it and dispatch `update_settings`, but must not make locale, theme,
  font/emoji, density, sidebar category/sort/collapse, recent emoji, or
  composer-send shortcut preferences a React or localStorage source of truth.
  Recent emoji is a Rust-canonical distinct MRU capped at 24; the picker owns only
  open/search/category/focus presentation.
- Home subsection/DM memory and per-Space local name/icon presentation are
  account-private encrypted `NavigationState`, not general settings. AppActor
  loads the current navigation store before accepting mutation/import, Core
  validates bounds and redacts the complete navigation Debug, and Rust projects
  final Space rail/header labels/icons. React may retain only text-field drafts.
- Legacy WebView keys are read only by the allowlisted migration module. It
  submits typed imports and clears each key only after persisted import-marker
  plus exact Rust snapshot proof. A marked replay removes the stale key without
  applying it; failed or stale work retains the source key.
- Notification preferences are Rust-owned `SettingsValues.notifications` product
  state and must persist through the settings store with legacy JSON backfill.
  React settings UI may dispatch typed `SettingsPatch.notifications` updates, but
  it must not keep independent local notification policy state. Browser-headless
  notification settings tests must click the visible switches and assert the
  resulting `update_settings` payload; the UI must reflect changed switch state
  only after the Rust-shaped settings snapshot updates.
- The settings file is a non-secret JSON store under the core data directory
  (`settings/settings.json`). Do not route it through the credential store and do
  not add Matrix IDs, message content, raw SDK errors, credentials, tokens,
  recovery material, SDK store keys, or search-index keys to it.
- Locale/display behavior is resolved by
  `koushi_state::resolve_locale_display_profile`. GUI components may consume the
  resulting `lang`, `dir`, catalog locale, pseudo-locale mode, platform, and
  modifier labels, but must not parse raw language tags or own fallback locale
  rules. Root `lang`/`dir` and active catalog selection come from
  `snapshot.state.locale_profile`. Raw visible strings in React components should
  fail the catalog gate unless they are reviewed structured registry data or
  synthetic fixture content.
- `LocaleDisplayProfile` and `TypographyDisplayProfile` are snapshot contract
  fields, not browser-only conveniences. `TypographyDisplayProfile` is resolved in
  Rust from `SettingsValues.typography` plus the platform profile and exposes only
  font/emoji preference and asset-status tokens. GUI code may apply those tokens
  to root attributes/CSS; it must not invent Inter/Twemoji/system fallback
  behavior per component.
- Font assets: Inter and Twemoji COLR are bundled-preferred choices with system
  fallbacks, and any included font package must update `THIRD_PARTY_NOTICES.md`
  with version, local path, license, and provenance. The current Twemoji COLR
  package (`twemoji-colr-font@15.0.3`) is pinned but npm marks it deprecated; do
  not upgrade or replace it without checking the rendered family name, license
  stack (package/font/artwork), and browser COLR/CPAL behavior.
- Keep the root font stack as a single resolved custom property, e.g.
  `font-family: var(--font-ui)`. A 2026-06-15 attempt used `font-family:
  var(--font-ui), var(--font-emoji)` with list-valued variables; headless Chromium
  rendered the page, but Playwright `locator.click()` hung at the actionability
  "visible, enabled and stable" step for ordinary buttons. Fold emoji fallbacks
  into `--font-ui` / `--font-message` instead of chaining list-valued font
  variables at the declaration site.
- Composer key behavior belongs to the Rust-owned resolver in `koushi-state`,
  shared by main, thread, and edit composer surfaces. GUI code normalizes
  DOM/native key input into typed resolver facts and then dispatches/renders the
  returned action.
- Composer send semantics also stay Rust-owned. `MentionIntent`, markdown/html
  formatting, `/me` slash-command emote conversion, and unsupported slash-command
  failures are derived in Rust/core before SDK send. React may pass typed
  draft/key/selection facts, but it must not synthesize `m.mentions`, formatted
  bodies, slash-command dispatch, or a local fallback send path when the resolver
  returns `noop` or `commitImeCandidate`. Because the resolver crosses an async
  IPC boundary, GUI key handlers must not call `preventDefault()` for
  `is_composing` key events; native IME commit owns that browser default while
  Rust still owns the product action (`CommitImeCandidate`).
- Main and thread composer draft survival is Rust-owned. React reads
  `snapshot.state.timeline.composer.draft` or the open thread composer, then
  dispatches `set_composer_draft` / `set_thread_composer_draft`; do not add a
  React-local per-room/per-thread draft map. The backing store is encrypted,
  debounced, and account-scoped in `koushi-core`; it is not serialized as a full
  draft map to the webview snapshot.
- Core exclusively allocates and validates composer renderer generations, lease
  ids, account/target scopes, and command/persistence permits. Their IPC form is
  a canonical nonzero decimal `u64`; parsing a string grants no authority.
  Tauri only parses/formats these opaque identities and keeps no counter, map,
  or mirror registry. A renderer must begin the current runtime generation,
  acquire a lease for the exact Ready account and active main/thread target,
  and pass Core's live generation/lease/scope check for every terminal permit.
- Scheduled/send-later state follows the same boundary. The full queue and local
  fallback timer are Rust/core-owned; React may render only
  `snapshot.state.timeline.scheduled_sends` for the selected room and
  `scheduled_send_capability`, then dispatch typed schedule/cancel/reschedule
  commands. MSC4140 delayed-event capability detection and
  create/cancel/reschedule requests live in `AccountActor` through SDK/Ruma APIs.
  The local fallback timer must consider only `ScheduledSendHandle::Local` items;
  server handles are owned by the homeserver and must not be fired by the local
  timer. Do not add browser timers, React-local scheduled-message maps, raw
  Matrix delayed-event calls, or logs/screenshots containing scheduled message
  bodies or server delayed-event handles.
- Browser-headless proof for scheduled send drives the real Composer `Send later`
  control and scheduled-message list, records typed `schedule_send`,
  `reschedule_scheduled_send`, and `cancel_scheduled_send` IPC calls, and verifies
  rows stay visible until a later Rust-shaped snapshot changes `scheduled_sends`:
  `cd apps/desktop && npx playwright test e2e/composer-send-queue-upload.spec.ts -g "scheduled send UI"`.

## E2EE trust

- Device verification SDK handles are actor-private resources. Keep
  `VerificationRequest` and `SasVerification` wrapped in `koushi-sdk` opaque
  handles and store them only inside `AccountActor`; snapshots, Tauri DTOs,
  TypeScript types, and React state get only `VerificationFlowState` plus
  private-data-free SAS emoji DTOs.
- Verification progress is Rust-owned. `AccountActor` listens to SDK request/SAS
  state streams and projects `VerificationSasPresented`, `VerificationCompleted`,
  or `VerificationFailed`; GUI code must not infer SAS readiness, completion, or
  cancellation from local React state.
- SAS mismatch is not a generic UI cancel. Route it as
  `VerificationCancelReason::Mismatch` so the reducer settles
  `VerificationFlowState::Failed { kind: Mismatch }` and `AccountActor` calls the
  SDK `SasVerification::mismatch()` path. Plain user decline/cancel uses
  `VerificationCancelReason::User` and returns the reducer to `Idle`.
- Incoming verification requests are discovered by the Rust `AccountActor`
  observer, not by GUI code. Follow-up verification commands must pass the
  Rust-owned `flow_id` from `AppState`; their command `request_id` is separate and
  is used only for command submission/failure correlation.
- Incoming verification observers may report the same SDK verification flow more
  than once as sync catches up. `AccountActor` must ignore duplicate incoming
  requests with the same SDK `flow_id`; only a different active flow should be
  cancelled/rejected.
- SAS peer acceptance is driven by SDK SAS state, not by React state or the SDK
  `we_started` flag. In this wrapper, `Started` is the peer side that must call
  `accept_sas_verification`; `Created` is the local side after `start_sas` and
  must not be auto-accepted.
- In same-user two-device SAS QA, keep the request direction A2 -> A and let the
  requester A2 start SAS after A accepts. Starting SAS from the accepting device
  reproduced Tuwunel `m.key_mismatch` cancellation before emoji presentation,
  while the requester-start sequence is stable.
- During the local SAS proof, keep exactly one Koushi-owned sync cursor per SDK
  client. Do not overlap an actor-owned restricted or continuous lane with manual
  `SyncOnce`; wait on typed event/state conditions instead. The ready primary may
  keep its normal owner while the provisional peer keeps its restricted owner. If
  QA needs to prove peer-device readiness, use the `qa-bin`-only read-only
  exact-device key refresh/acknowledgement, not a verification request/cancel
  probe.
- Verification observers and SDK handles must be stopped/cancelled on logout,
  account switch, and actor shutdown before dropping the Matrix session.
- Secret-bearing commands may carry an `AuthSecret` **only** inside the
  `CoreCommand::Account` command boundary: `BootstrapCrossSigning` (UIAA
  password), `EnableKeyBackup` (optional recovery passphrase), and
  `RestoreKeyBackup` (recovery secret). Their reducer actions, effects, events,
  snapshots, logs, and `Debug` output must remain secret-free.
- Secure-backup setup/passphrase-change may produce a new recovery key through
  the SDK. Do not project that key into reducer state, Tauri DTO snapshots, React
  state, logs, QA tokens, screenshots, or issue comments. Desktop recovery-key
  delivery writes through the Rust/Tauri native artifact path and reports only
  `Written`/`NotWritten` style status.
- Secure-backup setup/re-enable confirmation policy is Rust-owned. The closed
  `SecureBackupSetupIntent` must ride both reducer projection and actor command;
  Core admits it against the projected gate before actor routing and preserves
  the SDK's fresh confirmation guard. React may own only the accessible,
  catalog-backed confirmation dialog and mounted input values: cancel dispatches
  nothing and confirm sends `Reenable { confirmed: true }`. Tauri must not show
  native policy dialogs, contain hardcoded confirmation copy, or translate a
  boolean outside the typed request.
- `RestoreKeyBackup` must not be runtime gated to `SessionState::Ready` only. A
  newly logged-in device can become `NeedsRecovery` after sync discovers secret
  storage, and key-backup restore is the operation that gets it out of that
  state. Let `AccountActor` enforce that a store-backed Matrix session exists;
  `SignedOut` still fails as `SessionRequired`.
- The vendored SDK's backup-wide all-room-key download helper is private. Restore
  code must use public SDK APIs only: recover/import the secret, then hydrate
  currently joined rooms with `Backups::download_room_keys_for_room`. Do not patch
  vendored SDK just to call `download_all_room_keys` unless that patch is
  separately justified and recorded in the upstream feedback ledger. Restore
  progress in this slice counts joined-room hydration attempts; do not describe it
  as exhaustive backup-wide restore until a local homeserver QA lane proves the
  exact all-session behavior.
- Matrix identity reset can complete immediately or return an SDK auth
  continuation. Model that as Rust-owned `IdentityResetState` (`Idle`,
  `Resetting`, `AwaitingAuth`, `Failed`), not as React-local state or a nullable
  request id. `AwaitingAuth` exposes only UIAA/OAuth/unknown auth type; the SDK
  handle stays inside `AccountActor` and must be cancelled on logout, account
  switch, and actor shutdown. Auth continuation submission must be a
  `CoreCommand::Account` path that projects `ResetIdentityAuthSubmitted` through
  the reducer before actor routing. Identity-reset commands must use a fresh
  command `request_id` for submission correlation and pass the Rust-owned
  identity-reset `flow_id` from `AppState.e2ee_trust.identity_reset`.
- In production, `CoreCommand::Account` E2EE trust commands must be projected
  through the reducer before `AccountActor` routing. If this is skipped, the GUI
  can only infer pending trust state locally, violating the Rust-owned state
  machine rule.
- If a trust operation has already projected pending reducer state but the actor
  cannot complete it (session mismatch, unavailable local encryption, or an
  unimplemented SDK path), the actor must also send the matching reducer failure
  action. An `OperationFailed` event alone leaves Rust-owned pending state stuck
  and pushes recovery semantics toward the GUI.
- Login OIDC authorization is also Rust/native-owned. `AccountActor` retains the
  PKCE/state flow and replays only a same-homeserver authorization; Tauri alone
  receives the full authorization URL and launches it through the native opener.
  WebView Core-event projection contains only `request_id`, and React receives
  only coarse launch outcome plus settlement. Never return the provider URL or
  OAuth state to React or add an SSO `window.open` fallback.
- Trust GUI controls are transport clients only. Add Tauri commands as thin
  `CoreCommand::Account` submitters and keep SDK calls, UIAA/OAuth continuation
  handles, and verification handles inside Rust actors. React must render
  `snapshot.state.e2ee_trust` and dispatch typed API methods; do not add
  React-local pending/success/failure state for verification, cross-signing, key
  backup, or identity reset.
- User Settings uses Rust-owned `current_session_status` as the canonical
  read-only summary of the active session's verification, owner cross-signing,
  own identity, and key-backup readiness. `e2ee_trust` remains the owner of
  trust operations, continuation state, and action availability. Its `devices`
  projection is not a complete homeserver session inventory and an empty array
  must never be labelled `0 devices`.
- Remote account/device management is delegated to the active server. Rust/Core
  owns one optional HTTP(S) `account_management_url`, resolves it after session
  promotion through public OAuth metadata APIs or active-session well-known
  discovery, exact-session fences completion, and clears it on quarantine,
  logout, replacement, or switch. Login discovery owns no account-management
  URL. React renders **Manage account & devices** only when the Rust destination
  exists and never fetches metadata, constructs a URL, retries discovery, or
  renders local remote-device list/rename/sign-out controls.
- Verification and device DTOs include user/device ids for Rust correlation, but
  the GUI should not display those ids by default. Use ordinal/status labels
  (`Device 1`, `Verified`, etc.) unless a Rust-owned redacted display model is
  added.
- Identity-reset password/UIAA input may exist only as transient DOM input that
  is immediately sent to Tauri. Clear the input after submit, and verify the
  mocked IPC layer records password fields as `[REDACTED]`.
- When adding trust GUI tests, update `apps/desktop/src/test/appHarnessMain.tsx`
  with Rust-shaped `e2ee_trust` fixtures and command responses. Do not test trust
  success by mutating React component state; assert the returned snapshot changed
  and the expected Tauri command name/flow id was invoked.
- All visible trust labels/status text must go through
  `apps/desktop/src/i18n/messages.ts`. SDK-provided SAS emoji descriptions are not
  catalog strings; render emoji symbols or add a Rust-owned localized DTO before
  showing descriptions.

## Device-to-device verification and device cleanup

- Rust-owned `VerificationGateState.methods` is the only availability source for
  session verification. When it contains `existingDeviceSas`, the end-user UI
  offers device-to-device SAS verification; do not add a second frontend or
  build-time availability gate.
- Starting SAS always requires the existing confirmation dialog. It warns that
  the flow can be unreliable when the other device is offline, slow to sync, or
  missing keys; recommends recovery-key verification when available; and does not
  dispatch `start_own_user_sas` until the user explicitly continues.
- A SAS-only gate remains actionable. Render `gate.noRecoveryKey*` guidance only
  when recovery, bootstrap, and `existingDeviceSas` are all unavailable.
- The seven emoji and match, mismatch, and cancel actions render only from the
  Rust-owned verifying snapshot and use its `flow_id`; React must not infer SAS
  progress or settlement locally.
- The explicit `Cancel sign-in and remove this device…` path is Rust-owned
  `AppState.device_cleanup`. It is always remote-first: legacy sessions use
  password UIAA, OAuth/MAS sessions revoke through OAuth logout, and
  already-absent devices settle idempotently. A remote failure preserves the
  provisional session and local credentials for retry; erasing local data while
  the remote device may remain requires a separate confirmation.
- Device-cleanup commands are valid only while cleanup owns a retryable
  provisional/rechecking or awaiting-verification gate. Starting recovery or
  verification clears the offer; `Verifying` never admits or renders cleanup.
  React renders the Rust snapshot and submits typed commands; it must not infer
  success, delete local state, retain UIAA secrets, or auto-start cleanup after
  verification failure.
- Private-data-free diagnostics use source `device_cleanup` with stages
  `offered`, `remote_started`, `uia_required`, `uia_submitted`, `remote_settled`,
  `remote_failed`, `local_reset_started`, `local_reset_failed`, and `completed`.
- Focused checks: `cargo test -p koushi-state --test session_state`,
  `cargo test -p koushi-core --lib device_cleanup`, and
  `npm --prefix apps/desktop exec -- playwright test e2e/session-verification-gate.spec.ts -g "device cleanup" --workers=1`.
- Rich device naming and account-management presentation remain #369. Do not
  expand this cleanup state machine into that surface.

## Credential health

- Local-encryption / credential-store health is Rust-owned
  `AppState.local_encryption`; GUI code must dispatch typed
  `probe_local_encryption_health` / `reset_local_data` commands and render the
  snapshot, not infer OS/keyring semantics.
- Browser-headless Settings/Security GUI tests seed Rust-shaped
  `AppState.local_encryption` snapshots and Linux/macOS/Windows platform
  profiles. React may render the coarse status, show recovery/reset affordances,
  and dispatch the two commands; it must not read OS/keyring errors, infer
  fail-open behavior, locally change health after a click, or clean stores through
  any React-local logout path.
- `reset_local_data` is owned by Rust `AccountActor`/`StoreActor`, clears
  current-account local persistence, and returns the app to a local signed-out
  snapshot.
- Fast Tier 1 checks:

```bash
cargo test -p koushi-state --test local_encryption_state
cargo test -p koushi-key credential_backend
cargo test -p koushi-core store_actor_probe_maps_credential_backend_health_without_raw_errors
cargo test -p koushi-core reset_local_data_clears_current_account_persistence_and_signs_out_locally
```

## Japanese / CJK and i18n

- Japanese/CJK product semantics stay Rust-owned. React may render the `ja`
  catalog and Rust-owned ordering/highlight data, but it must not compute CJK
  normalization, collation, query folding, or highlight repair locally.
- CJK GUI text fitting is a presentation contract, not a product-semantic
  workaround. Long room names, member/sender names, message bodies, thread
  labels, and search snippets must keep Rust-owned text/order unchanged while CSS
  supplies `line-break: strict`, `word-break: normal`, `hyphens: none`, logical
  spacing, and width-aware ellipsis/wrapping as appropriate.
- Search/review paths for this area are: `apps/desktop/src/i18n/messages.ts`,
  `apps/desktop/src/i18n/messages.test.ts`, `apps/desktop/src/styles.css`,
  `apps/desktop/e2e/profile-settings-session.spec.ts`,
  `apps/desktop/e2e/composer-send-queue-upload.spec.ts`,
  `crates/koushi-state/src/locale_profile.rs`,
  `crates/koushi-state/tests/locale_display_profile.rs`,
  `crates/koushi-search/src/document.rs`, `crates/koushi-search/src/verify.rs`,
  `crates/koushi-search/tests/search_adapter.rs`, and
  `crates/koushi-core/src/search.rs`.
- Fast focused checks:

```bash
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
cd apps/desktop && npx playwright test e2e/profile-settings-session.spec.ts e2e/composer-send-queue-upload.spec.ts -g "Japanese locale renders shell labels and CJK text without clipping|thread and edit composers composing Enter" --workers=1
cargo test -p koushi-search --test search_adapter
cargo test -p koushi-state --test locale_display_profile
```

## GUI presentation contracts

Presentation-only rules. They never own product semantics, but each one below
has caused a real visible bug.

- GUI-only tooltips are presentation state only when their displayed text already
  comes from Rust-owned snapshots, such as `space.display_name`. Prefer the
  reusable `Tooltip` component over native `title=` for styled, testable
  tooltips: it must expose `role="tooltip"`, add `aria-describedby` only while
  open, open on hover/focus, and dismiss on mouse-leave/blur/Escape.
- Do not add ad hoc `px` literals in TSX for fixed GUI geometry. Repeated or
  semantic dimensions for rails, icon buttons, badges, avatars, counters, and
  tooltip placement belong in named CSS custom properties. Keeping `px` behind a
  token is acceptable for deliberately fixed-format controls; text-driven layout
  should prefer logical properties and scalable units where practical. Repeated
  Lucide icon `size` props are fixed GUI geometry too; centralize them in a local
  constant map instead of scattering numeric props through React.
- Event-driven `TimelineItemRow` uses the same `.message` grid contract as the
  legacy snapshot `MessageArticle`: direct child `.avatar`, `.message-main`, and
  row-level `.message-actions`. Keep direct-child grid placement explicit;
  pre-placing the actions without placing the main cell can push message content
  into the 44px avatar column and hide media titles.
- Popups anchored to a control inside a pane render in the shared body-level
  floating layer. `EmojiPicker` portals to `document.body` and positions itself
  from `resolveEmojiPickerPlacement`, which owns flip, clamp, RTL mirroring, and
  panel size. Callers pass preferences only (`placement`, `align`, and an optional
  boundary-container resolver for surfaces such as the timeline reaction picker);
  they must not compute their own placement or max size. An anchored popup is
  clipped by pane boundaries and `body { overflow: hidden }`, and two placement
  owners drift apart.
- When a popup moves into a floating layer, delete any ancestor-containment
  outside-press handler left in its former parent. A row- or pane-scoped
  `contains()` check reads the portaled panel's own controls as outside presses
  and closes the popup before the click lands; the 2026-07-25 #302 change hit
  exactly this and silently stopped `send_reaction` until the reaction
  browser-headless test caught it. Outside-press dismissal belongs to the
  component that owns the panel.
- A floating-layer popup's size is owned by the measurement it passes to
  `useFloatingPlacement`, not by CSS. `floatingPlacementStyle` writes the resolved
  `inlineSize`/`blockSize` as INLINE styles, so a stylesheet
  `min-inline-size`/`max-inline-size` on the panel is dead code — that is why the
  `--receipt-tooltip-min/max-inline-size` tokens only ever affected
  `.reaction-tooltip`. When a popup is the wrong size, fix the measurement.
- A constant `blockSize` for a variable-length list is a layout bug (#360). The
  reader popup passed a fixed 132px whatever the reader count, and because
  `.receipt-tooltip` is a grid its auto rows stretched to fill the slack: two
  readers rendered as two ~55px rows with a large blank gap between them. Derive
  the block size from the row count (`receiptPopupBlockSize`) and pair it with
  `align-content: start` so residual slack can never stretch rows. Keep the JS
  row/gap/padding constants in step with the matching `--receipt-tooltip-*` CSS
  tokens.
- Assert popup compactness on measured slack, not on a guessed total. The useful
  invariant is `popupHeight - rowsHeight - (padding + border + row gaps) <
  lineHeight`; reading padding/border/`rowGap` from `getComputedStyle` keeps the
  test honest when a token changes. A raw `popupHeight < rowsHeight + lineHeight`
  bound looks equivalent and fails on correct output.
- Rendered geometry that keyboard navigation depends on needs one owner.
  `EMOJI_PICKER_GRID_COLUMNS` feeds both `--emoji-picker-columns` and the
  ArrowUp/ArrowDown step, and `styles.contract.test.ts` pins the CSS fallback to
  that constant so the grid and the keyboard step cannot disagree.
