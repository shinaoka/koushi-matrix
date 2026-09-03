# Matrix Desktop Architecture Overview

Status: normative. This is the long-term blueprint for the whole application.
Dated specs and plans under `docs/superpowers/` are implementation guides
toward this document and must not contradict it. Amend this document first
when a design change is needed, then update or supersede the affected specs.

Last amended: 2026-09-05.

The evidence-based classification of remaining frontend-owned resources and
semantic migration candidates is maintained in
[frontend-ownership-inventory.md](frontend-ownership-inventory.md). It is an
inventory, not authority over the normative layer rules in this document.

## Product Scope

A Windows/macOS desktop Matrix client following Element X's Rust SDK direction
with an Element Desktop/Web-like three-pane desktop UX:

- Shell: Tauri v2. Frontend: React + TypeScript. Backend: Rust on
  `matrix-sdk` / `matrix-sdk-ui`.
- First version: E2EE text chat, Spaces, room timelines, threads, desktop
  interaction, encrypted ngram full-text search (CJK-capable).
- Out of scope for MVP: voice/video calls, screen sharing, bots, widgets,
  app integrations.
- DMs are account-level conversations (Element X Android-style two-member
  classification). They are shown in full only in Home (no active Space); when a
  Space is active, the DM section shows DMs where at least one counterpart is a
  member of that Space's room (any counterpart for group DMs). A DM matching no
  Space appears only in Home. DMs are never assigned to Spaces via
  `m.space.child`/`m.space.parent`; the association is by counterpart space-room
  membership, computed Rust-side as `RoomSummary.dm_space_ids`. Direct Space
  membership completeness is explicit Rust-owned projection input. A partial
  observation may add known-positive associations but must preserve prior
  positives; only a complete direct-member observation may remove one.
- Threads are linear. The thread pane's composer sends ordinary thread messages,
  and thread events expose no reply-composition affordance: no per-event Reply,
  no Reply in thread, no nested thread. Rich replies authored by Element or
  another Matrix client still render their quoted context inside a thread, and
  core keeps a thread-keyed reply an ordinary thread message
  (`EnforceThread::Threaded(ReplyWithinThread::No)`), so creating a thread rich
  reply stays unreachable from the product UI. Reply and Reply in thread remain
  room-timeline actions.
- Opening a thread carries a Rust-owned creation intent. A root with a positive
  projected reply count and every Threads-list entry is an existing thread; a
  room-timeline root with no known replies is a new-thread draft. Draft panes
  subscribe for live activity and expose the composer immediately, but they are
  ineligible for automatic backward history until an accepted local thread send
  or matching event-backed thread activity promotes them. React renders this
  state and must not infer thread existence from an empty viewport.
- A browser-hosted build (Element Web-like deployment of the same core) is a
  potential future target. It is not scheduled, but the architecture must not
  preclude it; see Platform Portability.

## Layers

```text
React UI (apps/desktop)                     presentation only
        |  typed client calls / snapshots / events
Tauri adapter (apps/desktop/src-tauri)      transport + platform delivery
        |  constructs/serializes public DTOs
koushi-protocol                     neutral command/event/state-update DTOs
        |  typed runtime boundary
koushi-core                         the ONLY production runtime owner
        |  actors own SDK handles, tasks, policy, projection
koushi-sdk                         thin matrix-rust-sdk adapter
koushi-store                       credential + encrypted-file persistence
koushi-state                        pure reducer + snapshot DTOs
koushi-search / koushi-media / koushi-key   pure algorithm/key leaves
        |
matrix-rust-sdk (vendored)                  sync, timeline, send queue, crypto

koushi-qa (non-default) consumes koushi-protocol + Core test hooks for the
headless and real-homeserver QA binaries; it is not in the production stack.
koushi-core-testkit is likewise non-default and test-only: it owns shared Core
integration fixtures/targets and never participates in the product runtime.
```

Crate responsibilities:

- `koushi-state` — pure. `AppState`, `AppAction`, `reduce()`,
  serializable snapshot DTOs. No SDK handles, no Tauri, no async.
  E2EE trust, verification, cross-signing, key-backup, and identity-reset UI
  state is modeled here as guarded, request-correlated state. GUI code renders
  that state; it does not own trust decisions. Personal local user aliases are
  also Rust-owned profile state: `ProfileState.local_aliases` stores the
  account-data-backed map, reducer actions own set/clear/list lifecycle, and
  display-name resolution follows the surface-specific Rust policy before
  React sees labels. Normal people-facing labels remain optional and do not
  promote an MXID when no friendly label exists. `ProfileState.users` is an
  account-scoped cache, not room-membership evidence.
  `AppState.mention_candidates` is the room/surface-keyed, joined-member-only
  autocomplete projection. Rust owns membership eligibility, query
  normalization, Unicode/CJK matching, deterministic ranking, completeness,
  `@room` permission, and stale-result fencing; React owns only popup
  visibility/focus and typed mention intent. Timeline relabeling after profile
  or alias changes
  is also a Rust-owned `CoreEvent::Timeline` patch stream; React may match rows
  by raw identity fields and apply Rust-provided labels, but it must not
  recompute alias precedence. React may render the DTO and dispatch typed alias
  commands only; it must not maintain a separate alias cache or write aliases to
  Matrix profile/events.
- `SettingsState` is serializable Rust product state owned by
  `koushi-state` and persisted by `koushi-core` through a
  non-secret settings store. React may apply settings to presentation, but it
  must not be the source of truth for locale, theme, font/emoji choice, or
  composer send shortcut semantics. Locale/display profile resolution is also
  Rust-owned; GUI code consumes the resolved profile and catalog selector
  defined in `docs/architecture/i18n.md` rather than parsing raw language tags.
  Font/emoji display profile resolution is likewise Rust-owned:
  `koushi-state` resolves `TypographyDisplayProfile` from
  `SettingsValues.typography` and the platform profile, and the frontend may
  only apply the resulting font, emoji, and asset-status tokens to root
  attributes/CSS. Inter and Twemoji COLR are bundled-preferred choices with
  system fallbacks; React must not choose fallback semantics per component.
  Display preferences such as code-block line wrapping and desktop density live
  under `SettingsValues`; sidebar category, section collapse and room-list sort
  are likewise Rust-owned settings, and recent emoji is a bounded canonical MRU
  projected from `SettingsValues.composer`. React may map these snapshot values
  to presentation CSS/visibility and dispatch typed updates, but it must not keep
  an independent display, sidebar or recent-emoji product store.
  Home subsection/DM memory and per-Space local name/icon presentation contain
  Matrix identifiers or free-form account data, so they live in the existing
  per-account encrypted navigation store rather than the non-secret settings
  JSON. Rust projects the final Space label/icon and complete sidebar section
  membership/order; React never joins, classifies or sorts those product facts.
  Composer key handling uses the pure Rust-owned resolver in
  `koushi-state`; GUI code supplies typed key facts and
  renders/dispatches the resolved action. Because the resolver may cross an
  async transport boundary, GUI code captures key facts and editor selection
  synchronously, prevents default only for resolver-owned keys, and applies
  newline/send/cancel only from the returned action. Resolver failures are
  no-ops; React must not fall back to local send semantics. Composition key
  events keep the native browser default so IME candidate commits are not
  blocked by the async resolver boundary; Rust still owns the returned product
  action (`CommitImeCandidate`). Composer send payload semantics are also owned
  by Rust/core: the versioned `ComposerDocument` carries text and identity-bearing
  atomic mention nodes, from which Core derives readable plain text, safe
  `matrix.to` mention anchors, and Matrix `m.mentions`; markdown/html and `/me`
  emote conversion are built before SDK send, and
  recognized-but-unavailable slash commands (/join, /invite) fail locally with
  structured private-data-free failure kinds surfaced near the composer; unknown
  leading-slash text is ordinary content and sends literally (#450). React does
  not construct `m.mentions`, formatted bodies, or slash-command dispatch.
  The same native-composition boundary applies to every desktop text-entry
  surface, including upload captions, search, room/profile/settings fields,
  authentication, recovery, and dialogs. These surfaces share one React
  primitive layer: the DOM owns composition and unacknowledged drafts, logical
  keys define explicit resets, candidate-confirmation Enter fences associated
  form submission, and per-field latest-wins queues serialize writes, coalesce
  superseded pending work, and reject stale async snapshots.
  Password and recovery strings remain uncontrolled DOM values and do not enter
  React state. A repository AST gate prevents feature components from bypassing
  this layer with raw composable controls or forms.
  Room management is likewise Rust-owned: room settings snapshots, room-scoped
  member summaries, permission facts, setting changes, power-level role edits,
  and kick/ban/unban moderation operations live in
  `AppState.room_management` and `RoomCommand` / `RoomEvent`. React renders
  `settings.permissions`, `settings.avatar_url`, and `settings.members`
  including Rust-projected member display labels, role facts, and power facts,
  and dispatches typed commands only; it must not decide whether a user can edit
  settings, edit roles, or moderate members locally.
  Room list/title labels are also Rust projections: `RoomSummary.display_label`
  is the normal display value for room headers, sidebar entries, forward/search
  metadata, space child rows, and native attention labels. `display_name` stays
  the upstream/original room name, and one-to-one DM identity is supplied by
  `dm_user_ids` rather than inferred from text in React.
  Core Batch A0 ownership also lives in this crate: local encryption /
  credential-store health, native attention candidates and capabilities,
  Japanese/CJK display/search policy, and backup restore scope are
  serializable Rust state or DTO contracts. React renders those contracts and
  dispatches typed commands; it does not decide credential health, notification
  eligibility, CJK collation/normalization, IME send-vs-commit behavior, or
  whether key-backup restore is complete.
- `koushi-protocol` — transport-neutral public Rust commands, events,
  identities, failures, command-admission/versioned-snapshot and state-delta
  DTOs. It depends only on pure app-owned crates and serde support: no Matrix
  SDK, Tauri, async runtime, filesystem/platform, or OS dependency. Data-shape
  helpers and redacted `Debug` live here; actor routing/admission policy,
  AppState-dependent projection, state-delta construction and SDK behavior do
  not. Secret-bearing command aggregates are typed Rust inputs constructed by
  validated adapters rather than wholesale serde payloads.
- `koushi-sdk` — low-level SDK adapter (login, restore, recovery,
  sync, room, timeline, search primitives). It may include feature-gated,
  direct-adapter smoke binaries and private-data-free smoke reports for
  adapter integration. No app state, actor lifecycle, or authoritative app QA
  orchestration; those remain in `koushi-core` and `koushi-state`.
  Password and OIDC/MAS sessions both stay SDK-owned here: OAuth dynamic client
  registration, PKCE authorization-code construction, callback completion,
  refresh-token handling, token revocation on logout, and tagged session
  persistence are exposed to `koushi-core` only as app-owned DTOs and
  persistable secret blobs. Authorization URLs may be returned for the WebView
  browser handoff, but access tokens, refresh tokens, PKCE verifiers, raw OAuth
  errors, and provider callback details never enter reducer state or normal
  diagnostics.
  E2EE key-backup restore wrappers consume recovery secrets internally and
  return private-data-free restore summaries whose scope is explicitly
  `JoinedRooms`; they do not expose SDK backup keys, room keys, or raw backup
  versions across the command/event boundary. The MVP restore scope is
  recovery secret import plus currently joined-room key hydration through
  public SDK APIs. Product state, QA evidence, and UI copy must not claim
  exhaustive backup-wide restore until a public SDK API or reviewed vendored
  patch proves that broader scope.
- `koushi-core` — actor lifecycle, command routing/admission policy, event
  emission/projection, SDK session handles, background tasks and AppState
  projection. It consumes `koushi-protocol` DTOs and contains no QA binary
  source tree. Its `StoreActor` remains the sole account/path/key/migration
  policy owner while delegating credential and encrypted-envelope mechanics to
  `koushi-store`. It retains the task/subscription handles it creates and makes
  replacement plus ordered shutdown a cancel-and-await barrier; a detached
  task is never a lifecycle owner. Production Matrix behavior lives here and
  nowhere else. Scheduled
  send uses this layer for MSC4140 capability detection and SDK/Ruma delayed
  event requests, and for the local fallback timer that routes due Local-handle
  items back through the normal outbound send queue; the GUI never owns
  delayed-send timers or Matrix delayed-event API calls.
- `koushi-store` — native persistence leaf for credential backend selection,
  the encrypted credential-vault file, and the shared encrypted-file envelope.
  It depends on app-owned identity/key/state types and diagnostics but has no
  Matrix SDK, Tauri, async runtime, Core, QA or OS-keyring implementation.
  `StoreActor` remains in Core as the sole policy/lifecycle owner: it selects
  account paths, obtains and derives secrets, owns migrations and generation
  fences, maps coarse failures, and supplies SDK store/search configuration.
- `koushi-key` — platform-neutral credential-store port, key derivation (HKDF
  from the local unlock secret), and zeroizing secret wrappers. The OS keyring
  backend lives in Tauri.
- `koushi-search` — candidate verification, document store, index
  maintenance queue.
- `koushi-media` — pure image decode-limit, resize/format, encoding and byte-kind
  classification helpers. Core owns media/cache lifecycle and state projection;
  adapters own platform delivery.
- `koushi-core-testkit` — non-default, publish-disabled test package for shared
  Core integration fixtures and targets. It may enable Core `test-hooks`; it is
  not a production dependency, QA runtime, or reason to expose private actors.
- `koushi-qa` — non-default, feature-gated package owning the authoritative
  `headless-core-qa` and `real-homeserver-qa` binaries, scenario registry,
  orchestration and private-data-free evidence production. It uses protocol
  DTOs and narrow Core test hooks without owning product semantics.
- `apps/desktop/src-tauri` — transport/platform adapter. Holds a `CoreRuntime`,
  constructs protocol commands, forwards events/snapshots, registers native
  artifact paths and maps opaque thumbnail references to the desktop custom
  URI. No direct SDK wrapper calls.
- `apps/desktop` — view and interaction code only, including viewport state,
  DOM measurement, and scroll anchoring. It may own browser listeners,
  observers, frames, and timers only for mounted presentation lifetime, with
  cleanup in the same effect/controller; product retry and session lifecycle
  remain Rust-owned.

Upstream SDK deltas are consumed exclusively from the checked-out
`vendor/matrix-rust-sdk` submodule. Root workspace Matrix SDK dependencies are
exact paths beneath that checkout, and the parent repository gitlink is the
single revision pin; builds must not substitute a remote git dependency for the
code under review. The submodule tracks the upstreamable fork on
`github.com/shinaoka/matrix-rust-sdk-work`. Local comments document patch
surfaces, and `docs/upstream/matrix-rust-sdk-feedback.md` is the required ledger
for every local SDK behavior or API divergence.

GUI, Tauri, CLI, and QA all use the same `koushi-protocol` command/event
boundary. There is no standalone daemon; the runtime is in-process. The
boundary is frontend-neutral: public Rust integration evidence starts
`CoreRuntime`, attaches independent consumers, submits a connection-scoped
typed command, observes `CoreEvent` plus versioned-snapshot convergence,
recovers a lagged consumer from the latest snapshot, and awaits ordered
shutdown without importing Tauri. Serialized `FrontendDesktopSnapshot` mirrors
remain adapter-only in `apps/desktop/src-tauri`.

### Core request outcomes (Phase A, issue #755)

`koushi-core` owns request settlement through the closed, non-serde
`runtime::request_outcome` service. `CoreConnection::wait_for_request_outcome`
uses the connection's event stream as a wake source and its versioned watch
snapshot as authority: it checks the initial snapshot, applies the expectation's
exact request/account/target/submission guards, uses one absolute deadline, and
performs one final snapshot check after timeout, closure, or lag. A waiter may
use a separate attached event connection; a `RequestId` is globally unique on
the shared stream and is never compared with the waiting connection ID.
Submission correlations require both the originating request and
`SubmissionId`. Lag is either recoverable or terminal according to the closed
expectation variant, and `Lagged`, `Disconnected`, `TimedOut`, operation
failure, and typed no-op outcomes remain distinct. `select_room_and_wait` is a
compatibility wrapper over this service.

This is Phase A runtime infrastructure only: it adds no `AppState`,
`AppAction`, reducer, or reducer transition. Tauri waiters and their product
loops are intentionally not migrated until the later phases of issue #755.

### Core-owned staged upload orchestration (Phase B, issue #755)

`koushi_core::media_staging::MediaStagingService` owns the production staged-upload
orchestration API. It validates a non-empty batch, the named
`MAX_MEDIA_STAGING_BATCH_SIZE` and `MAX_MEDIA_STAGING_BATCH_BYTES` limits,
normalizes MIME values, and derives the initial media kind before publishing
`AppCommand::SetUploadStaging` with `StagedUploadPreparation::Preparing`.
Preparation and output encoding run through `crate::executor::spawn_blocking`
without either media lock held. The service captures the account, active
`ComposerTarget`, staged-id set, and selection generation, then revalidates all
of them before merging a detached `MediaPreparationRegistry`; stale or failed
work that has not been merged drops its bytes. Caption and compression metadata
are copied from the current state when a prepared item is replaced.

The service reuses the existing `MediaPreparationService`/registry and
`AppCommand` reducers; it adds no reducer action or transition. Its target
mutation methods use the same Core request-outcome service and exact versioned
snapshot generation. Transition guards acquire the transition owner before the
registry and never hold either guard across an await or preparation work.
The service serializes operations per `ComposerTarget` without holding the
global preparation registry during encoding. It also owns prepared-preview byte
lookup and prepared-send admission/correlation. Tauri handlers only deserialize
inputs, convert opaque composer tokens, call these Core methods, and serialize
the settled snapshot or preview bytes; they contain no batch, MIME, preparation,
selection-generation, registry-merge, replacement, or send policy.

### Core-owned composer transport identities (Phase D, issue #755)

`ComposerDraftLeaseRegistry` is the sole authority for renderer generations,
lease ids, exact account/target scopes, and command/persistence permits. The two
opaque identities expose canonical nonzero decimal `u64` wire conversion;
parsing validates shape only and grants no authority. Core rechecks the current
Ready `SessionKeyId` and exact active main/thread `ComposerTarget` before lease
or terminal-permit admission. Tauri stores no identity counter or lookup map and
only parses/formats IPC strings. A fresh process may restart numeric values;
safety comes from beginning that runtime's live generation and passing its
registry/scope checks, not cross-process numeric uniqueness.

This ownership move adds no `AppState`, `AppAction`, or reducer transition.

### Rust-owned secure-backup confirmation admission (Phase E, issue #755)

`SecureBackupSetupIntent` carries `InitialSetup` or `Reenable { confirmed }`
through both the projected reducer action and `SecureBackupSetupRequest`.
`AppActor` runs gate × intent admission before routing to `AccountActor`, so
unconfirmed, stale, forged, duplicate, or gate-incompatible intents produce a
typed private-safe failure and no SDK effect. Only confirmed re-enable at the
explicitly-disabled gate maps to the SDK re-enable path; initial setup never
claims confirmation. The SDK's fresh server/local/trust inspection remains the
authoritative guard and Core never overrides its confirmation-required result.
React renders the accessible catalog-backed dialog, cancel emits no command,
and Tauri only transports the typed intent; native hardcoded policy copy is
absent.

## Platform Portability

The desktop app is the only shipping target today, but a browser-hosted build
of the same core (Element Web-like) is a plausible future. matrix-rust-sdk
already supports `wasm32` (executor abstraction over tokio /
`wasm_bindgen_futures`, IndexedDB store besides SQLite), so portability is
decided by our own code discipline, not by the SDK. These rules keep the
option open at near-zero ongoing cost; retrofitting them later would mean
rewriting the runtime.

1. **The command/event boundary is transport-neutral.** `koushi-protocol`
   owns `CoreCommand`, `CoreEvent`, identities, failures and state-update DTOs;
   they contain no Tauri, OS, filesystem, SDK or async-runtime types. Events,
   identities, failures and snapshots preserve safe serde contracts. Commands
   that carry secrets remain typed Rust inputs: each adapter validates its IPC
   payload and constructs them without requiring the secret-bearing aggregate
   itself to implement serde. Tauri IPC is one transport; a WebWorker
   `postMessage` / wasm-bindgen bridge must be addable without changing the
   protocol shapes.
2. **Core logic uses executor abstractions, not tokio directly.** Task spawn,
   timers, and timeouts in `koushi-core` go through the SDK's
   executor layer (`matrix_sdk_common::executor`) or a thin core-owned
   wrapper. No `tokio::spawn`/`tokio::time` calls scattered through actor
   logic; no thread-blocking (`block_on`, blocking locks held across await)
   inside actors. The actor runtime must be able to run on a single-threaded
   executor (wasm) as well as multi-threaded tokio.
3. **Platform capabilities live behind ports, owned by `StoreActor` and the
   adapters.** OS credential store (`keyring`), filesystem paths, SQLite
   store config, media-save filesystem operations, and process/OS APIs appear
   only behind traits with platform backends (today: OS keychain + SQLite and
   the native media-save port; browser later: WebCrypto-derived keys +
   IndexedDB). `koushi-store` may implement native credential/encrypted-file
   mechanics behind those ports, but `StoreActor` is the only actor allowed
   platform-conditional policy and remains the account/path/key/migration
   owner. The fail-closed local-encryption rule still applies on every
   platform: a weaker browser at-rest story must be an explicit, surfaced
   property, never a silent fallback.
4. **Pure crates stay wasm-clean.** `koushi-state`, `koushi-search`, and
   `koushi-protocol` compile for `wasm32-unknown-unknown`, enforced by CI.
   `koushi-core`'s
   portability is enforced structurally by rules 1–3 until a web spike makes
   a wasm CI check for it practical.
5. **Known open items for a web target** (recorded, not designed): ngram
   search index backend on wasm, credential storage UX without an OS
   keychain, and multi-tab/single-runtime coordination. None of these may be
   solved by weakening the desktop security model.

## Runtime Model

An in-process actor system in `koushi-core`:

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
  (`m.tag` favourite / low priority). It also owns the demanded room member
  directory for mention autocomplete: cached `JOIN` members produce a
  fail-closed partial projection, incomplete lazy-loaded membership triggers
  one SDK refresh, and base-room membership updates invalidate and recompute
  every demanded main/thread target for that room.
  On the single Element X-compatible Simplified Sliding Sync engine it
  consumes the one `RoomListService` owned by the running `SyncService`;
  constructing additional ad-hoc `RoomListService` instances is prohibited —
  they are not driven by the sync loop, race it, and return entries without the
  `required_state` (e.g. `m.room.create` for space classification) the live
  service requests. Its live entries adapter uses a non-left filter, but a bounded
  adapter diff is not the sole liveness signal: an invite can commit outside
  the visible entries head without changing that head. The same observer
  therefore consumes the base client's already-committed room-update broadcast
  as an auxiliary wake source. It coalesces queued updates, reprojects on an
  invite payload or invite-membership change, performs one reconciliation after
  lag, and ignores ordinary joined-room updates whose invite fingerprint is
  unchanged. Closing the auxiliary broadcast disables only that wake arm; it
  never starts a second sync loop or `RoomListService`. Room tags are projected into
  `RoomSummary.tags` by the same Rust-owned room-list normalization path, and
  sidebar unread/mention affordances consume Rust-owned unread/highlight counts
  from `SidebarModel`. React must not derive favourite, low-priority, unread,
  or mention membership from local UI state. Selecting a Space demands direct
  Space-member hydration through the existing SDK member API and the same live
  room-list observer; the members panel consumes that source and is never the
  hydration trigger. Partial direct-member snapshots preserve last-known
  positive `dm_space_ids`, while a complete snapshot may remove stale
  associations. The sidebar and Rust `People` projection use the same scope
  predicate. Room-list bootstrap readiness is separate from `SyncState::Running`:
  the actor retains the last usable
  snapshot while the current engine is unproven/loading, holds an unproven
  empty SyncService Reset, accepts an authoritative zero only after the current
  engine generation proves connectivity, and ignores delayed projections from
  retired observers. First-response proof and engine replacement use the same
  generation-fenced contract.
- `TimelineManager` (per account session) — timeline actor routing, the
  session-resident Sliding Sync room-subscription set, and the session-scoped
  outbound-send lifecycle. It is the only Koushi caller that mutates the live
  `RoomListService` room-subscription set. Opened timeline rooms, successfully
  projected non-left room-list entries, and subscriptions restored with a valid
  Sliding Sync position are unioned into one uncapped in-memory set. Actor
  unsubscribe/rebuild, thread/focused navigation, cache eviction, and projection
  replay never remove residency; only successful room leave removes one room,
  while logout/account switch/reset/delete drops the account-session owner. A
  manager-instance typed handle is atomically bound to its exact SDK session;
  room membership operations require pointer identity with RoomActor's current
  session, then snapshot an admitted permit before their SDK await, receive a
  manager acknowledgement, settle existing reducer/events under that permit,
  and drain before that
  manager is retired. Session-local leave state blocks restore/visibility
  resurrection until an admitted successful local rejoin or ordered SDK `left`
  then `joined|invited` observation; observer coalescing preserves that order.
  Visible-range intents are fenced by the current sync generation, and an
  identical desired set is an SDK no-op. The set is not persisted separately and
  never overrides the SDK's members-missing reload or Megolm rotation after real
  coverage loss/UnknownPos. It directly polls supervised SDK
  enqueue futures, the sole client-global SDK send-queue terminal observer, and the
  composite `(room_id, sdk_transaction_id)` correlation back to the original
  `TimelineKey`, `RequestId`, and `SubmissionId`. Its lifetime spans
  room/thread unsubscribe and actor replacement. Enqueue futures obey the async
  contract that each poll returns; panic isolation converts an unwind into the
  registration's private-safe fail-closed terminal. Before an accepted route
  returns, the manager drives that specific worker past its admission permit to
  a one-shot signal marking the start of payload-specific preflight; completing
  an unrelated ready worker is not sufficient. Reply preparation may still
  suspend before the eventual SDK queue call, so this signal does not serialize
  SDK enqueue order across workers. FIFO retry after queue insertion remains
  SDK-owned. Orderly and unexpected
  teardown synchronously drops any remaining manager-owned futures while the
  observer is still owned, so no raw task handle can detach an accepted send.
- `TimelineActor` (per room/thread/focused timeline) — subscription, diffs,
  pagination, edit/redaction relay, reaction annotation projection and
  guarded send/redact relay, media/file projection, upload progress,
  room-scoped live signals, send-state presentation, and Rust-only media
  download effects. It provides a cloneable timeline/send context to the
  manager and projects local echoes, but it neither owns the accepted SDK
  enqueue future nor terminal send observation or command correlation.
  Room live timelines use
  `TimelineFocus::Live { hide_threaded_events: true }` so threaded replies
  are hidden from the main room timeline. Expanded threads use
  `TimelineKind::Thread`. On the sliding-sync backend, timeline admission
  submits its room ID to the manager-owned session residency coordinator, which
  reconciles the one live `RoomListService` through the standard Element
  X-style room-subscription API; individual `TimelineActor` lifetimes do not own
  room subscription lifetime. The all-rooms list alone only guarantees the
  initial window on some servers. Thread backward pagination uses the same
  `TimelineKind::Thread {
  room_id, root_event_id }` key as the thread subscription. Plain sends,
  replies, media, edits, and redactions go through the SDK `Timeline` handle
  (not
  direct room/send-queue calls) so their diffs are produced as local echoes
  instead of depending on the server echoing them back; for own sent events whose
  remote echo has not arrived, the actor resolves the event id back to the
  local-echo transaction identity. Media messages are projected into
  `TimelineItem.media` from SDK message content. Upload staging and room media
  gallery state are also Rust-owned: reducer backing stores are projected into
  `TimelinePaneState.staged_uploads` and `TimelinePaneState.media_gallery` for
  the selected room. React renders those DTOs and dispatches typed commands
  only; it does not infer Matrix media semantics, keep a parallel upload-staging
  store, synthesize gallery membership, own upload state, encrypted media
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
- Room-scoped mention demand enters `RoomActor` through
  `RoomCommand::QueryMentionCandidates`. The actor publishes reducer actions,
  `AppState.mention_candidates` flows through the versioned state-delta
  transport and WebView projection store, and selectors consume only the exact
  `(room_id, surface)` entry. Late refresh results are fenced by account, room,
  surface, request, query, and generation before publication.
- Account-wide Activity is projected in `AppActor` from Rust-owned timeline
  observations plus room unread/tag summaries. `TimelineView` and focused
  timelines remain event-driven render surfaces; they do not own the Activity
  state machine. React dispatches typed Activity commands and focused-context
  opens using event references supplied by Rust.
- `ThreadsListActor` (per account session) — scoped thread-list subscriptions.
  A room scope uses one SDK `ThreadListService`; Home and Space scopes resolve
  their Rust-owned room sets first and use one bounded service per room. The
  actor merges rows, deduplicates by `(room_id, root_event_id)`, preserves the
  owning room on every row, and owns pagination completion/failure. React
  renders this projection and never aggregates room or timeline data itself.
- Account-level live signals such as presence are Rust-owned state in
  `AppState.live_signals.presence`. In the current Phase A contract,
  `AccountCommand::SetPresence` records the requested presence and emits typed
  `LiveSignalsEvent` updates. Network presence propagation is sync-backend
  policy: the legacy SDK path uses `SyncSettings::set_presence`, while the
  current `SyncService` builder in the vendored SDK has no direct presence
  setter. Do not move presence semantics into React while that SDK/API decision
  remains open.
- `SearchActor` — ngram candidates, canonical-text verification,
  document-level index mutations for edits/redactions/late decryptions, and
  Element-style background history crawling. Historical `/messages` requests
  are account-wide backpressured: `TimelineActor` pagination and
  `SearchActor` crawler pages share one gate per account, and user-visible
  timeline pagination has priority over background crawler work.
- `StoreActor` — credential store access, store/search keys, per-account
  paths, cleanup, debug/test secret injection policy.

**Account store bootstrap invariant.** Authentication never runs on a memory-
store client. Before password, OAuth, SSO, restore, or soft-logout reauth can
activate E2EE, `StoreActor` selects an encrypted persistent SDK/search store by
an opaque random local store ID and builds the authentication client with it.
Fresh authentication journals that store, its unlock secret, and a generated
Matrix device ID before network authorization; `PreAuth` and bound-tokenless
journal states remain resumable until verified promotion atomically persists
tokens. The exact authenticated client is promoted directly through capability
and verification admission—Koushi never authenticates a disposable client or
transplants its session into another client.

Saved-device login, restore, and reauth require the existing crypto DB before
SDK builder use, open it with the saved key, and load the expected user/device
Olm account. Online saved-device authentication and reauth compare that account
to a fresh server device-key query before provisional installation. Offline
restore compares the persisted local device view so startup remains available;
the next encryption sync refreshes device keys and the authoritative trust gate
quarantines any mismatch before normal encrypted use. Missing, corrupt,
wrong-key, wrong-account, mismatched, or unknown state fails closed and never
recreates crypto under the saved device ID. Legacy identity-slug store roots migrate once, unopened, into
the opaque-ID layout through a durable, resumable same-volume rename. Soft-
logout may retain a client-free actor-owned Locked record after the invalid
client is dropped; only reauth, logout, and local-reset commands are admitted
until a matching store-backed client is authenticated. `SwitchAccount` remains
ordered shutdown followed by the same non-creating store-backed restore.

**Keyed SDK media-store invariant.** The account-keyed `SqliteStoreConfig` is
also the configuration used by the SDK `SqliteMediaStore`; supplying a separate
`cache_path` does not remove or replace its `MatrixClientStoreKey`. Automatic
avatar persistence therefore uses the SDK media store and its retention policy,
not a Koushi-owned disk cache. Koushi may materialize decrypted bytes into a
session-scoped in-memory renderable-thumbnail cache. Core/state/protocol expose
only an opaque cache reference; the desktop Tauri adapter may map that reference
to `koushi-thumbnail://`, while a native adapter may consume bytes directly.
The cache is an entry-and-byte-bounded LRU: access refreshes recency, eviction
or session clear releases the owned bytes, and an item larger than the byte
bound fails before a Ready reference is published. It must never persist
automatic avatar/link-preview plaintext or return `file://` URLs for them. Legacy
plaintext thumbnail directories remain cleanup-only. After renderer visibility
submits an avatar MXC, `AccountActor` owns single-flight deduplication, bounded
concurrency, two network attempts, terminal Ready/Failed caching and session-
generation teardown; React owns no retry classifier or attempt counter.

**Prepared-upload retention invariant.** `MediaPreparationService` owns staged
upload source bytes for the lifetime of the corresponding composer item.
Untouched originals are source-backed cached variants rather than duplicate
byte vectors; only transformed outputs own additional bytes. Item removal,
target/thread clear, snapshot reconciliation, account change, and full clear
must release both sources and dependent variants. Exported retention summaries
contain counts, byte totals, high-water marks, and fixed tokens only.

**Verified-session admission invariant.** Password/OIDC authentication and
credential restore first install an AccountActor-owned provisional SDK session;
they do not publish `Ready` or an active saved session. The actor subscribes to
the SDK current-device verification stream before reading its current value and
promotes only an authoritative `Verified` observation. While provisional, only
a restricted crypto synchronization loop may process the account data,
device-list, key-query, recovery, and to-device traffic required for trust
discovery and SAS when the initial authoritative value is not Verified. An
initial Verified value skips restricted sync. A later Verified value cancels
and joins the restricted loop before the Ready projection is dispatched; the
normal SyncActor starts only after that projection is acknowledged. These are
exclusive classic-sync owners and do not share or overlap sync-token lanes.
Admission never performs an unconditional full-state catch-up: trust decides
Ready, while normal sync health and offline/reconnect behavior belong to the
Ready shell. The provisional runtime must not start normal
Sync/Room/Timeline/Search actors,
publish room or attention projections, restore drafts/navigation/scheduled
sends, or authorize ordinary commands. Initial provisional credentials are not persisted; a process restart returns
to `SignedOut`. Rejection performs best-effort server logout and deletes
account-local keyed stores. A later authoritative `Unverified` observation on a
Ready session atomically re-enters the actionable verification gate, stops
normal children, clears session views, and then starts the restricted crypto
lane; the already-persisted Ready session is retained. A later authoritative
`Unknown` observation performs the same fail-closed quarantine but remains in a
retryable checking state and starts neither verification-method discovery nor
destructive cleanup. `Locked` is reserved for authentication/session
invalidation. Current-session diagnostics use this same three-state SDK signal
as their sole verification verdict; cross-signing, own-identity, sync, and
backup facts remain supplemental.

**Provisional-device cleanup invariant.** A failed or unavailable verification
method never deletes a device or local data automatically. The verification
gate may offer an explicit destructive cleanup, but its ordering and outcome
are Rust-owned: `AccountActor` first resolves the active SDK session's
authentication mode and authoritative current Device ID, then removes the
legacy Matrix device through UIAA or revokes the OAuth/MAS session through the
SDK OAuth logout path. Only remote success or an authoritative already-absent
result permits account-local persistence clearing and `SignedOut`. A remote
failure retains the session and keyed persistence for retry. A separately
confirmed local-only escape is allowed only from that failed state and must say
that the remote device may remain. Raw Device IDs, UIAA sessions, tokens,
passwords, and SDK errors stay inside Rust. Remote account/device management is
not a Koushi surface: after promotion, an AccountActor-owned task resolves one
optional destination for the exact active session. OAuth uses public SDK server
metadata plus its devices-list account action; non-OAuth sessions use the active
homeserver's well-known client metadata. Only HTTP(S) destinations cross into
AppState, and replacement, authentication lock, trust quarantine, logout, or
switch invalidates them.
Login discovery never owns this active-session capability. Ambiguous legacy
`M_UNKNOWN_TOKEN` and generic `M_NOT_FOUND` errors are not
proof of target-device absence and therefore remain retryable. Starting a new
verification/recovery attempt retires the cleanup offer; the two flows never
own the provisional session concurrently.

Actor deployment is flexible. The boundaries above define state ownership,
command routing, event production, and shutdown responsibility; they do not
require one Tokio task per actor in the first implementation. The runtime may
colocate child loops under `AccountActor` while preserving the same public
contracts and resource ownership.

Supervision follows the same ownership tree:

- `AppActor` owns account runtimes; each `AccountActor` owns its child task
  handles and subscription handles. Every child actor in turn retains the
  `JoinHandle`s and live subscription handles it creates. Replacement and
  shutdown cancel and await those children before the owner reports completion;
  `JoinHandle` drop is an unexpected-detach hazard, not an orderly teardown.
- Expected SDK failures are reported through domain state (`SyncFailed`,
  pagination failure, search failure) and redacted `OperationFailed` events.
- A child task panic or unexpected join error tears down only that child when
  the SDK handle can be safely recreated (`TimelineActor`, `SearchActor`) and
  emits a failure with a new generation marker. `SyncActor` panics or join
  failures move the account to `SyncFailed`; the SDK's normal reconnect loop
  handles network churn, while an internal actor crash requires an explicit
  `SyncCommand::Restart` or account restore path. An observed steady
  `SyncService::Terminated` state with the actor still alive is recoverable owner
  loss instead: the actor projects `Reconnecting`, starts one replacement, and
  projects `Running` only after matching room-list and encryption-generation
  response proofs. Replacing a `TimelineActor` does not replace the
  session-owned outbound-send workers, terminal observer, or correlation
  coordinator.
- `AccountActor` failure is fatal to that account runtime: stop children,
  drop SDK handles in runtime context, emit a redacted account failure, and
  require restore/login rather than silently continuing with unknown state.
- Hangs are detected per command by request deadlines and missing required
  progress. Idle timeline or sync streams are valid states, not hangs.

State projection keeps the reducer as the single UI state transition
mechanism:

```text
CoreCommand -> actor side effect -> CoreEvent -> AppAction
            -> reduce(AppState) -> StateDelta(generation, changed slices)
```

`AppState` contains only serializable UI data. SDK handles, task handles,
subscriptions, and keys live in actor-owned runtime state.

Desktop WebViews consume `AppState` through a selector-subscribed projection
cache, not as React-owned product state. Browser tests consume explicit
Rust-shaped snapshots/events through a transport mock; no production or test
`BrowserFakeApi` may reproduce reducers, actor transitions, sidebar/search
semantics or composer resolution in TypeScript. Runtime/background state updates use
one ordered Rust-owned state-update lane: contiguous `StateDelta` envelopes
replace only changed top-level `AppState` slices and carry a monotonic
generation. Versioned full snapshots are limited to initial attach and explicit
gap, lag, or command-watermark resync; normal product commands return typed
Core settlement/admission generations or their non-state result, never an
application snapshot. The WebView compares top-level
`DesktopSnapshot`/`AppState` slices and preserves references for unchanged
`domain`, `ui`, `sidebar`, timeline, and thread data. Components subscribe to
selectors or memoized derived selectors for the slices they render, so
background changes to unrelated slices do not force hot timeline or composer
consumers to receive freshly allocated derived arrays. A delta generation gap
atomically admits a full versioned snapshot, resets the timeline projection
cache and requests timeline replay before later deltas apply; AppState and
timeline events share the delivery lane, so state-only gap recovery is invalid.
A settlement/admission generation ahead of the store is a deterministic resync
watermark, not proof of renderer delivery or paint. Tauri Channels are reserved
for measured high-frequency streams and must not become a second React-owned
state source.

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

A session-scoped Core thread-summary projection reconciles SDK/event-cache
aggregates with accepted live reply activity once per `(room_id,
root_event_id)`. The same checked activity/summary revisions and aggregate
produce canonical Room roots, Thread/Focused root presentations, and hydrated
off-window roots. A newer accepted live reply cannot be regressed by an older
bundled SDK summary; edits retain reply identity/count, redactions select the
previous renderable reply or clear latest details, and replay/restart rebuilds
the same result from the SDK event cache. React renders this DTO and never
infers or repairs its fields from visible replies.

Thread-root projection lifetime and placement use that same session-scoped Core
owner. `ThreadRootProjectionService` retains one bounded per-root lifecycle and
renderable root snapshot; absence from a temporary Room display window is
*dormant visibility*, not deletion. Only an authoritative zero-reply aggregate
with no active root/reply, accepted redaction reconciliation, Room unsubscribe,
or session teardown clears the record. A failed disappearance check preserves
the last accepted projection. Admission is capped at 120 roots per active Room
owner and unsubscribe/session teardown releases the complete Room set.

The current Room `TimelineActor` applies those records through its existing
`DisplayProjectionState`. The manager's bounded latest-wins projection ingress
carries generation-fenced Updated or Cleared wakes; an accepted clear rebuilds
and emits the exact display removal instead of relying on a WebView cache scan.
Rust chooses root-event versus latest-reply placement, suppresses standalone
thread replies, assigns stable content/activity display identity, and emits
validated display-relative InitialItems/diffs. Rust State
mirrors explicit observed/ready/failed/cleared actions only; it does not prune
from bounded-window absence. TypeScript caches and renders the Rust projection
and retains DOM measurement, virtualization, date-divider presentation, scroll
anchoring and layout settlement; it never infers projection death or thread
placement from frontend timeline contents.

Opening an `ExistingThread` or `PinnedReply` whose first SDK Thread snapshot is
empty performs one bounded scheduler-owned backward page before any InitialItems
or `ThreadSubscribed` success is published. The accepted Rust intent travels
through AppEffect and an internal Core subscription policy; mutable reducer state
is not reread later. End-reached plus empty is authoritative empty. A non-end
empty page or SDK error takes the typed subscription-failure path and publishes
no InitialItems. The existing Room empty-hydration policy remains separately
non-fatal. `NewThreadDraft` stays immediately composer-capable and performs no
initial history page.

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
`SyncService` state observable, send-queue update stream), and the runtime must
relay that model, not fight it.

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
   on prepend. In-session room re-entry evaluates a remembered anchor against
   the first committed initial window; a later prepend cannot change an
   initially absent anchor into a restore target. Actual live-edge state while
   user input is pending supersedes programmatic-scroll echo classification.
   Matrix replacement events (`m.replace`) are separate events from
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
   timeline handle, which cancels its presentation/background tasks, but it
   does not cancel an already accepted send or discard its manager-owned
   terminal correlation. Room switching policy
   (drop immediately vs. keep-warm) is decided by the UI through these
   commands; the runtime never leaks timeline state in an unbounded map.
8. **Sends go through the SDK timeline/send queue path.** Local echo, offline
   persistence, strict FIFO retry, retry-after-reconnect, and remote-echo
   matching come from the SDK send queue, reached through the SDK UI timeline
   handle for visible timeline sends. The Rust runtime owns the product state
   projection. `TimelineManager` supervises every accepted text, reply, and
   media enqueue future, observes client-global queue terminals, and preserves
   request/submission correlation across timeline unsubscribe and actor
   replacement. Per-timeline actors own only the presentation subscription and
   guarded queue handles:
   `TimelineItem.send_state`, transaction-id keyed retry/cancel guards, and
   `RetrySend` / `CancelSend` command routing through SDK `SendHandle`s. After
   recoverable send errors, retry/cancel also re-enable the SDK room queue so
   FIFO successors are not stranded. React renders and dispatches only; it must
   not infer send legality or repair queue state locally. During ordered
   shutdown the manager gives the complete enqueue-worker set one absolute,
   count-independent five-second graceful deadline while terminal admission
   and the global observer remain live. The manager polls both workers and the
   observer during that grace period, then gives the observer one final
   non-blocking poll after worker quiescence or deadline cancellation to admit
   an already queued exact terminal. It synchronously drops the remaining
   directly-polled futures, settling their registrations before it drops the
   directly-polled terminal observer. Only then does it stop
   presentation actors and drain terminal ingress. An
   unexpected manager drop first closes terminal admission, synchronously drops
   every remaining enqueue future, and then drops the terminal observer. `Transaction`
   timeline identities are stable local-echo keys only; visible failed/sending
   state comes from `TimelineItem.send_state`. The runtime does not serialize
   sends behind a command loop.
   `RequestId` completion is connection-scoped: after process restart the SDK's
   persisted queue/local echo converges the product projection, but the new
   runtime does not replay completion for a dead connection. Durable
   cross-process submission settlement would require a separate encrypted,
   body-free outbox journal and is not part of this contract.
9. **Foreground navigation is value-driven, not mailbox-ordered.** A committed
   room selection emits its sole public `IntentLifecycle` terminal immediately
   after reducer commit. AppActor then admits an account-stable, generation-
   ordered latest-desired projection through a bounded wake channel that the
   timeline manager polls ahead of ordinary completions and background work.
   Request IDs remain correlation values, not ordering values. Old-room
   pagination, link-preview, gap-repair, live-tail, persistence, and read-state
   cleanup are not prerequisites for the terminal or cached target replay.
   Timeline actor cancel/start/begin operations use a distinct control lane;
   invalidated generations make late acknowledgements and completions inert,
   and every acknowledgement wait uses one absolute deadline.
10. **Sync uses one Element X-compatible Simplified Sliding Sync engine.**
   `SyncService` is the sole authoritative sync owner and owns the single
   `RoomListService` used by `RoomActor`; no backend selection, forced mode, or
   fallback sync path is part of the product. Session admission may check the
   homeserver's advertised Simplified Sliding Sync support and fail closed when
   it is unsupported; this compatibility check starts no sync owner and never
   selects an alternative backend. Room-list and invite projections come from
   the sole service and the SDK's committed room state, while
   `CoreCommand`/`CoreEvent` and snapshot contracts remain unchanged. A one-shot
   sync operation remains a QA/debug tool only, not the product continuous-sync
   path. Room-list bootstrap readiness and replacement are generation-fenced,
   and invite projection remains Rust-owned in `AppState.invites` rather than
   React-local state.
11. **Backpressure is defined, not accidental.** The event channel policy is
    explicit: versioned state snapshots are latest-wins (watch semantics),
    runtime state changes emit at most one `StateDelta` per batch, and
    discrete events use bounded channels with a defined recovery path (drop +
    full versioned snapshot and timeline replay). A slow, missing, stale, or
    unmounted UI must not stall Core product progress or grow memory without
    bound.
12. **SDK handles are dropped inside a Tokio runtime context.** Store-backed
    SDK clients panic (`deadpool-runtime`) when dropped outside one. Shutdown
    paths and QA binaries must respect this.
13. **Shutdown is ordered**: stop accepting commands → stop timeline
    subscriptions → stop search queues → stop sync → persist session state →
    drop SDK handles → (on logout/removal) clear credentials and stores →
    publish the final versioned snapshot / state delta.

### Desktop Window Model

The product runtime is single-window for now. The native shell creates and
restores one Tauri webview window labelled `main`, and one process-wide
`CoreRuntime` owns command dispatch, event forwarding, QA title state, and
window-state persistence. Opening additional product windows is out of scope
until a later explicit design defines per-window navigation, timeline
subscriptions, QA title ownership, persisted geometry, and shutdown behavior.
Secondary OS dialogs or system prompts do not change this product-window
contract.

### Desktop Window Lifecycle And Tray

The tray (system tray on Linux, notification area on Windows, status item on
macOS) is a platform capability owned by the Tauri adapter. The adapter attempts
to create exactly one process-wide tray icon during `setup`, and the outcome is
recorded so it can be resolved truthfully into
`NativeAttentionCapabilities.tray`. Tray creation is best-effort: a desktop
environment without a status-notifier host is a NORMAL outcome, the adapter
records the failure as a diagnostic, reports the capability as unavailable, and
the app must still run as an ordinary windowed application. The tray carries no
Matrix data — only a static tooltip and two commands, Show and Quit — so it is
not an attention rendering surface and must not display room labels, counts, or
message content.

Close-to-hide is the default window-close behavior on all three desktop
platforms. Requesting a window close hides the `main` window instead of
destroying it, after the same window-state persistence that a real close
performs. macOS keeps the hide unconditional, per platform convention, because
the application stays alive in the Dock with no window. On Linux and Windows the
behavior is gated by the Rust-owned persisted setting
`SettingsValues.window.close_to_tray` (default `true`) and additionally requires
that the tray icon was created: hiding the only window with no tray and no dock
presence would leave the process unreachable, so when the setting is off or the
tray is unavailable the close proceeds and destroys the window. React must not
own this decision; it may only render the setting toggle and dispatch a settings
patch.

`AppCommand::Shutdown` is submitted exactly once as part of process exit, and
one barrier owns it for every path. App-menu Quit and tray Quit request
application exit; the adapter intercepts the exit request, submits
`AppCommand::Shutdown`, awaits it, and only then lets the process exit. A second
exit request while shutdown is in flight is held, and the one re-delivered after
shutdown completes proceeds immediately, so a hidden product window quits
cleanly. Window destruction (an unavoidable close, or close-to-hide disabled)
means the process is ending either way, so it enters the same barrier rather
than submitting its own shutdown: whichever of the destroy path and the exit
request that follows it claims the barrier first is the single submitter, and it
is also the one that finally exits the process. The exit request's code is not
inspected — a last-window-closed request and an explicit `exit(0)` are handled
identically — and the awaited submit is bounded by the adapter's core-command
submit timeout with its error ignored, so a wedged core can never leave the exit
held forever.

### Desktop Viewport Synchronization

Live desktop viewport synchronization is Rust-owned at the Tauri adapter
boundary. On macOS, the WKWebView parent NSView bounds are the native authority;
the WKWebView frame is measured and, when needed, repaired to those bounds in one
main-thread measure/decide/apply block. The adapter never resizes the native
window in response to display density or panel presentation, and React never
stores expected geometry or owns a retry/timer state machine.

React may submit one finite, typed observation after a committed density render
or browser resize. The Rust receipt carries an in-memory monotonic generation,
the repair decision, final post-repair native origin/size alignment, and
separate JavaScript-viewport and root/body alignment booleans. Other platforms
report unsupported without speculative native repair. The receipt is the only
source for the optional private-data-free QA viewport title tokens; QA mode is
off for normal title semantics.

### Desktop Attention Surfaces

Desktop notifications, dock/taskbar badges, and unread window-title hints are
derived interaction surfaces. They do not own Matrix behavior and must be
computed from the same serializable `AppState` projection used by the UI.
Core/state may expose a notification decision surface, but it contains only
allowed UI metadata: a safe room display label, notification kind
(`mention`, `dm`, or `message`), unread notification/highlight counts, and the
coarse unread total. It must not contain message bodies, sender identifiers,
room IDs, event IDs, transaction IDs, raw SDK errors, or secrets.
The safe room label is the Rust-projected `RoomSummary.display_label`; alias or
profile relabeling refreshes that candidate label inside the reducer without
serializing room identity in the native attention candidate.

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
snapshot. `native_attention_capabilities_for_platform` is the platform-static
baseline only: capabilities that are decided by the platform alone are resolved
there, and capabilities that depend on a runtime attempt are left `Unknown` for
the adapter to overwrite before the snapshot reaches React. `tray` is such a
capability — its value is whatever the adapter observed when it tried to create
the tray icon, applied in the DTO projection alongside the process-wide display
platform, so React never sees a claimed tray that does not exist. Sound and activation hooks are candidate-scoped transient effects, so
they run only for a Rust-owned notification candidate and not for every later
snapshot that still contains unread state. Until a native Core-owned
notification dispatcher replaces the webview/window sound port,
`createDesktopBadgeSoundDispatcher` is the explicit platform-mechanics
exception: it may retain positive-edge, three-second cooldown and one in-flight
call state, but receives Rust-owned count/candidate/capability/settings facts and
must not classify Matrix attention or carry identifiers/content.
Pane-level thread attention is also Rust-owned: `AppState.thread_attention`
tracks the open thread's notification, highlight, and live-event marker counts
and reaches React only through the Tauri/TypeScript DTO.
User notification preferences are the same boundary: `SettingsValues.notifications`
is the Rust-owned persisted source of truth, and legacy settings files backfill
the default policy before any GUI reads the snapshot.
Message formatting is also projected before it reaches React:
`TimelineItem.formatted` is sanitized in Rust from Matrix `formatted_body` and
carries sanitized HTML plus plain-text/code-block metadata. Message type
display (`text`, `emote`, `notice`) and spoiler spans are projected in Rust on
`TimelineItem.message_kind` / `TimelineItem.spoiler_spans`. React must not
render unsanitized server HTML or own Matrix HTML sanitizer policy. TimelineView
may only adapt that DTO into rendered nodes, copy-code controls, spoiler reveal
state, search highlights, and CSS driven by `SettingsValues.display.code_block_wrap`.
Composer drafts are also Rust-owned product state. The reducer keeps a keyed
per-room and per-thread draft store outside transient pane state and hydrates
only the active composer into the snapshot; React may render and dispatch typed
draft changes, but it must not own cross-room or cross-thread draft survival.
The backing draft store is not sent to the webview because unsent message
content for non-visible rooms should not be exposed as snapshot data.
`koushi-core` persists that store as account-scoped encrypted local data
derived from the local unlock secret through a dedicated HKDF domain. Persistence
is debounced and size-bounded; empty stores remove the encrypted draft file.
Tauri exposes only typed draft commands (`set_composer_draft`,
`set_thread_composer_draft`) and the active composer snapshot. Every main-room
or `(room, thread-root)` target carries a monotonic causal draft revision.
`ComposerDraftRevision` is a checked `u128` in Rust and an opaque canonical
decimal string on every snapshot, Tauri, and IPC boundary. JavaScript
`number` conversion, wrapping, and saturation are forbidden.
Draft writes apply only above the stored revision. An accepted plain/reply
send, scheduled send, or prepared-upload send advances and persists an
empty-draft revision tombstone when the accepted submission is still current.
If newer input was already persisted, acceptance preserves that content while
rolling it forward to the advanced revision. Delayed pre-acceptance commands,
responses, or projections therefore cannot restore sent content or erase the
next draft. The webview reserves that acceptance fence before awaiting IPC;
immediate next input receives a newer revision and survives either completion
ordering. Main-room and thread debounce timers are target-keyed so switching
composers cannot cancel another target's pending persistence. Draft writes and
every operation that can accept a draft capture the active account owner
(homeserver, user, and device). Account transitions cancel pending timers,
discard late webview completions, and Tauri, AppActor, and AccountActor
revalidate that owner against the ready session before routing or reducing the
operation. The AccountActor check is the ordered final barrier after any
account-switch message already queued in its mailbox.
Thus an already-fired write, send, schedule, or upload from one account cannot
enter another account's state even when both accounts contain the same room
identifier. Correlated plain/reply
submission outcomes are acceptance evidence even after the target
leaves the active pane. Scheduled and prepared-upload commands instead wait
for the keyed Rust backing-store revision to advance and return that accepted
revision alongside the latest snapshot; an enqueue acknowledgement or
active-pane snapshot alone is not causal proof. Acceptance also advances a
Rust-owned target-local `last_accepted_clear_revision` only when an accepted
operation actually clears current content. React includes that token in the
active IME synchronization key; ordinary persistence and accepted preservation
of newer input do not change it. This makes the empty Rust projection
an authoritative reset even when the composition-owned textarea has not yet
observed an acknowledgement of the sent local value; ordinary stale snapshots
continue to be ignored. Legacy encrypted draft payloads backfill revision zero.

Revision history is bounded by lifecycle, not lexical target order. Non-empty,
active, debounce/IPC/submission/schedule/upload-pending, command-pending,
and touch-leased targets are protected. Only empty, inactive, zero-touch-lease
targets are quiescent tombstones; main targets retain the 128 most recent
eligible quiescent tombstones and thread targets retain 256.
Activation and command leases are touch protections: an empty target leaves
the quiescent LRU and re-enters newest when the touch protection retires. An
ordered-store persistence hold is instead a non-touching collector guard. It
may coexist with a remembered quiescent LRU position, blocks victim eligibility
without refreshing or removing that position, does not by itself enter the
persisted protected-empty bucket, and does not consume the eligible-quiescent
quota. A touch-protected empty target
that becomes store-pending is enqueued newest exactly once. The live bound is
non-empty and protected excess plus that fixed eligible-quiescent quota. Every
revision-bearing producer
acquires the exact account/target/renderer-generation lease before scheduling
or entering Core; lease admission/release and victim selection are serialized.
A same-key debounced replacement classifies victims with only the superseded
pending write's persistence-hold contribution removed, acquires the new holds
before swapping pending state, and leaves the prior save intact if admission
fails.
A retired generation cannot deliver a command or recreate collected state.
Diagnostics expose only counts and coarse lifecycle outcomes, never draft
bodies, Matrix identifiers, revisions, leases, paths, or raw errors.

Channel capacities are named constants, not scattered literals, and MUST be
sized for large-account (100+ room) sync bursts — never for the handful of
rooms in headless tests. A too-small core channel is invisible to CI and only
fails on real accounts:

- command inbox per runtime: `COMMAND_INBOX_CAPACITY`
- inter-actor command/message inboxes (AppActor -> Account -> Room/Timeline):
  `ACTOR_MESSAGE_QUEUE_CAPACITY`
- AppActor action-projection inbox (actors project `Vec<AppAction>` here at high
  volume during sync): `ACTION_QUEUE_CAPACITY`
- discrete core events per consumer: `EVENT_QUEUE_CAPACITY`
- timeline diff batches per subscribed timeline: 128
- search index mutation queue: 512

Delivery discipline by payload type:

- One-shot, non-re-projected actions — navigation (`SelectRoom`, `SelectSpace`,
  `ReorderSpaces`) and command-result projections — MUST use reliable delivery
  (`send().await`), never a drop-on-full `try_send`. A dropped one-shot action
  is lost forever; an overflow that silently drops `SelectRoom` is the
  large-account "room selection did not complete" / blank-timeline /
  unloaded-members regression class.
- Drop-on-full `try_send` is permitted ONLY for high-frequency data that is
  re-projected on the next sync (e.g. room-list snapshots), where a dropped
  update self-heals. Such channels still must be sized for large-account bursts.

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

- Treat SDK timeline order and indices as a canonical, Core-internal domain.
  One Core-owned projection transaction applies each SDK batch exactly once,
  advances the bounded display membership/mirror, and emits only display-space
  diffs. No raw canonical SDK index crosses `TimelineEvent::ItemsUpdated`.
- Emit an initial item set followed by FIFO, `VectorDiff`-shaped display
  batches. Every numeric `PushFront`, `PushBack`, `Insert`, `Set`, `Remove`,
  `Truncate`, `Clear`, or `Reset` operation is relative to the desktop display
  sequence immediately before that operation. Applying a batch to the prior
  display must equal Core's authoritative normalized display after the batch.
- Validate emitted display operations and final convergence in release builds.
  An ambiguous or invalid incremental translation recovers with one
  authoritative display `Reset` and increments the private-data-free
  `display_projection_reset_fallbacks` counter. Ordinary focused and local
  homeserver gates require a zero counter delta; Reset is exceptional recovery,
  not a normal translation strategy.
- Emit pagination state changes with `TimelineKey`, direction, state, and
  `Option<RequestId>`: `Idle`, `Paginating`, `EndReached`, `Failed(kind)`.
- Treat a pagination command as data-complete when the SDK has produced the
  diff batch or end/failure state. The core does not wait for React rendering or
  DOM measurement, because it has no DOM.
- Provide stable item identity for every renderable item: remote event ID when
  known, transaction ID for local echo, and stable synthetic IDs for separators
  or virtual items. A remote echo replaces the local transaction identity through
  an explicit diff/update, not by changing a React key in place.
- Own timeline-navigation semantics. A subscribed room timeline actor keeps the
  current projected item order plus the current fully-read marker. React may
  report viewport facts through `TimelineCommand::ObserveViewport`, but unread
  position, first-unread event, read-marker anchor, and jump-to-bottom counts
  are emitted by Rust as `TimelineEvent::NavigationUpdated`.
- Resolve jump-to-date through Rust. `AppCommand::OpenTimelineAtTimestamp`
  calls the Matrix `timestamp_to_event` endpoint from the active session and
  re-enters the existing focused-context path; React must not call raw Matrix
  endpoints or choose event ids for date jumps.

UI responsibilities:

- Maintain the render list and viewport model per `TimelineKey`; full timeline
  lists are not copied into `AppState`.
- A Tauri `InitialItems` event can be emitted before React remounts the
  corresponding `TimelineView`. If the first observed event for a key is a live
  `ItemsUpdated` batch and no resync is pending, initialize that key from an
  empty render list and apply the diff. After `ResyncRequired` or
  `ResyncMarker`, continue to require a fresh `InitialItems`.
- Keep command completion and projection ownership causally distinct.
  `InitialItems` retains the original projection request identity until the
  TimelineActor commits the exact actor/generation internally, while a separate
  causal request identity correlates each initial or idempotent replay to the
  Subscribe command that requested it. A same-key event without the matching
  causal identity must not settle a later Subscribe; failures remain correlated
  to that later command's request identity.
- A Focused main-pane anchor is not event delivery or display success. For
  Activity, search, and date navigation, Core publishes the anchor only after
  the reliable internal `FocusedProjectionCommitted` route proves the exact
  actor-owned InitialItems request/key/actor/timeline generation and target
  presence. `EnsureSubscribed` may reproject rows after consumer remount or lost
  delivery, but renderer application and DOM paint never participate in Core
  navigation settlement. Tauri only transports the resulting state/events.
- Consumer evidence is renderer-specific: TimelineView captures committed Room
  DOM evidence and App's canonical store captures Focused/Thread application for
  layout and diagnostics only. It is safe to drop on unmount and has no
  DesktopApi acknowledgement, retry/backoff delivery owner, navigation/repair
  consequence, or Core timeout.
- Before a backward pagination request can affect the viewport, capture an
  anchor item (first visible stable item ID plus pixel offset, or an equivalent
  bottom-aligned strategy). After applying the diff and after React commits the
  DOM update, restore that anchor in `requestAnimationFrame`/layout effect.
- Decide backward pagination through one state evaluator. Automatic demand is
  either a settled underfilled viewport (both projected and DOM height models)
  or near-top prefetch; an explicit top request additionally requires genuine
  wheel, touch, keyboard, or scrollbar intent. Programmatic restore and
  live-edge scroll echoes are not explicit user demand.
- Block a request until initialization/resync, projection layout, virtual
  layout, and anchor restoration are settled. Keep one request epoch active
  through backward-page projection. `Paginating`, a front insertion, or a
  replacement `Reset` projection proves that Core accepted the request. Core snapshots the observable oldest event before
  and after the SDK call and reports whether a prepend is expected. An accepted
  `Idle` terminal and an expected oldest-edge projection may arrive in either order; release the
  epoch only after both have been observed and let anchor settlement block the
  next request. If Core reports that no prepend is expected, the terminal alone
  settles the epoch so filtered/aggregation-only pages cannot wedge scrollback.
  Core emits that terminal only after releasing actor task ownership. An `Idle`
  without acceptance evidence, failure, or transport rejection releases the
  epoch but installs an owned retry fence. General failures wait for a new
  external transition; an admission-rejected `Idle` waits specifically for
  `GapRepairReleased`, so layout and gap-position projection cannot reopen the
  request while repair still owns the scheduler. End/reset releases directly.
  This prevents either event order from opening a duplicate-request
  window or losing the next wake-up, without spinning when Core rejects scheduler
  ownership.
- Re-evaluate after every transition that can add demand or remove a blocker:
  initial projection, layout/anchor settlement, genuine user scroll,
  pagination terminal state, prepend/Reset settlement, gap-position projection,
  post-terminal `GapRepairReleased`, resync replay, setting change, reset, and
  live-edge settlement. `GapPositionsUpdated` may precede a repair and is not a
  scheduler-release signal; Core emits `GapRepairReleased` only after terminal
  processing has left no queued or active gap work. Each
  transition schedules the same evaluator; scrollback does not use polling, a
  watchdog, or event-order assumptions.
- Treat scroll position, measured heights, overscan windows, and virtual-list
  cache as UI state. These values never cross into core and never affect Matrix
  ordering.
- Report only viewport facts needed by the Rust navigation projection:
  first visible event id, last visible event id, and whether the view is at the
  bottom. GUI code may scroll to Rust-provided anchors, but it must not derive
  unread counts, marker positions, or jump-to-date targets locally.

Headless QA proves the data contract: request correlation, pagination states,
diff order, generation reset, replacement/redaction/late-decryption handling.
GUI smoke proves the DOM contract: scrolling back prepends older items without
jumping, live appends do not steal the viewport while scrolled up, and end-of
history stops further automatic pagination.

Backfill observability is private-data-free. UI evaluations use diagnostic
source `timeline.backfill_evaluation` with trigger, decision, demand/blocker,
pagination state, local request epoch, item count, and 100-pixel-bucketed height
metrics. Core viewport wakes use source `core.timeline_gap_repair`, stage
`evaluation`, with viewport trigger, decision, projected gap count,
candidate-changed boolean, and scheduler phase. Neither source may include room,
event, transaction, or user identifiers, message content, or raw SDK errors.
Repeated Core evaluations with the same coarse signature are deduplicated.

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
  reset, and outgoing device verification through `koushi-sdk`
  private-data-free wrappers. Identity reset and verification continuation
  handles are held only by `AccountActor`; SDK request/SAS streams settle the
  reducer with typed actions and expose only private-data-free DTOs such as SAS
  emojis. Incoming verification request discovery is Rust-owned in
  `AccountActor`. Replayed same-peer/device/flow SAS starts are idempotent at
  the SDK boundary, and `AccountActor` adopts only one SAS continuation per
  flow; a replay must not replace its handle, observer, timeout, or acceptance.
  A valid to-device verification request that races ahead of sender-device
  discovery is retained at the crypto boundary in a bounded, timestamp-gated,
  sender/flow-deduplicated queue. The existing key-query owner is marked for
  that sender, and the request is revalidated and materialized
  after matching device keys commit. A still-missing device remains pending
  until a later matching key response or expiry; recovery must not add another
  sync owner, blind resend, fixed delay, or identifier-bearing diagnostic.
  Each generated key query has stable in-flight metadata mapping its exact
  `request_id` to the users covered by that request and its dirty-state sequence.
  Repeated, concurrent, or cancelled request collection must not overwrite an
  earlier mapping; collectors reuse the stable request and add requests only
  for uncovered dirty users. Registration uses a final dirty-state snapshot
  protected from response cleanup, so an awaited collector cannot insert stale
  metadata after a response makes its earlier snapshot clean. Complete
  response-associated processing for one `request_id` is serialized by a
  stable per-entry async gate. A handler clones and awaits that gate without a
  registry or store guard, then revalidates the entry before taking its coverage
  snapshot and verification claim. Success consumes only the pointer-matched
  entry; failure or cancellation leaves it for the next same-ID waiter. A
  waiter after successful consumption revalidates no entry and carries no
  metadata obligation. Different request IDs remain concurrent. Verification
  recovery scopes a response to the exact
  union of users present in `device_keys` and users covered by that `request_id`,
  so a failure-only response with an empty `device_keys` map still reaches the
  pending sender without claiming unrelated users.
  Pending entries are removed only after terminal validation or successful
  materialization; a fallible store read or failed initial key-query schedule
  leaves the original FIFO slot retryable without starting an out-of-band
  query. Normal materialization and key-query recovery publish the same stable
  handle through one typed incoming-request lease stream. Unknown pending
  entries, publications, subscriber generation, and the active head claim live
  under one bounded owner lock; their combined count is at most 32. A recovered
  pending slot becomes a publication under that lock, so a full queue cannot
  strand a committed replay. An active lease retains the head slot; commit
  removes it, while drop releases the claim in place without reordering.
  Subscriber generation check and head claim are one linearization point: a
  claim that wins first remains owned, while a replacement that wins first
  prevents the stale subscriber from claiming. Capacity is strict FIFO: no
  existing pending entry, publication, or active lease is evicted to admit a
  newcomer. At capacity, a newly materialized request is explicitly cancelled
  with a private-data-free protocol terminal and outgoing cancel; an unknown-
  device newcomer is not retained and does not schedule a key query. Sync cursor
  advancement never silently loses a materialized product delivery. Cache
  insertion decides existing-versus-inserted in
  one critical section, and a same-flow collision never upgrades unrelated
  cached provenance.
  Koushi has no raw to-device verification handler. It commits a typed lease
  only after its product observer channel accepts an actionable request, and it
  commits terminal/non-actionable heads immediately so they cannot starve the
  FIFO. Generic SDK raw handlers remain independent compatibility fanout:
  cancellation after only some handlers finish may repeat them on redelivery.
  Transport is at-least-once, keyed by stable `(sender, flow_id)` identity;
  `AccountActor` owns product idempotence using the full incoming
  `VerificationTarget` (peer and device) plus SDK flow id. Only an exact
  target-and-flow replay is ignored; the same flow id from a different peer or
  device is a conflict and is explicitly cancelled. Replayed SAS continuations
  are no-ops; distinct conflicting SAS handles are explicitly rejected without
  exposing raw SDK errors. An active own-user verification, including its
  pre-SAS request phase, owns the shared verification continuation and observer
  slots. Because that handle has no incoming target/flow identity available for
  replay matching, every incoming request during the own-user flow is an
  explicit conflict cancelled before any handle or observer replacement.
  Delivery and wrapper `Debug` implementations are constant/redacted and never
  delegate to request handles, owner state, clients, accounts, devices, or
  identity keys. Pending query state is explicit: `NeedsQuery`,
  `QueryInFlight`, `WaitingForExternalUpdate`, response-claimed, or
  replay-claimed. A response RAII claim is acquired before device-key response
  processing begins, including its durable commit and later cache/lock awaits.
  Cancellation or error anywhere in that window returns only claimed entries to
  `NeedsQuery`; the claim is response-token-scoped, so a failed response cannot
  reset or steal a newer same-sender response's claim. Normal still-missing
  completion explicitly transitions its entries to `WaitingForExternalUpdate`,
  so duplicates neither strand nor reschedule work.
  Overlapping responses record a per-entry committed-update generation even
  when another response owns replay; that owner must observe and replay the
  deferred commit before it may enter `WaitingForExternalUpdate`.
  Session replacement has the same ordered-owner rule. Soft-logout reauth stops
  and joins the old Account forwarder plus the SDK typed-lease worker before
  installing an observer on the new client. Removing the room handler prevents
  new dispatch; already-dispatched room-handler futures are owned and awaited by
  the SDK sync dispatch, and the old `SyncActor` stop-and-join is their session-
  replacement settlement barrier. Every observer-to-actor message carries the observer's session
  generation and is accepted only while that generation and an active session
  still match. A full actor mailbox send is raced against stop with stop
  priority; observer join has a bounded abort fallback and awaits abort
  settlement. Dropping a stop sender without awaiting the old task is not an
  ownership barrier.
  Manual one-shot sync is admitted only when that SDK client has no continuous
  or restricted sync owner: `AccountActor` rejects it before routing while a
  restricted owner is active, and `SyncActor` rejects it before any SDK call
  while a continuous owner or owner artifact remains.
  A filtered verification-only sync always requests `SyncToken::NoToken` and
  sets the SDK's opt-in `save_sync_token(false)` contract. The response still
  commits crypto, device, account-data, and to-device state and runs handlers,
  but its filter-scoped `next_batch` never replaces the persisted global room
  cursor. A fresh store therefore remains tokenless; a restored account keeps
  its previous canonical cursor across restricted sync, process shutdown,
  account switching, and SQLite reopen. The normal Simplified Sliding Sync
  owner reuses that canonical cursor directly, with no parallel in-memory taint
  ledger or repair baseline. Server-family labels, empty local room lists, retries, and longer
  waits are not cursor-provenance evidence.
  Headless E2EE QA that needs a device-readiness barrier performs a read-only,
  `qa-bin`-only user-key refresh and requires the exact device to be present
  before acknowledging. The receiver-side acknowledgement causally precedes a
  fresh device's verification request as well as encrypted sends; it does not
  create a verification flow as a probe. Multi-party waits select the relevant
  event streams against one absolute deadline rather than polling snapshots.
  Composed QA scenarios carry participant ownership explicitly: a role already
  logged in and syncing earlier in the scenario is borrowed by later stages and
  is neither duplicated nor cleaned up there. A focused scenario with no prior
  participant creates, bootstraps, owns, and cleans up exactly one instance.
  Ownership begins before a fallible login submission, not after the first
  successful post-login checkpoint. Cleanup therefore distinguishes an
  unsubmitted runtime, a submitted provisional session, and a keyed logged-in
  session; every owned role attempts the strongest available logout barrier
  before connection drop and runtime shutdown. Cleanup is best-effort across
  all owned roles so one failure cannot strand the remaining participants.
  Logout event streams are wake signals only: timeout, lag, or closure is
  followed by one final authoritative `SignedOut` snapshot observation against
  the original deadline. Helpers must not infer a gate from timing or
  manufacture another device for a role the scenario already owns.
  Closed cancellation diagnostics may expose only a fixed cancellation kind
  and whether cancellation originated locally, never raw protocol text or
  identifiers. The local core `e2ee_trust` proof exercises same-user
  two-device SAS verification, cross-signing bootstrap, passphrase-backed
  key-backup enable, encrypted seed-room backup upload, wrong-secret restore
  failure, successful joined-room restore on the second device, and identity
  reset through the sole Simplified Sliding Sync core leg on disposable local
  Tuwunel and Synapse before GUI wiring. E2EE correctness must not depend on the
  server label. No design doc
  may claim exhaustive backup-wide restore
  until the exact supported restore scope is proven or split into an explicit
  follow-up.

### Initial outbound Megolm delivery

For a newly created outbound Megolm session, the SDK's standard pre-share is the
authoritative production path and remains unconditional for encrypted sends. It
uses the normal `/keys/claim`, signed one-time/fallback-key handling, per-device
share-state update, encrypted `m.room_key`, recipient key-request and verified
device-gossip recovery, and configured backup recovery. Homeserver acceptance
commits share state but never means recipient decryption acknowledgement.

Recent outbound Megolm creation/rotation attribution is diagnostic-only. The
anonymous exported-diagnostic ledger remains runtime-local: it retains closed
reasons and anonymous room/session ordinals outside the general diagnostic
ring, is count-bounded, reports its own eviction count, and resets with
account/crypto-runtime replacement. Separately, the crypto layer persists an
exact bounded `(room_id, session_id) -> closed reason` ledger in the account's
encrypted SDK crypto store when a new outbound session is created. It restores
that ledger with the crypto machine so local Encryption details survive process
and account-runtime replacement. Missing, legacy, corrupt, or evicted evidence
is reported as unavailable and never blocks encryption or sending. Local
Encryption details for an event sent by the current device may query the exact
room/session only inside the trusted Rust/SDK boundary and receive a closed
reason; React receives only that presentation enum. Raw identifiers from the
persisted ledger never enter exported diagnostics, logs, `Debug`, QA tokens, or
the WebView, and no reason is reconstructed from aggregate counters, visible
event dates, fingerprints, or timing. Hard logout and local-data deletion remove
the ledger with the crypto store. Attribution does not change rotation, sharing,
recipient, or retry behavior.

Outbound encrypted sends follow the stock Element X / matrix-rust-sdk sequence:
synchronize room members when needed, query untracked or dirty device keys, call
`preshare_room_key` exactly once, then encrypt. Koushi adds no current-generation
readiness fence, repeated or duplicate pre-share, initial-share repair, fixed-
delay post-send re-share, manual share-index-0/resend-index-0 control,
original-recipient ledger, repair timer, or wake listener. An explicit encrypted-
room debugging control may call stock `matrix_sdk::Room::discard_room_key()` so
the next ordinary send rotates normally; it adds no sharing, retry, recipient,
or send-path state. The send path has one share step and no Koushi retry window
between sharing and encryption.

Upstream rotate-on-full-member-reload remains intact. Normal receive-side
recovery uses Matrix key requests, verified-device gossip, configured backup
lookup, and decrypt retry. Homeserver acceptance of `m.room_key` is diagnostic
evidence only and is never presented as recipient decryption proof. Read-only
initial-share and rotation diagnostics may observe the stock path when they do
not add state or control flow to it; diagnostics that exist only for deleted
repair mechanisms are removed.

### Mandatory recoverable Secure Backup

Device verification and Secure Backup readiness are independent Rust-owned
admission facts. Verification starts the normal sync owner so receiving and
local decryption can continue. `AppState.secure_backup_gate` remains blocking
until `koushi-sdk` has established that Recovery is complete and the existing
trusted backup is enabled locally. Pending room-key upload is observed as
health/progress after that authority is established; it does not keep the
normal shell hidden or close ordinary encrypted sending. A transient failure
before initial authority is established remains blocking and exposes explicit
retry/diagnostics instead of entering an automatic retry loop. React never
infers readiness from a click or from backup existence alone.

An existing backup without its local decryption key is recovered in place.
Automatic setup is permitted only when the authoritative probe reports no
server backup. Koushi never calls a destructive backup fix/reset path from this
gate. If backup was explicitly disabled, re-enabling requires a user action
which states that the account-wide setting also affects other Matrix clients.

Koushi encrypted user-content sends use only the SDK's normal encryption setup
and recipient-device key sharing sequence above. The SDK backup worker uploads
new and rotated Megolm sessions asynchronously. Core continuously observes
backup state changes and runs a single-owner periodic inspection while the
verified session is active; a degraded backup is visible and diagnosed but does
not turn an otherwise valid encrypted send into `NotSent`.

## Room Timeline Gap Repair

Room timeline continuity is a Rust-owned, proof-based contract. The Matrix SDK
owns opaque pagination tokens, persisted linked chunks, targeted cache mutation,
deduplication, and boundary/start validation. `koushi-sdk` exposes only a
token-free Rust adapter. SDK gap descriptors are snapshot-scoped actor-private
values; they never enter `AppState`, IPC, diagnostics, or Koushi persistence.

Core owns one generation-guarded repair scheduler per room and projects
`Unknown`, `Inspecting`, `Healthy`, `Incomplete`, `Repairing`, or
`FailedIncomplete`. `Healthy` requires an SDK continuity proof. Live sync,
local emptiness, edge pagination completion, and visually adjacent event rows
are not continuity proofs. Restart and reconnect re-inspect rather than restore
opaque handles.

Viewport observation is an explicit scheduler wake-up, not a repair decision.
Core selects the projected gap intersecting the viewport, falling back to the
gap nearest the live edge, and queues an automatic inspection only when that
candidate or its relation changes. Repeated observations of the same candidate
are idle. A wake that arrives during an active inspection or an outstanding
Core relay/display-projection commit remains queued and is released by the
exact actor/timeline/repair/batch-fenced internal commit signal. DOM paint and
renderer acknowledgements are never scheduler prerequisites.

Room live-edge recovery is a distinct bounded scheduler intent, not a
relaxation of viewport-driven automatic repair. After the initial Core
projection commit, and again when the actor-private live-edge target changes,
Core may repair the newest SDK descriptor even when its raw boundary
is an aggregated relation with no standalone row. This intent reveals at most
one cached chunk per request, has a small actor-generation batch ceiling, and
stops on unchanged topology or zero progress. It never invents a gap row or
causes unrelated historical gaps to become ordinary automatic work.
Simplified Sliding Sync supplies both retained per-room commit observations and
a retained global response-commit fence published after event-cache topology
mutation.
When the current response contains an active room, room-entry repair may admit
only that response's exact opaque gap. When the global fence proves that an
active room was omitted from the incremental response, Core may authoritatively
inspect persisted topology and admit only its newest gap as the same bounded
live-edge intent. An explicit update for the room with no timeline gap still
closes the intent; omission is not inferred from a timeout or pre-commit room
update broadcast.
When that intent first selects a projected descriptor, it remains live-edge
work after the exact actor/timeline/repair/minimum-batch internal projection commit. It downgrades to ordinary automatic repair
only after a joined/start-reached descriptor was actually selected by the
unprojected live-edge fallback, so repairing another visible gap cannot discard
the live-edge recovery target.

Timeline gaps cross the WebView boundary only as Rust-positioned, content-free
rows with coarse state. React renders those rows, reports presentation-only
viewport facts, preserves the viewport anchor while repair diffs apply, and
dispatches typed retry/navigation commands. It must not infer gaps, construct
tokens, select cache boundaries, or synthesize **Start of conversation**.

Manual, live-edge, and automatic repair share the same bounded scheduler.
Normal product
paths must never repair by removing a room event cache. Failure, cancellation,
stale generations, and unsupported missing-token recovery preserve existing
events and expose a retryable incomplete state. Candidate-driven automatic
repair retains the zero cached-chunk budget; viewport motion cannot silently
turn it into cache hydration or an unbounded background backfill loop.

Every SDK gap-repair publication is causally tagged through the UI timeline
relay. Core fences continuation on the exact actor/timeline/repair/minimum-batch
`GapProjectionRelayed` commit containing the final tagged publication, not on
whichever live batch happens to arrive next. The SDK UI layer settles each tag
at its observable Core boundary: filtered repairs with no remote item report no
projection, while aggregation-only repairs emit one tagged remote-item barrier.
Gap-only cache reveals likewise report that no projection was published. A
lag-triggered `InitialItems` replay repairs renderer transport independently;
Core repair continuation never waits for that replay, WebView application,
renderer layout signal, or DOM paint. If the observable update is lost to
a lagged SDK subscriber, Core bounds the internal relay settlement wait and
exposes a retryable repair failure instead of leaving the timeline permanently
`Repairing`.

See
`docs/superpowers/specs/2026-07-03-room-timeline-cache-repair-design.md`.

## QA Model

QA is layered; GUI automation is the last and weakest layer, never the
primary correctness gate.

1. **Unit tests** — network-free: routing, redaction, unauthenticated command
   rejection, state transitions with fake ports, normalization, reducer.
2. **Local homeserver QA** — disposable Tuwunel/Synapse servers, synthetic
   users, the `koushi-qa` headless binary speaking `koushi-protocol`
   `CoreCommand`/`CoreEvent` through Core (never direct SDK wrapper calls).
   Covers login, sync, room/space create, invite receipt,
   invite accept/decline, DM start, bidirectional messaging, room list, logout
   cleanup, and stdout/stderr redaction through the sole Simplified Sliding Sync
   engine.
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
behavior lands in `koushi-core`, is exercised by `koushi-qa` through
`koushi-protocol` `CoreCommand`/`CoreEvent` against disposable local
Tuwunel/Synapse homeservers (and real homeserver QA where that gate applies),
and only then is wired through
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
