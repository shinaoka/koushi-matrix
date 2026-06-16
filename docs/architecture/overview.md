# Matrix Desktop Architecture Overview

Status: normative. This is the long-term blueprint for the whole application.
Dated specs and plans under `docs/superpowers/` are implementation guides
toward this document and must not contradict it. Amend this document first
when a design change is needed, then update or supersede the affected specs.

Last amended: 2026-06-16.

## Product Scope

A Windows/macOS desktop Matrix client following Element X's Rust SDK direction
with an Element Desktop/Web-like three-pane desktop UX:

- Shell: Tauri v2. Frontend: React + TypeScript. Backend: Rust on
  `matrix-sdk` / `matrix-sdk-ui`.
- First version: E2EE text chat, Spaces, room timelines, threads, desktop
  interaction, encrypted ngram full-text search (CJK-capable).
- Out of scope for MVP: voice/video calls, screen sharing, bots, widgets,
  app integrations.
- DMs are global account-level conversations (Element X Android-style
  two-member classification), never duplicated under Spaces.
- A browser-hosted build (Element Web-like deployment of the same core) is a
  potential future target. It is not scheduled, but the architecture must not
  preclude it; see Platform Portability.

## Layers

```text
React UI (apps/desktop)                     presentation only
        |  typed client calls / snapshots / events
Tauri adapter (apps/desktop/src-tauri)      transport only
        |  CoreCommand -> / <- CoreEvent, AppStateSnapshot
matrix-desktop-core                         the ONLY production runtime owner
        |  actors own SDK handles, tasks, projection
matrix-desktop-sdk                         thin matrix-rust-sdk adapter
matrix-desktop-state                        pure reducer + snapshot DTOs
matrix-desktop-search / matrix-desktop-key  search verification / credential store
        |
matrix-rust-sdk (vendored)                  sync, timeline, send queue, crypto
```

Crate responsibilities:

- `matrix-desktop-state` — pure. `AppState`, `AppAction`, `reduce()`,
  serializable snapshot DTOs. No SDK handles, no Tauri, no async.
  E2EE trust, verification, cross-signing, key-backup, and identity-reset UI
  state is modeled here as guarded, request-correlated state. GUI code renders
  that state; it does not own trust decisions. Personal local user aliases are
  also Rust-owned profile state: `ProfileState.local_aliases` stores the
  account-data-backed map, reducer actions own set/clear/list lifecycle, and
  display-name resolution follows `alias ?? upstream display name ?? MXID`
  before React sees labels. Timeline relabeling after profile or alias changes
  is also a Rust-owned `CoreEvent::Timeline` patch stream; React may match rows
  by raw identity fields and apply Rust-provided labels, but it must not
  recompute alias precedence. React may render the DTO and dispatch typed alias
  commands only; it must not maintain a separate alias cache or write aliases to
  Matrix profile/events.
- `SettingsState` is serializable Rust product state owned by
  `matrix-desktop-state` and persisted by `matrix-desktop-core` through a
  non-secret settings store. React may apply settings to presentation, but it
  must not be the source of truth for locale, theme, font/emoji choice, or
  composer send shortcut semantics. Locale/display profile resolution is also
  Rust-owned; GUI code consumes the resolved profile and catalog selector
  defined in `docs/architecture/i18n.md` rather than parsing raw language tags.
  Font/emoji display profile resolution is likewise Rust-owned:
  `matrix-desktop-state` resolves `TypographyDisplayProfile` from
  `SettingsValues.typography` and the platform profile, and the frontend may
  only apply the resulting font, emoji, and asset-status tokens to root
  attributes/CSS. Inter and Twemoji COLR are bundled-preferred choices with
  system fallbacks; React must not choose fallback semantics per component.
  Composer key handling uses the pure Rust-owned resolver in
  `matrix-desktop-state`; GUI code supplies typed key facts and
  renders/dispatches the resolved action. Because the resolver may cross an
  async transport boundary, GUI code captures key facts and textarea selection
  synchronously, prevents default only for resolver-owned keys, and applies
  newline/send/cancel only from the returned action. Resolver failures are
  no-ops; React must not fall back to local send semantics. Composition key
  events keep the native browser default so IME candidate commits are not
  blocked by the async resolver boundary; Rust still owns the returned product
  action (`CommitImeCandidate`). Composer send payload semantics are also owned
  by Rust/core: intentional mentions are typed `MentionIntent` data,
  markdown/html and `/me` emote conversion are built before SDK send, and
  unsupported slash commands fail locally with structured private-data-free
  failure kinds. React does not construct `m.mentions`, formatted bodies, or
  slash-command dispatch.
  Room management is likewise Rust-owned: room settings snapshots, room-scoped
  member summaries, permission facts, setting changes, power-level role edits,
  and kick/ban/unban moderation operations live in
  `AppState.room_management` and `RoomCommand` / `RoomEvent`. React renders
  `settings.permissions`, `settings.avatar_url`, and `settings.members`
  including Rust-projected member display labels, role facts, and power facts,
  and dispatches typed commands only; it must not decide whether a user can edit
  settings, edit roles, or moderate members locally.
  Core Batch A0 ownership also lives in this crate: local encryption /
  credential-store health, native attention candidates and capabilities,
  Japanese/CJK display/search policy, and backup restore scope are
  serializable Rust state or DTO contracts. React renders those contracts and
  dispatches typed commands; it does not decide credential health, notification
  eligibility, CJK collation/normalization, IME send-vs-commit behavior, or
  whether key-backup restore is complete.
- `matrix-desktop-sdk` — low-level SDK adapter (login, restore, recovery,
  sync, room, timeline, search primitives). No app state, no QA orchestration.
  E2EE key-backup restore wrappers consume recovery secrets internally and
  return private-data-free restore summaries whose scope is explicitly
  `JoinedRooms`; they do not expose SDK backup keys, room keys, or raw backup
  versions across the command/event boundary. The MVP restore scope is
  recovery secret import plus currently joined-room key hydration through
  public SDK APIs. Product state, QA evidence, and UI copy must not claim
  exhaustive backup-wide restore until a public SDK API or reviewed vendored
  patch proves that broader scope.
- `matrix-desktop-core` — actor lifecycle, command routing, event emission,
  SDK session handles, background tasks, AppState projection, headless QA
  binaries. Production Matrix behavior lives here and nowhere else.
- `matrix-desktop-backend` — fixture/demo data only. Never on a production
  Matrix path.
- `matrix-desktop-key` — OS credential store, key derivation (HKDF from the
  local unlock secret), zeroizing secret wrappers.
- `matrix-desktop-search` — candidate verification, document store, index
  maintenance queue.
- `apps/desktop/src-tauri` — transport adapter. Holds a `CoreRuntime`, sends
  commands, forwards events/snapshots. No direct SDK wrapper calls.
- `apps/desktop` — view and interaction code only, including viewport state,
  DOM measurement, and scroll anchoring.

Upstream SDK deltas are carried in the
`github.com/shinaoka/matrix-rust-sdk-work` submodule branch
(`shinaoka/search-ngram`). Local comments document the patch surfaces, and
`docs/upstream/matrix-rust-sdk-feedback.md` remains the ledger for PR
candidate material. The vendored fork already contains the behavior/API deltas
needed for the current search and runtime work; Phase 9 added explanatory
comments and management docs, not new runtime behavior. The Phase 9 cleanup
follow-up completed the SDK adapter rename, room lifecycle commands, runtime
IPC contract drift check, and optional AppCommand decision.

GUI, Tauri, CLI, and QA all use the same command/event boundary. There is no
standalone daemon; the runtime is in-process.

## Platform Portability

The desktop app is the only shipping target today, but a browser-hosted build
of the same core (Element Web-like) is a plausible future. matrix-rust-sdk
already supports `wasm32` (executor abstraction over tokio /
`wasm_bindgen_futures`, IndexedDB store besides SQLite), so portability is
decided by our own code discipline, not by the SDK. These rules keep the
option open at near-zero ongoing cost; retrofitting them later would mean
rewriting the runtime.

1. **The command/event boundary is transport-neutral.** `CoreCommand`,
   `CoreEvent`, and `AppStateSnapshot` are serde-serializable and contain no
   Tauri, OS, or filesystem types. Tauri IPC is one transport; a WebWorker
   `postMessage` / wasm-bindgen bridge must be addable as another without
   touching core types.
2. **Core logic uses executor abstractions, not tokio directly.** Task spawn,
   timers, and timeouts in `matrix-desktop-core` go through the SDK's
   executor layer (`matrix_sdk_common::executor`) or a thin core-owned
   wrapper. No `tokio::spawn`/`tokio::time` calls scattered through actor
   logic; no thread-blocking (`block_on`, blocking locks held across await)
   inside actors. The actor runtime must be able to run on a single-threaded
   executor (wasm) as well as multi-threaded tokio.
3. **Platform capabilities live behind ports, owned by `StoreActor` and the
   adapters.** OS credential store (`keyring`), filesystem paths, SQLite
   store config, and process/OS APIs appear only behind traits with platform
   backends (today: OS keychain + SQLite; browser later: WebCrypto-derived
   keys + IndexedDB). `StoreActor` is the only actor allowed
   platform-conditional code. The fail-closed local-encryption rule still
   applies on every platform: a weaker browser at-rest story must be an
   explicit, surfaced property, never a silent fallback.
4. **Pure crates stay wasm-clean.** `matrix-desktop-state` and
   `matrix-desktop-search` must compile for `wasm32-unknown-unknown`; a CI
   check target should enforce this once wired. `matrix-desktop-core`'s
   portability is enforced structurally by rules 1–3 until a web spike makes
   a wasm CI check for it practical.
5. **Known open items for a web target** (recorded, not designed): ngram
   search index backend on wasm, credential storage UX without an OS
   keychain, and multi-tab/single-runtime coordination. None of these may be
   solved by weakening the desktop security model.

## Runtime Model

An in-process actor system in `matrix-desktop-core`:

- `AppActor` — command entry point, routing, active account, ordered event
  broadcast and snapshots. It also owns the account-wide Activity projection
  cache: room timeline actors may report message rows, but Recent/Unread
  ordering, unread membership, low-priority exclusion, and mark-read clearing
  are materialized into `AppState.activity` by Rust before React sees them.
- `AccountActor` (per account/device) — SDK session ownership,
  login/restore/recovery/logout, account switch, child shutdown.
- `SyncActor` — continuous sync lifecycle
  (starting/running/reconnecting/failed/stopped).
- `RoomActor` — room list normalization
  (`SpaceSummary`/`RoomSummary`/`InvitePreview`), create/invite/join/space
  operations, invite accept/decline, DM start, public directory query and
  join-by-alias, unread counts, DM classification, and Matrix room tags
  (`m.tag` favourite / low priority).
  On the sliding-sync backend it consumes the one `RoomListService` owned by
  the running `SyncService`; constructing additional ad-hoc
  `RoomListService` instances is prohibited — they are not driven by the
  sync loop, race it, and return entries without the `required_state`
  (e.g. `m.room.create` for space classification) the live service
  requests. Its live entries adapter uses a non-left filter so invited-room
  diffs also wake Rust-owned invite projection; joined-only observation leaves
  `AppState.invites` stale. Room tags are projected into
  `RoomSummary.tags` by the same Rust-owned room-list normalization path, and
  sidebar unread/mention affordances consume Rust-owned unread/highlight counts
  from `SidebarModel`. React must not derive favourite, low-priority, unread,
  or mention membership from local UI state.
- `TimelineActor` (per room/thread/focused timeline) — subscription, diffs,
  pagination, send/edit/redaction relay, reaction annotation projection and
  guarded send/redact relay, media/file projection, upload progress,
  room-scoped live signals, and Rust-only media download effects.
  Room live timelines use
  `TimelineFocus::Live { hide_threaded_events: true }` so threaded replies
  are hidden from the main room timeline. Expanded threads use
  `TimelineKind::Thread`. On the sliding-sync backend,
  subscribing a timeline also subscribes its room with the live
  `RoomListService` (`subscribe_to_rooms`, the Element X room-open pattern):
  the all-rooms list alone only guarantees the initial window on some
  servers. Thread backward pagination uses the same `TimelineKind::Thread {
  room_id, root_event_id }` key as the thread subscription. Edits and
  plain sends, edits, and redactions go through the SDK `Timeline` handle (not
  direct room/send-queue calls) so their diffs are produced as local echoes
  instead of depending on the server echoing them back; for own sent events whose
  remote echo has not arrived, the actor resolves the event id back to the
  local-echo transaction identity. Media messages are projected into
  `TimelineItem.media` from SDK message content. React renders that DTO only:
  it does not infer Matrix media semantics, upload state, encrypted media
  metadata, or download behavior. Downloaded bytes and encrypted media keys or
  hashes stay inside Rust actor effects and are never sent through CoreEvents.
  Reaction groups are projected the same way from SDK aggregation data; React
  renders the grouped DTO and dispatches typed reaction commands only, while
  Rust guards current state before delegating to the SDK toggle helper.
  Reply quote previews are projected into `TimelineItem.reply_quote`; React
  renders the quote state and does not resolve Matrix reply bodies. Pinned
  events live in `AppState.room_interactions`, and pin/unpin commands route
  through `RoomActor` before the Rust snapshot/event stream updates the GUI.
  Read receipts, fully-read markers, and typing notifications are projected
  from SDK timeline/room signals into `AppState.live_signals`; React may render
  that snapshot and dispatch typed commands, but it must not synthesize receipt,
  marker, or typing lifecycle locally. Receipt reader avatars are part of this
  Rust-owned projection: reducers resolve reader display labels and avatar DTOs
  from profile state, order readers most-recent-first, cap the rendered reader
  list, and expose an overflow count before the data reaches `TimelineView`.
  React must not join receipt user ids with profile maps or choose receipt
  ordering locally.
- Account-wide Activity is projected in `AppActor` from Rust-owned timeline
  observations plus room unread/tag summaries. `TimelineView` and focused
  timelines remain event-driven render surfaces; they do not own the Activity
  state machine. React dispatches typed Activity commands and focused-context
  opens using event references supplied by Rust.
- Account-level live signals such as presence are Rust-owned state in
  `AppState.live_signals.presence`. In the current Phase A contract,
  `AccountCommand::SetPresence` records the requested presence and emits typed
  `LiveSignalsEvent` updates. Network presence propagation is sync-backend
  policy: the legacy SDK path uses `SyncSettings::set_presence`, while the
  current `SyncService` builder in the vendored SDK has no direct presence
  setter. Do not move presence semantics into React while that SDK/API decision
  remains open.
- `SearchActor` — ngram candidates, canonical-text verification,
  document-level index mutations for edits/redactions/late decryptions.
- `StoreActor` — credential store access, store/search keys, per-account
  paths, cleanup, debug/test secret injection policy.

**Account store bootstrap invariant.** Per-account store paths derive from
`homeserver|user|device`, so the device id — and therefore the store path —
is unknown until the password exchange completes. First login therefore runs
on a storeless client, and that client must never sync or initialize
encryption: immediately after login the session is persisted and restored
into the per-account encrypted store, and only the store-backed session may
start sync or any E2EE traffic. This preserves the device's crypto identity
across restarts; the fail-closed local-encryption rule applies to the store
creation step. `SwitchAccount` is the ordered shutdown of the current
account runtime (without clearing credentials or stores) followed by a
store-backed restore of the target account; phases that do not yet have the
affected children treat those shutdown steps as no-ops.

Actor deployment is flexible. The boundaries above define state ownership,
command routing, event production, and shutdown responsibility; they do not
require one Tokio task per actor in the first implementation. The runtime may
colocate child loops under `AccountActor` while preserving the same public
contracts and resource ownership.

Supervision follows the same ownership tree:

- `AppActor` owns account runtimes; each `AccountActor` owns its child task
  handles and subscription handles.
- Expected SDK failures are reported through domain state (`SyncFailed`,
  pagination failure, search failure) and redacted `OperationFailed` events.
- A child task panic or unexpected join error tears down only that child when
  the SDK handle can be safely recreated (`TimelineActor`, `SearchActor`) and
  emits a failure with a new generation marker. `SyncActor` crashes move the
  account to `SyncFailed`; the SDK's normal reconnect loop handles network
  churn, while an internal crash requires an explicit `SyncCommand::Restart`
  or account restore path.
- `AccountActor` failure is fatal to that account runtime: stop children,
  drop SDK handles in runtime context, emit a redacted account failure, and
  require restore/login rather than silently continuing with unknown state.
- Hangs are detected per command by request deadlines and missing required
  progress. Idle timeline or sync streams are valid states, not hangs.

State projection keeps the reducer as the single UI state transition
mechanism:

```text
CoreCommand -> actor side effect -> CoreEvent -> AppAction
            -> reduce(AppState) -> StateChanged(AppStateSnapshot)
```

`AppState` contains only serializable UI data. SDK handles, task handles,
subscriptions, and keys live in actor-owned runtime state.

Core identity types are concrete and stable:

```rust
pub struct RuntimeConnectionId(pub u64);

pub struct RequestId {
    pub connection_id: RuntimeConnectionId,
    pub sequence: u64,
}

pub struct TimelineKey {
    pub account_key: AccountKey,
    pub kind: TimelineKind,
}

pub enum TimelineKind {
    Room { room_id: String },
    Thread { room_id: String, root_event_id: String },
    Focused { room_id: String, event_id: String },
}

pub enum PaginationDirection {
    Backward,
    Forward,
}

pub enum PaginationState {
    Idle,
    Paginating,
    EndReached,
    Failed { kind: TimelineFailureKind },
}

pub enum TimelineFailureKind {
    InvalidDirection,
    NotSubscribed,
    Forbidden,
    Network,
    Timeout,
    Sdk,
    QueueOverflow,
}
```

Timeline item events carry app-owned DTOs. `TimelineItem` includes stable
identity, sender/body/timestamp fields, `in_reply_to_event_id`,
`reply_quote`, reactions and edit/redact affordances, plus thread fields:
`thread_root: Option<String>` for items that are in a thread, and
`thread_summary: Option<ThreadSummaryDto>` on thread root items.
`ThreadSummaryDto` contains `reply_count`, `latest_sender`,
`latest_body_preview`, and `latest_timestamp_ms`; the `latest_*` fields are
`None` when the SDK has not loaded the latest event details.

The runtime assigns each attached consumer a `RuntimeConnectionId`; the
attached connection allocates a monotonically increasing `sequence` within that
connection. The full `RequestId` is therefore unique on the shared event
stream, and consumers correlate by the full value. The command transport wraps
each inbound command with the connection it arrived on. A command whose
`request_id.connection_id` does not match that transport connection is rejected
before routing and before any `CoreEvent` is published; it is a local
`CommandSubmitError::InvalidRequestId`, not an `OperationFailed` with the
forged `RequestId`. `TimelineKey` always includes the account so late
events from a previous account switch can be rejected. Timeline item events
also carry a monotonic `generation`; after any reset/resync the UI discards
diffs from older generations.

## Async Design Rules

These rules are normative for all core runtime code. They exist because
matrix-rust-sdk is designed around cloneable handles and observable streams
(`Timeline::subscribe()` returning `Vector` + batched `VectorDiff` stream,
`SyncService` state observable when MSC4186 is available, send-queue update
stream), and the runtime must relay that model, not fight it.

1. **Actors relay the SDK; they do not reimplement it.** An actor owns SDK
   handles and subscriptions, converts observable updates into `CoreEvent`s,
   and manages lifecycle. Concurrency the SDK already provides — pagination
   coalescing, send-queue persistence and retry, sync service reconnection —
   must not be duplicated in actor logic.
2. **Commands never return Matrix data.** A connection send call may report
   only local submission errors before acceptance, such as a closed runtime or
   invalid request ID. Accepted command results are observed as events and
   snapshots so that GUI, CLI, and QA observe identical behavior.
3. **Every accepted command carries a runtime-scoped `request_id`.** Every
   accepted command result event carries that same full `request_id`, whether
   the result is success or failure. Failures are emitted as
   `OperationFailed { request_id, failure }`; successes such as room creation,
   join, send completion, pagination state changes, and search completion carry
   `request_id` in their domain event. Events that can also occur without a
   client command — e.g. pagination state transitions triggered by SDK
   coalescing or sync gap-fill — carry the originating `request_id` when one
   exists (`Option<RequestId>`). A command with a forged or mismatched
   `connection_id` is not accepted and is rejected as a local submission error
   before it can publish another consumer's `RequestId` on the shared stream.
   Message sends additionally carry a `transaction_id` used for local-echo
   matching end to end.
4. **Timeline data flows as diffs, not snapshots.** Timeline items are
   delivered as an initial item set plus `VectorDiff`-shaped update events per
   timeline. `AppState` snapshots must not embed full timeline item lists;
   re-serializing a timeline on every change does not scale to scroll-back.
   The UI applies diffs and may therefore implement stable scroll anchoring
   on prepend. Matrix replacement events (`m.replace`) are separate events from
   the original message. The runtime preserves both identities, keeps pending
   edit relationships when an edit is visible before its original event, and
   reprojects the original item and mutates only its affected search document
   when the missing original, a late edit, redaction, or decryption result
   arrives. Replacement events whose
   original is missing are exposed as unresolved edit relations, not as ordinary
   standalone messages. Timeline-side edit aggregation itself comes from the
   SDK — edits arrive as diffs on the original item (rule 1); the obligations
   above bind the runtime's projection and the search pipeline, which keeps its
   own pending-edit relations, and are not a reimplementation of SDK
   aggregation.
5. **Pagination is stateful, directional, and observable.** Every timeline
   exposes per-direction pagination state events: `Idle`, `Paginating`,
   `EndReached` (timeline start/end hit), `Failed(kind)`. The UI uses these
   to drive spinners and to suppress duplicate pagination requests while one
   is in flight. Backward pagination is valid on every timeline kind; forward
   pagination is valid only on non-live (`Focused`) timelines — on live
   timelines the forward edge comes from sync. The runtime relays the SDK's
   pagination status; reaching the start of history must be surfaced, or the
   UI will paginate forever.
6. **Timelines are addressed by `TimelineKey`, not bare room IDs.** A
   `TimelineKey` identifies a room live timeline, a thread timeline, or an
   event-focused timeline (`TimelineKind`). Subscribe, unsubscribe, paginate,
   send, edit, and redact all take a `TimelineKey`, so threads paginate and operate
   identically to rooms.
7. **Subscriptions have explicit lifecycles.** Every subscribe has a matching
   unsubscribe command. Unsubscribing (or account shutdown) drops the SDK
   timeline handle, which cancels its background tasks. Room switching policy
   (drop immediately vs. keep-warm) is decided by the UI through these
   commands; the runtime never leaks timeline state in an unbounded map.
8. **Sends go through the SDK timeline/send queue path.** Local echo, offline
   persistence, strict FIFO retry, retry-after-reconnect, and remote-echo
   matching come from the SDK send queue, reached through the SDK UI timeline
   handle for visible timeline sends. The Rust runtime owns the product state
   projection:
   `TimelineItem.send_state`, transaction-id keyed retry/cancel guards, and
   `RetrySend` / `CancelSend` command routing through SDK `SendHandle`s. After
   recoverable send errors, retry/cancel also re-enable the SDK room queue so
   FIFO successors are not stranded. React renders and dispatches only; it must
   not infer send legality or repair queue state locally. `Transaction`
   timeline identities are stable local-echo keys only; visible failed/sending
   state comes from `TimelineItem.send_state`. The runtime does not serialize
   sends behind a command loop.
9. **Sync uses capability-probed SDK services, not ad hoc polling.** Prefer
   `SyncService`/`RoomListService` when the homeserver supports MSC4186. If
   `SyncService` is unavailable for a target homeserver, the `SyncActor`
   switches to an explicit `LegacySync` backend using SDK `/sync` primitives
   while preserving the same `CoreCommand`/`CoreEvent` contract. The selected
   backend is emitted as a redacted diagnostic/event field so QA can assert it.
   `sync_once`-style one-shot polling remains a QA/debug tool, not the product
   continuous-sync path. Because legacy `/sync` works against any homeserver,
   a debug/test-only override (compile-time gated out of release builds)
   forces the `LegacySync` backend so local QA exercises the legacy path even
   against MSC4186-capable servers — both local QA servers advertise MSC4186,
   so without the override the fallback would never run before the real
   homeserver gate. Note that `RoomListService` is built on sliding sync:
   on the `LegacySync` backend, `RoomActor` must normalize the room list from
   legacy sync state (`Client::rooms()` plus sync updates) instead. Because the
   local QA matrix includes homeservers without MSC4186, this legacy room-list
   path is a fully implemented, QA-gated product path, not a stub. Invite
   projection is part of the same contract: both sync backends must produce
   `AppState.invites` from SDK invited rooms, not from React-local state.
10. **Backpressure is defined, not accidental.** The event channel policy is
    explicit: state snapshots are latest-wins (watch semantics, coalesced to
    at most one `StateChanged` per batch), discrete events use bounded
    channels with a defined recovery path (drop + full snapshot resync). A
    slow UI must not stall the core or grow memory without bound.
11. **SDK handles are dropped inside a Tokio runtime context.** Store-backed
    SDK clients panic (`deadpool-runtime`) when dropped outside one. Shutdown
    paths and QA binaries must respect this.
12. **Shutdown is ordered**: stop accepting commands → stop timeline
    subscriptions → stop search queues → stop sync → persist session state →
    drop SDK handles → (on logout/removal) clear credentials and stores →
    emit final `StateChanged`.

### Desktop Window Model

The product runtime is single-window for now. The native shell creates and
restores one Tauri webview window labelled `main`, and one process-wide
`CoreRuntime` owns command dispatch, event forwarding, QA title state, and
window-state persistence. Opening additional product windows is out of scope
until a later explicit design defines per-window navigation, timeline
subscriptions, QA title ownership, persisted geometry, and shutdown behavior.
Secondary OS dialogs or system prompts do not change this product-window
contract.

### Desktop Attention Surfaces

Desktop notifications, dock/taskbar badges, and unread window-title hints are
derived interaction surfaces. They do not own Matrix behavior and must be
computed from the same serializable `AppState` projection used by the UI.
Core/state may expose a notification decision surface, but it contains only
allowed UI metadata: a safe room display label, notification kind
(`mention`, `dm`, or `message`), unread notification/highlight counts, and the
coarse unread total. It must not contain message bodies, sender identifiers,
room IDs, event IDs, transaction IDs, raw SDK errors, or secrets.

The Tauri adapter maps that transport-neutral surface to platform capabilities
such as OS notifications, badge counts, and window-title updates. The redacted
notification content policy is fail-closed: message bodies are excluded by
default, and any future preview option requires an explicit settings design and
new tests. Private-data-free QA title tokens may expose only aggregate values
such as `unread=N`, `badge=N`, and `notify=<kind|none>`.

Native attention is Rust-owned candidate data plus a platform capability
profile. The core decides whether a room, thread, mention, focus change, or
read-marker transition creates, suppresses, updates, or clears an attention
candidate. The adapter may only map that private-data-minimized candidate to
macOS, Windows, Linux, or no-op capabilities; React must not branch on platform
notification semantics or synthesize badge/window-title state locally.
Persistent title, badge, overlay, tray, and clear hooks follow the Rust-owned
snapshot. Sound and activation hooks are candidate-scoped transient effects, so
they run only for a Rust-owned notification candidate and not for every later
snapshot that still contains unread state.
Pane-level thread attention is also Rust-owned: `AppState.thread_attention`
tracks the open thread's notification, highlight, and live-event marker counts
and reaches React only through the Tauri/TypeScript DTO.
User notification preferences are the same boundary: `SettingsValues.notifications`
is the Rust-owned persisted source of truth, and legacy settings files backfill
the default policy before any GUI reads the snapshot.

Initial channel capacities are named constants, not scattered literals:

- command inbox per runtime: 256
- discrete core events per consumer: 1024
- timeline diff batches per subscribed timeline: 128
- search index mutation queue: 512

If a bounded event or diff queue overflows, the runtime marks that consumer or
timeline generation dirty, drops further incremental diffs for that generation,
and emits a reset/resync event once the queue can accept it. The UI then
requests or receives the latest snapshot/initial item set and resumes on the
new generation. Queue overflow must never silently lose a Matrix event while
continuing to apply later diffs as if the stream were complete.

## Timeline Viewport And Scrollback

Timeline scrollback uses a two-layer contract: core owns Matrix ordering,
subscriptions, diffs, and pagination state; React owns render lists, viewport
measurement, and DOM anchoring.

Runtime responsibilities:

- Emit an initial item set followed by FIFO, `VectorDiff`-shaped diff batches.
  Diff batches preserve positional operations (`PushFront`, `PushBack`,
  `Insert`, `Set`, `Remove`, `Truncate`, `Clear`, `Reset`) closely enough that
  the UI can distinguish prepend pagination from live append/update/remove.
- Emit pagination state changes with `TimelineKey`, direction, state, and
  `Option<RequestId>`: `Idle`, `Paginating`, `EndReached`, `Failed(kind)`.
- Treat a pagination command as data-complete when the SDK has produced the
  diff batch or end/failure state. The core does not wait for React rendering or
  DOM measurement, because it has no DOM.
- Provide stable item identity for every renderable item: remote event ID when
  known, transaction ID for local echo, and stable synthetic IDs for separators
  or virtual items. A remote echo replaces the local transaction identity through
  an explicit diff/update, not by changing a React key in place.

UI responsibilities:

- Maintain the render list and viewport model per `TimelineKey`; full timeline
  lists are not copied into `AppState`.
- A Tauri `InitialItems` event can be emitted before React remounts the
  corresponding `TimelineView`. If the first observed event for a key is a live
  `ItemsUpdated` batch and no resync is pending, initialize that key from an
  empty render list and apply the diff. After `ResyncRequired` or
  `ResyncMarker`, continue to require a fresh `InitialItems`.
- Before a backward pagination request can affect the viewport, capture an
  anchor item (first visible stable item ID plus pixel offset, or an equivalent
  bottom-aligned strategy). After applying the diff and after React commits the
  DOM update, restore that anchor in `requestAnimationFrame`/layout effect.
- Do not issue the next automatic fill request until the previous diff has been
  applied and anchor restoration for that generation has completed.
- Treat scroll position, measured heights, overscan windows, and virtual-list
  cache as UI state. These values never cross into core and never affect Matrix
  ordering.

Headless QA proves the data contract: request correlation, pagination states,
diff order, generation reset, replacement/redaction/late-decryption handling.
GUI smoke proves the DOM contract: scrolling back prepends older items without
jumping, live appends do not steal the viewport while scrolled up, and end-of
history stops further automatic pagination.

## Security Model

Full prohibitions live in
[REPOSITORY_RULES.md](../../REPOSITORY_RULES.md) and the detailed policy
extension in
[docs/policies/engineering-rules.md](../policies/engineering-rules.md). The
architectural invariants:

- **Secret classes.** Passwords, recovery material, access tokens, SDK store
  keys, and search index keys never appear in logs, `Debug` output, events,
  `AppState`, committed files, or ordinary test fixtures. Secret-bearing
  types use zeroizing wrappers with redacted `Debug`.
- **Key ownership.** `StoreActor` owns store and search keys, derived per
  account (HKDF from the local unlock secret kept in the OS credential
  store). Keys never cross the command/event boundary.
- **Local encryption is fail-closed.** If the OS credential store, SDK store
  encryption, or search index encryption cannot be initialized, the core refuses
  login/restore/startup for that account and emits a redacted
  `LocalEncryptionUnavailable` failure. There is no production fallback to
  plaintext stores or plaintext search indexes. Credential-store health is
  reported as one of the Rust-owned coarse states `unknown`, `healthy`,
  `unavailable`, `locked_or_inaccessible`, `missing_credential`, or
  `reset_required`; raw OS/keyring errors never cross into snapshots, logs, or
  UI decisions.
- **Webview threat model.** The React webview is the least-trusted layer.
  Secrets entered there (password, recovery key) flow one way: webview →
  Tauri IPC → core. The core never returns secret material to the webview.
  Release builds disable devtools, ship a strict CSP, and must not trace
  Tauri IPC payloads. JS strings cannot be zeroized; minimizing secret
  residency in the webview is a design obligation, not an optimization.
- **Coarse public failures.** Public errors are redacted (`CoreFailure`)
  but carry a non-secret `kind` per category (e.g. invalid credentials /
  network / rate-limited / server) so the UI never needs raw SDK errors.
  Raw SDK errors appear only behind an explicit debug/test diagnostic
  switch.
- **Production credential gates.** Release builds reject
  environment-variable credential injection and the file-based credential
  store; these are compile-time gated to debug/test and verified by CI, not
  merely by `debug_assertions`.
- **Search.** The ngram index is encrypted with its own key and is a
  candidate generator only; results are emitted after verification against
  canonical visible text, so index false positives never surface content.
  Timeline edits, redactions, and late decryptions are document-level index
  mutations, not append-only events and not full reindex operations: an edit
  updates only the affected document by removing terms for the previous
  canonical visible text and indexing the replacement text, a redaction removes
  only the redacted document from the searchable corpus, and an unresolved
  replacement event is not indexed as a standalone message.
- **Device verification, cross-signing, key backup, and identity reset** are
  release-blocking E2EE trust work. Issue #13 Phase A establishes the
  Rust-owned reducer state and typed `CoreCommand`/`CoreEvent` surface.
  Production `CoreCommand::Account` trust commands project reducer pending
  state before routing to `AccountActor`, so GUI work observes Rust-owned
  progress rather than inventing pending/settle semantics. SDK-backed actor
  slices wire cross-signing bootstrap, key-backup enable/restore, identity
  reset, and outgoing device verification through `matrix-desktop-sdk`
  private-data-free wrappers. Identity reset and verification continuation
  handles are held only by `AccountActor`; SDK request/SAS streams settle the
  reducer with typed actions and expose only private-data-free DTOs such as SAS
  emojis. Incoming verification request discovery is Rust-owned in
  `AccountActor`. The local core `e2ee_trust` proof exercises same-user
  two-device SAS verification, cross-signing bootstrap, passphrase-backed
  key-backup enable, encrypted seed-room backup upload, wrong-secret restore
  failure, successful joined-room restore on the second device, and identity
  reset on disposable local homeservers through the probed SyncService core leg
  before GUI wiring. No design doc may claim exhaustive backup-wide restore
  until the exact supported restore scope is proven or split into an explicit
  follow-up.

## QA Model

QA is layered; GUI automation is the last and weakest layer, never the
primary correctness gate.

1. **Unit tests** — network-free: routing, redaction, unauthenticated command
   rejection, state transitions with fake ports, normalization, reducer.
2. **Local homeserver QA** — disposable Conduit/Tuwunel servers, synthetic
   users, a core QA binary speaking `CoreCommand`/`CoreEvent` (never direct
   SDK wrapper calls). Covers login, sync, room/space create, invite receipt,
   invite accept/decline, DM start, bidirectional messaging, room list, logout
   cleanup, and stdout/stderr redaction. It records and asserts the selected
   sync backend
   (`SyncService` or `LegacySync`) so server capability gaps are visible,
   and runs an additional forced-`LegacySync` leg (debug/test-only backend
   override) so both sync backends stay covered locally.
3. **Real homeserver QA** — required before GUI-level confidence claims:
   HTTPS login, recovery, encrypted store restore, sync lifecycle, room list,
   timeline, send, search smoke, logout, account switch.
4. **Headless UI tests** — the frontend runs in a plain headless browser
   (Vite dev server + mocked Tauri IPC) against fake `CoreEvent`/snapshot
   streams. This layer owns React UI behavior: timeline diff application,
   generation handling, scroll anchoring and DOM scrollback behavior, command
   invocation shapes, right-panel/settings/search interactions, and responsive
   layout states. It runs without any native window or OS keychain access.
   The current canonical harness is Playwright headless Chromium via
   `npm --prefix apps/desktop run test:ui-headless`; WebdriverIO/Tauri
   browser mode is allowed only after a package spike proves it keeps the
   same no-native-app property.
5. **GUI smoke** — a deliberately minimal, last layer for what only the
   real Tauri app can prove: native window behavior, real IPC, webview
   integration. Subject to the automation rules in the policies document.
   Agents drive GUI design and testing as far as possible without a visible
   window: headless browser first (layer 4), and — once a Linux lane
   exists — the real Tauri app under a virtual display (Xvfb +
   `tauri-driver`, which supports Linux/Windows but not macOS), unattended.
   macOS-specific behavior (WKWebView, OS menu accelerators, Keychain
   prompts) stays a minimal attended smoke coordinated with the user —
   never unattended agent verification. If the virtual-display lane proves
   valuable, moving primary GUI development/testing to Linux is an accepted
   option.

**Implementation workflow: headless-first, local-server-first.** New Matrix
behavior lands in `matrix-desktop-core`, is exercised through
`CoreCommand`/`CoreEvent` against disposable local Conduit/Tuwunel homeservers
(and real homeserver QA where that gate applies), and only then is wired through
Tauri into React. Matrix behavior must not be introduced first in GUI or Tauri
code and back-filled into core later.

QA waits on events, never on fixed sleeps. QA asserts on `CoreEvent` and
`AppStateSnapshot`, never on logs. Diagnostics are structured, redacted, and
not a source of truth.

## Phase 10+ Product Surface Roadmap

The headless core runtime is complete through Phase 9 cleanup. Product UI work
continues in
`docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md`
and keeps the same QA hierarchy.

- **Phase 10:** harden the headless browser harness and IPC mock so the real
  app shell can be mounted under fake `CoreEvent` and snapshot streams.
- **Phase 11:** complete the thread model core-to-UI path headless-first.
- **Phase 12:** build the three-pane product surface, right panel, settings,
  search, shortcut, and responsive UI behaviors in React, verified headless.
- **Phase 13:** complete remaining transport integration hardening on Linux as
  the primary agent environment; still no native GUI launch.
- **Phase 14:** build the Linux virtual-display real-Tauri lane for native
  window, IPC, menu, and WebKitGTK behavior under Xvfb + `tauri-driver`.
  macOS-specific WKWebView/menu/Keychain checks remain attended only.
- **Phase 15+:** finish desktop interaction completeness, E2EE trust
  implementation and GUI, performance/soak, distribution hardening,
  platform credential-store evidence, signing/notarization, and release.
