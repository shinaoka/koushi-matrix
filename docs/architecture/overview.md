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
- `apps/desktop` — view and interaction code only.

GUI, Tauri, CLI, and QA all use the same command/event boundary. There is no
standalone daemon; the runtime is in-process.

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

State projection keeps the reducer as the single UI state transition
mechanism:

```text
CoreCommand -> actor side effect -> CoreEvent -> AppAction
            -> reduce(AppState) -> StateChanged(AppStateSnapshot)
```

`AppState` contains only serializable UI data. SDK handles, task handles,
subscriptions, and keys live in actor-owned runtime state.

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
2. **Commands never return data.** Results are observed as events and
   snapshots so that GUI, CLI, and QA observe identical behavior.
3. **Every command carries a client-generated `request_id`.** Every command
   result event carries that same `request_id`, whether the result is success
   or failure. Failures are emitted as
   `OperationFailed { request_id, failure }`; successes such as room creation,
   join, send completion, pagination state changes, and search completion carry
   `request_id` in their domain event. Events that can also occur without a
   client command — e.g. pagination state transitions triggered by SDK
   coalescing or sync gap-fill — carry the originating `request_id` when one
   exists (`Option<RequestId>`). Message sends additionally carry a
   `transaction_id` used for local-echo matching end to end.
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
   standalone messages.
5. **Pagination is stateful and observable.** Every timeline exposes
   pagination state events: `Idle`, `Paginating`, `EndReached` (timeline
   start hit). The UI uses these to drive spinners and to suppress duplicate
   pagination requests while one is in flight. The runtime relays the SDK's
   pagination status; reaching the start of history must be surfaced, or the
   UI will paginate forever.
6. **Timelines are addressed by `TimelineKey`, not bare room IDs.** A
   `TimelineKey` identifies a room live timeline or a thread timeline (and,
   later, an event-focused timeline). Subscribe, unsubscribe, paginate, send,
   edit, and redact all take a `TimelineKey`, so threads paginate and operate
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

QA waits on events, never on fixed sleeps. QA asserts on `CoreEvent` and
`AppStateSnapshot`, never on logs. Diagnostics are structured, redacted,
and not a source of truth.

## Relationship to Dated Specs

The 2026-06-12 headless core runtime spec
(`docs/superpowers/specs/2026-06-12-headless-core-runtime-design.md`)
defines the migration milestones (A–G) toward this architecture. This
overview amends its public API in the following ways, found in the
2026-06-12 design review; implementations follow this document:

- Timeline commands take a `TimelineKey` (room/thread), not `room_id`
  strings, so threads can paginate (rule 6).
- `UnsubscribeTimeline` exists; timeline lifecycles are explicit (rule 7).
- All command result events carry a `request_id`; success and failure both
  correlate to the initiating command (rule 3).
- `TimelineEvent` includes pagination state (`Idle`/`Paginating`/
  `EndReached`) and diff-based item updates; snapshots exclude timeline
  bodies (rules 4–5).
- Sends use the SDK send queue; sync uses capability-probed SDK services with
  an explicit `LegacySync` fallback for homeservers without MSC4186
  (rules 8–9).
- `CoreFailure` variants carry non-secret `kind` values.
- The webview secret threat model above is part of the security policy.
