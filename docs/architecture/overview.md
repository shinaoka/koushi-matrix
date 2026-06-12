# Matrix Desktop Architecture Overview

Status: normative. This is the long-term blueprint for the whole application.
Dated specs and plans under `docs/superpowers/` are implementation guides
toward this document and must not contradict it. Amend this document first
when a design change is needed, then update or supersede the affected specs.

Last amended: 2026-06-12.

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
matrix-desktop-auth (-> matrix-desktop-sdk) thin matrix-rust-sdk adapter
matrix-desktop-state                        pure reducer + snapshot DTOs
matrix-desktop-search / matrix-desktop-key  search verification / credential store
        |
matrix-rust-sdk (vendored)                  sync, timeline, send queue, crypto
```

Crate responsibilities:

- `matrix-desktop-state` — pure. `AppState`, `AppAction`, `reduce()`,
  serializable snapshot DTOs. No SDK handles, no Tauri, no async.
- `matrix-desktop-auth` — low-level SDK adapter (login, restore, recovery,
  sync, room, timeline, search primitives). No app state, no QA orchestration.
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
  broadcast and snapshots.
- `AccountActor` (per account/device) — SDK session ownership,
  login/restore/recovery/logout, account switch, child shutdown.
- `SyncActor` — continuous sync lifecycle
  (starting/running/reconnecting/failed/stopped).
- `RoomActor` — room list normalization (`SpaceSummary`/`RoomSummary`),
  create/invite/join/space operations, unread counts, DM classification.
- `TimelineActor` (per room/thread timeline) — subscription, diffs,
  pagination, send/edit/redaction relay.
- `SearchActor` — ngram candidates, canonical-text verification,
  document-level index mutations for edits/redactions/late decryptions.
- `StoreActor` — credential store access, store/search keys, per-account
  paths, cleanup, debug/test secret injection policy.

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
8. **Sends go through the SDK send queue.** Local echo, offline persistence,
   retry, and remote-echo matching come from the SDK send queue and are
   relayed as events. The runtime does not serialize sends behind a command
   loop.
9. **Sync uses capability-probed SDK services, not ad hoc polling.** Prefer
   `SyncService`/`RoomListService` when the homeserver supports MSC4186. If
   `SyncService` is unavailable for a target homeserver, the `SyncActor`
   switches to an explicit `LegacySync` backend using SDK `/sync` primitives
   while preserving the same `CoreCommand`/`CoreEvent` contract. The selected
   backend is emitted as a redacted diagnostic/event field so QA can assert it.
   `sync_once`-style one-shot polling remains a QA/debug tool, not the product
   continuous-sync path. Note that `RoomListService` is built on sliding sync:
   on the `LegacySync` backend, `RoomActor` must normalize the room list from
   legacy sync state (`Client::rooms()` plus sync updates) instead. Because the
   local QA matrix includes homeservers without MSC4186, this legacy room-list
   path is a fully implemented, QA-gated product path, not a stub.
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
  plaintext stores or plaintext search indexes.
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
- **Device verification and cross-signing** are not yet designed. They are
  account-level security features and will live under `AccountActor` with
  their own commands/events; until then, no design doc may claim E2EE trust
  UX completeness.

## QA Model

QA is layered; GUI automation is the last and weakest layer, never the
primary correctness gate.

1. **Unit tests** — network-free: routing, redaction, unauthenticated command
   rejection, state transitions with fake ports, normalization, reducer.
2. **Local homeserver QA** — disposable Conduit/Tuwunel servers, synthetic
   users, a core QA binary speaking `CoreCommand`/`CoreEvent` (never direct
   SDK wrapper calls). Covers login, sync, room/space create, invite/join,
   bidirectional messaging, room list, logout cleanup, and stdout/stderr
   redaction. It records and asserts the selected sync backend
   (`SyncService` or `LegacySync`) so server capability gaps are visible.
3. **Real homeserver QA** — required before GUI-level confidence claims:
   HTTPS login, recovery, encrypted store restore, sync lifecycle, room list,
   timeline, send, search smoke, logout, account switch.
4. **GUI smoke** — thin sanity layer on top, subject to the automation rules
   in the policies document.

**Implementation workflow: headless-first, local-server-first.** New Matrix
behavior lands in `matrix-desktop-core`, is exercised through
`CoreCommand`/`CoreEvent` against disposable local Conduit/Tuwunel homeservers
(and real homeserver QA where that gate applies), and only then is wired through
Tauri into React. Matrix behavior must not be introduced first in GUI or Tauri
code and back-filled into core later.

QA waits on events, never on fixed sleeps. QA asserts on `CoreEvent` and
`AppStateSnapshot`, never on logs. Diagnostics are structured, redacted, and
not a source of truth.
