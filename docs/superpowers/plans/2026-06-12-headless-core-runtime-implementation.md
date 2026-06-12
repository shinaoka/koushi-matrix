# Headless Core Runtime Implementation Plan

Date: 2026-06-12
Status: active plan.

Implements the migration spec
(`../specs/2026-06-12-headless-core-runtime-design.md`) toward the normative
architecture (`../../architecture/overview.md`), phase by phase, under the
rules in `../../policies/engineering-rules.md`.

Workflow: headless-first, local-server-first (overview, QA Model). The core
QA binary is the development driver: every phase extends it with the new
commands and asserts on `CoreEvent`/`AppStateSnapshot` against disposable
Conduit/Tuwunel servers before any GUI work.

## Redesign Protocol (applies to every phase)

Each phase will expose gaps the design could not foresee. When implementation
contradicts the canon or hits an unspecified case:

1. **Stop coding on that point.** Do not improvise an undocumented behavior.
2. **Record the gap**: what the design assumed, what the SDK/homeserver/code
   actually does.
3. **Amend the canon first** — `docs/architecture/overview.md` and, if a rule
   changes, `docs/policies/engineering-rules.md`. Bump `Last amended`.
4. **Sync the dated spec** if the public API changes, and add an entry to the
   Changelog at the bottom of this plan.
5. **Then implement** to the amended design. Code that diverges from the canon
   must not land.

Every phase exit includes a docs-sync check: no known contradiction between
landed code and the canon documents.

## Model Assignment

Implementation is delegated by phase, with escalation tied to the Redesign
Protocol:

- **Default implementer: Sonnet** (claude-sonnet-4-6). The canon, the spec,
  and the per-phase QA gates are deliberately concrete enough that most phase
  work is "write the specified contract in Rust and make the gates pass".
- **Phase 1 is implemented by a stronger model** (Fable 5 / Opus). The API
  boundary types, channel topology, and executor abstraction are the
  foundation every later phase builds on; defects here propagate everywhere.
- **Canon amendments always escalate.** When the implementing model hits a
  design gap (Redesign Protocol step 1), it stops and reports; the redesign
  decision and the canon amendment are made by the strongest available model
  of the agent's family (Claude: Fable 5 / Opus; Codex: the highest GPT
  version, never a mini/lightweight tier) or by the user, never improvised
  by the implementing model. After the canon is amended,
  the implementing model resumes against the updated design. Phases 3 and 5
  are expected to trigger this most (see their gap watchlists).
- **Phase exits are reviewed by a stronger model**: code review plus the
  docs-sync check, before the phase is declared done.
- Quality is enforced by the gates (headless QA, secret scan, redaction
  tests, wasm checks), not by trust in any model's self-report.

## Phase 0 — Guardrails

Goal: enforcement exists before the code it must constrain.

- Create the `matrix-desktop-core` crate skeleton in the workspace.
- Wire the secret scan gate (pre-commit and CI; excludes `vendor/`,
  `.local-secrets/`, generated artifacts).
- Add `wasm32-unknown-unknown` check for `matrix-desktop-state` and
  `matrix-desktop-search` (Platform Portability rule 4).
- Add the release-build check that debug/test credential injection paths are
  compiled out.

Exit gate: all gates runnable locally and green on the current tree.

## Phase 1 — Core Boundary (spec Milestone A)

Goal: the public runtime API exists and its contracts are tested, with no
Matrix behavior yet.

- Identity and transport types: `RuntimeConnectionId`, `RequestId`,
  `CoreConnection` (allocates `next_request_id()`), `CommandSubmitError`,
  `TimelineKey`/`TimelineKind`, `TimelineGeneration`, `TimelineBatchId`,
  pagination enums, `CoreFailure` with per-category kinds.
- `CoreCommand`/`CoreEvent` enums; `AppActor` skeleton: routing, ordered event
  broadcast, `StateChanged` projection through the existing reducer.
- Channel topology per the backpressure rules: latest-wins snapshots, bounded
  discrete event queues with the named capacities.
- Executor abstraction wrapper (no direct `tokio::spawn`/`tokio::time` in
  actor logic).

Tests (network-free): redacted `Debug` for secret-bearing commands,
unauthenticated command rejection, request-id correlation including
`InvalidRequestId` on connection mismatch, snapshot coalescing, queue
overflow behavior.

Gap watchlist: broadcast/watch channel semantics vs. the documented overflow
protocol; executor wrapper ergonomics on multi-thread tokio.

## Phase 2 — Store + Account Actors

Goal: login, restore, logout work headlessly against a local server.

- `StoreActor`: credential store ports, per-account store paths, key
  derivation, fail-closed `LocalEncryptionUnavailable`, debug/test injection
  policy behind compile-time gates.
- `AccountActor`: `LoginPassword`, `RestoreSession`, `Logout`,
  `SwitchAccount` skeleton; shutdown order; SDK handles dropped in runtime
  context.
- Core QA binary v0: login A/B, logout cleanup, stdout/stderr redaction
  assertion, on Conduit and Tuwunel.

Gap watchlist: keychain behavior in unattended runs; multi-account store path
collisions; what `SwitchAccount` means before sync exists.

## Phase 3 — Sync Actor With Capability Probe

Goal: continuous sync lifecycle on both sync backends.

- MSC4186 capability probe; `SyncService`/`RoomListService` preferred,
  explicit `LegacySync` backend otherwise; selected backend emitted as a
  redacted event/diagnostic field.
- Sync state machine (starting/running/reconnecting/failed/stopped),
  supervision per the ownership tree, `SyncCommand::Restart`.
- QA: sync reaches running on both servers; backend assertion per server;
  stop on logout.

Gap watchlist: which backend Conduit and Tuwunel actually select (this
decides how real the `LegacySync` path is); offline-mode behavior of
`SyncService`; reconnect semantics differences between backends. Expect a
canon amendment here — the legacy room-list normalization contract is the
least-validated part of the design.

## Phase 4 — Room Actor (spec Milestone D)

Goal: room list and room operations on both backends.

- Normalization to `SpaceSummary`/`RoomSummary` from `RoomListService` and
  from legacy sync state; unread counts; DM classification; space-filtered
  lists.
- `CreateRoom`, `CreateSpace`, `SetSpaceChild`, `InviteUser`, `JoinRoom`,
  `SelectSpace`, `SelectRoom`.
- QA: create/invite/join/space-child flows A↔B; room list assertions on both
  backends; send permission check (joined-room requirement).

Gap watchlist: parity of room-list data between the two backends (unread
counts and DM detection may differ); ordering stability of summaries.

## Phase 5 — Timeline Actor (spec Milestone E)

Goal: the full timeline data contract, the heart of the design.

- `Subscribe`/`Unsubscribe` lifecycle keyed by `TimelineKey` (Room, Thread,
  Focused); generations; `InitialItems`/`ItemsUpdated` diff batches with
  stable item identity; `ResyncRequired` overflow path.
- Directional `Paginate` with per-direction `PaginationStateChanged`
  (`Idle`/`Paginating`/`EndReached`/`Failed{kind}`); forward pagination only
  on `Focused`.
- Send through the SDK send queue (local echo, transaction-id matching,
  offline retry relay); `EditText`, `Redact`; replacement/late-decryption
  projection per Async rule 4.
- QA: subscribe, backward paginate to `EndReached`, diff ordering, generation
  reset, A↔B send/receive, edit and redaction reflected in diffs.

Gap watchlist: SDK pagination-status mapping to the four documented states;
thread timeline support level in the vendored SDK; send-queue event mapping;
batch sizing vs. the 128-capacity diff queue under fast backfill.

## Phase 6 — Search Actor

Goal: encrypted search through the command/event boundary.

- Ngram candidates → canonical-text verification → results; document-level
  index mutations for edits/redactions/late decryptions; unresolved
  replacements not indexed.
- QA: index/search round trip with CJK text, edit and redaction mutations
  verified through search results.

Gap watchlist: index encryption key lifecycle on logout; reindex cost on
generation resets.

## Phase 7 — Tauri Integration (spec Milestone F)

Goal: the GUI becomes a pure transport client of the core.

- `src-tauri` holds `CoreRuntime`, attaches a `CoreConnection`; all direct
  SDK wrapper calls removed; fixture backend demoted to dev/demo preview.
- Webview threat-model items: release devtools disabled, strict CSP, no IPC
  payload tracing, secrets one-way.
- React timeline applies diffs with anchor-based scroll restoration per the
  Viewport/Scrollback contract.
- GUI smoke (existing scripts): scrollback anchor stability, live-append
  viewport behavior, `EndReached` stops auto-pagination.

Gap watchlist: Tauri event-channel throughput for diff batches; serialization
cost of snapshots; where typed TS bindings for `CoreCommand`/`CoreEvent`
come from (codegen vs. handwritten).

## Phase 8 — Real Homeserver Gate (spec Milestone G)

Goal: release-preflight confidence on a real homeserver.

- Recovery flows (`SubmitRecovery`, `NeedsRecovery` states) — local servers
  do not exercise these fully; this phase owns them.
- Encrypted store restore, sync lifecycle, room list, timeline, send, search
  smoke, logout, account switch, all through `CoreCommand`/`CoreEvent`.
- Debug/test credential loading per the secrets rules; secret scan in the
  preflight.

Exit gate: `qa:real-homeserver` green; release preflight documented.

## Phase 9 — Cleanup And Canon Sync

- Remove dead `AppEffect` paths and superseded wrappers; decide the
  `matrix-desktop-auth` → `matrix-desktop-sdk` rename.
- Final docs-sync pass: overview, engineering rules, spec status, AGENTS.md
  operational notes.
- Mark this plan completed; open items become new dated specs.

## Changelog

- 2026-06-12: plan created.
- 2026-06-12: Phase 2 review — three gaps escalated by the implementing
  model and resolved in the canon: (1) account store bootstrap invariant
  added to the overview (storeless login client must never sync; the
  store-backed restored session replaces it before any sync/E2EE traffic);
  (2) `SwitchAccount` defined as ordered shutdown without credential
  clearing plus store-backed restore of the target account; (3) one
  `CoreRuntime` per synthetic QA user legitimized in the spec QA section as
  the two-device topology; multi-account-in-one-runtime is account-switch
  QA's job.
- 2026-06-12: Phase 2 landed (StoreActor, AccountActor, headless-core-qa on
  both servers). Post-review additions: `CoreFailure::SessionNotFound` for
  restore/switch of accounts with no stored session (escalated kind gap);
  the core QA binary hard-refuses to run against the OS keychain after a
  Keychain prompt fired during the first implementation iteration
  (engineering-rules Secrets rule 8 incident); login follows the store
  bootstrap invariant with fail-closed abort (best-effort device logout +
  credential rollback) when the encrypted store cannot be created.
- 2026-06-12: Phase 3 landed (SyncActor, capability probe, sync QA on both
  servers). Canon-relevant evidence: **both Conduit and Tuwunel select
  `SyncService` (MSC4186)**; both advertise `org.matrix.simplified_msc3575`
  in `/versions` `unstable_features`. The `LegacySync` path exists and is
  tested at the unit level (classify_sdk_sync_error, empty-versions probe)
  but is unreachable in the local QA matrix because both local servers
  support MSC4186. Design gap for escalation: the LegacySync path (including
  legacy room-list normalization in Phase 4) cannot be validated against a
  local server; it requires a real homeserver known to lack MSC4186, or a
  capability-probe mock.  The `SyncActor` is colocated as a child task under
  `AccountActor` (spec: "Actor Deployment And Supervision — boundaries define
  ownership, not one task per actor"). Ordered shutdown wires sync stop
  before SDK handle drop per overview.md Async rule 12 step 4.
  All 25 unit tests green, 0 warnings, secret scan ok, release-gate
  structural ok, both-server QA green.
- 2026-06-12: Phase 3 review resolution — the LegacySync validation gap is
  closed with a debug/test-only forced-backend override
  (`MATRIX_DESKTOP_QA_FORCE_SYNC_BACKEND=legacy`, compiled out of release
  builds; the value must be exactly `legacy`, anything else probes
  normally), because legacy `/sync` works against MSC4186-capable servers
  too. The local QA script now runs two core QA legs per server with fresh
  data/cred dirs: probed (expects `SyncService`) and forced-legacy (expects
  `LegacySync`); `MATRIX_DESKTOP_LOCAL_QA_EXPECT_SYNC_BACKEND` makes
  backend drift in either direction a QA failure. Result: all four legs
  green on Conduit and Tuwunel; the first end-to-end run of
  `run_legacy_sync_loop` needed **no fixes** — login, sync Started/Running,
  stop, restore, and logout all passed unchanged on the legacy backend.
  The SyncService `Error → SyncFailureKind::Http` catch-all mapping was
  reviewed and accepted as the conservative choice. Unit tests now 26
  (adds the override value-parsing test); `cargo check --release` confirms
  the override symbols compile out of release builds.
- 2026-06-12: Phase 0 landed. Notes: (1) the repo has no hosted CI, so all
  gates run locally (pre-commit hook + npm scripts + release preflight) until
  CI infrastructure exists; (2) the release-gate check found and fixed a real
  violation — `MATRIX_DESKTOP_QA_LOGIN_PIPE` was honored in release builds;
  the QA login pipe cluster is now compiled out of release
  (`#[cfg(any(debug_assertions, test))]`); (3) QA behavior toggles that carry
  no credentials (`MATRIX_DESKTOP_QA_TITLE`, `MATRIX_DESKTOP_SKIP_*`) remain
  ungated by design — Secrets rule 2 covers credential injection only.
- 2026-06-12: model assignment added — Sonnet implements by default, Phase 1
  and all canon amendments escalate to a stronger model, phase exits reviewed
  by a stronger model.
