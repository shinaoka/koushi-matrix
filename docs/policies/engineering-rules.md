# Engineering Rules

Status: normative detailed policy, read by task. This document extends the
root durable rules in [REPOSITORY_RULES.md](../../REPOSITORY_RULES.md) with
concrete policy for secrets, logging, async/runtime behavior, GUI automation,
text input, and build gates. `AGENTS.md` is the router; operational setup,
lanes, and recovery steps live in `docs/agents/`. Feature-level state
boundaries live in [overview](../architecture/overview.md),
[state machines](../architecture/state-machine.md), and
[state ownership](../agents/state-ownership.md); this document keeps only the
policy those owners do not state.

## Design Simplicity

1. Add a guard, retry, fallback, duplicate state, or test hook only for a
   reproduced failure or named invariant.
2. Give each lifecycle state machine one owner and one explicit state model; do
   not synchronize parallel booleans.
3. When an artificial failure mechanism creates a boundary problem, remove it
   instead of adding boundary handling around it.
4. Put the smallest necessary guard at the authoritative boundary. This never
   weakens security, privacy, trust-boundary validation, data-loss prevention,
   accessibility, or explicitly approved requirements.

## Secrets and Private Data

Never log, print, commit, or store in fixtures:

- access tokens, passwords, recovery keys or recovery codes
- OAuth refresh tokens, PKCE verifiers, authorization callback query strings,
  and delegated-auth client credentials
- SDK store keys, search index keys, local unlock secrets
- raw request/response bodies
- real account private data; real room names or real discussion content in
  docs, tests, or mocks

Allowed only in debug/test contexts: synthetic local QA credentials, local
homeserver URLs, synthetic room/event IDs. Allowed in UI state: user ID,
device ID, room ID, event ID, visible message body, attachment filename.

Rules:

1. Secret-bearing types must use zeroizing wrappers with redacted `Debug`
   (`finish_non_exhaustive()` style). This includes command payloads:
   login requests redact username/password/device name; recovery requests
   redact recovery material; send/edit redact bodies in `Debug` and errors.
2. Release builds must reject environment-variable credential injection and
   the file-based credential store. The gate is compile-time (debug/test
   only) and CI must verify release builds ignore these paths.
3. QA credentials enter processes via FIFO (`KOUSHI_QA_LOGIN_PIPE`)
   or the gated file credential store — never via argv, never typed by
   coordinates, never echoed to a terminal, never in screenshots, logs, or
   committed scripts.
4. Do not pass the parent shell environment wholesale into QA child
   processes. Filter out secret-like variables (API keys, tokens,
   passwords) before spawning.
5. Do not store post-login real-account screenshots; they can contain room
   names, Matrix IDs, message bodies, attachment names. Use
   private-data-free QA window-title tokens. `--allow-private-screenshots`
   is restricted to explicitly approved test accounts and ignored artifact
   paths.
6. QA profile names must be synthetic and non-secret. Profile data lives
   under ignored `.local-secrets/qa-profiles/<name>/data`.
7. A secret scan gate runs before commits **and** in CI (pre-commit hooks
   can be bypassed). It excludes `vendor/`, `.local-secrets/`, and
   generated artifacts.
8. An unexpected macOS Keychain prompt during unattended QA is an
   automation failure, not something to click through. Fix the run's
   environment instead; the variables are listed in
   [QA lanes](../agents/qa-lanes.md#real-account-lanes).
9. OS notifications, badge labels, and QA window-title tokens are
   private-data-minimized surfaces. By default they may include only a safe
   room display label, notification kind (`mention`, `dm`, `message`), and
   aggregate unread/highlight counts. They must not include message bodies,
   sender identifiers, room IDs, event IDs, transaction IDs, raw SDK errors,
   or secrets. Native notification clearing failures must not mutate Matrix
   state or surface as React rendering failures. Backfill/prepend diffs and the
   current user's own messages must not create thread notification markers.
   The Rust-owned attention DTOs, capability profiles, snapshot-mapped versus
   candidate-scoped native effects, permission-prompt limits, Space rail and
   thread badges, and sound policy are specified in state ownership "Threads
   and attention" and overview "Desktop Attention Surfaces".
10. Device-local settings are non-secret product state, but they are still a
   privacy boundary. Settings files may contain only typed preferences such as
   locale, theme, font/emoji choice, keyboard behavior, and notification
   policy. They must not contain Matrix credentials, tokens, recovery material,
   local unlock secrets, SDK/search keys, raw Matrix session JSON,
   room/event/user IDs, message bodies, attachment filenames, search queries,
   or raw SDK errors.
   Recent emoji stores at most 24 canonical non-empty tokens, never arbitrary
   unbounded input, and must not be printed in diagnostics. Derived display
   profiles (`LocaleDisplayProfile`, `TypographyDisplayProfile`) are non-secret
   profile data only: they may carry platform/capability and asset-status
   tokens, but not account identifiers, content, local paths, raw errors, or
   credentials.
   WebView `localStorage` is legacy migration input only. The migration reader
   has a closed key allowlist, strict bounds and typed parsing, and submits typed
   Rust commands. It removes no key until a persisted one-time import marker and
   the authoritative Rust snapshot prove persistence; rejection, load/persist
   failure, account replacement or shutdown retains every unconfirmed key, and a
   retained stale key must never overwrite a later Rust edit. Device-global typed
   preferences use `SettingsValues`; preferences containing Matrix identifiers or
   free-form account data use a privacy-reviewed account-scoped encrypted store.
   Link-preview global defaults are Rust-owned display settings with separate
   preferences for non-encrypted rooms (`url_previews_enabled`, default true)
   and encrypted rooms (`encrypted_url_previews_enabled`, default false). The
   encrypted-room default remains privacy-conservative, but it is an explicit
   user setting rather than a hidden UI-only special case. Settings schema
   versioning is in state ownership "Settings, composer, and scheduled send".
   Per-room URL-preview overrides are not settings-file data because the key is
   a Matrix room identifier. They live in Rust-owned non-persisted
   `AppState.link_preview_settings.room_overrides`, are changed only through a
   typed room-override command, and are exposed to React as current snapshot
   state. `SettingsValues` / `SettingsPatch` must not gain room-, event-, or
   user-id keyed maps; a durable per-identity preference needs a dedicated
   privacy-reviewed local store.
   Unsent composer drafts and scheduled-send bodies are account-scoped
   encrypted message content, not settings; their storage, revision, lease, and
   quota contract is the composer-draft paragraphs of overview (under "Async
   Design Rules") and state machines. Bounded
   draft persistence prioritizes targets with non-empty content before
   revision-only tombstones. Timer cancellation or a second racing empty save is
   not a correctness fence. Normal `Debug`, QA logs, issue evidence, and
   window-title tokens redact scheduled bodies, room ids, server delayed-event
   handles, and transaction ids.
11. E2EE trust diagnostics are kind-only. Verification, cross-signing,
   key-backup, and identity-reset commands/events may expose structured state to
   the UI, but normal `Debug`, QA logs, and window-title tokens must redact
   account keys, verification target user/device IDs, backup versions, raw SDK
   errors, identity-reset auth details beyond UIAA/OAuth/unknown, and all key
   material.
   Secure Backup recovery-key reveal: a key the SDK produced (setup, re-enable,
   confirmed `recovery().reset_key()`, passphrase change, or the
   verification-gate identity bootstrap) exists only in the
   reveal state (`SecureBackupSetupState::RecoveryKeyReady` or
   `SecureBackupPassphraseChangeState::Changed`) as zeroizing
   `RecoveryKeyMaterial` with redacted `Debug`, plus one AccountActor-held copy for the optional save.
   It may serialize only into the live DTO snapshot the WebView renders, and is
   copied only through the platform clipboard on an explicit Copy action. It must
   never enter diagnostics, logs, QA tokens, window titles, screenshots,
   fixtures, persisted state, issue comments, or any command sent back from the
   WebView. Confirmation, logout, account switch, and session teardown drop both
   Rust-owned copies, which zeroize on drop. Copies outside Rust ownership (the
   serialized DTO/IPC string, the WebView's JS string, any `StateDelta` still
   buffered in the event broadcast ring) are released, not zeroized; a copied
   key stays in the OS clipboard until overwritten. The optional save writes the
   held copy to a user-selected native destination and reports only a coarse
   `Written`/`WriteFailed` status. Only keys returned by `enable()`,
   `reset_key()`, or `recover_and_reset()` are recovery keys; the fork's
   `backups().local_recovery_key()` backup decryption key must never be
   revealed, saved, or labelled as one. The verification-gate identity
   bootstrap (#1049) uses the same reveal slot keyed by its flow id; the widely
   observed `SessionState::AwaitingBootstrapConfirmation` (QA window titles,
   gate diagnostics, gate tests) carries only coarse correlation and never the
   key.
   Degraded Secure Backup health does not stop sync, sending, receiving, local
   decryption, diagnostics, or logout. Backup-health diagnostics never carry
   identifiers, backup versions, key material, message content, filesystem
   paths, or raw failures.
   The stock outbound Megolm path (overview "Initial outbound Megolm delivery")
   is also binding on test-only SDK code: no current-generation fence, repeated
   pre-share, fixed-delay re-share, share-index-0/resend-index-0 API, manual
   encryption-debug share, repair scheduler, or original-recipient ledger may
   exist there either. Read-only Megolm diagnostics may expose aliases, closed
   states, count/time buckets, and unchanged-index facts — not identifiers,
   keys, sync positions, request data, content, or raw errors.
   Session admission (overview "Runtime Model"): authoritative `Unknown` trust
   must not be labelled Unverified. Rejection and logout erase local keyed
   stores and attempt server logout before projecting `SignedOut`.
   The `device_cleanup` diagnostic source may record only request correlation,
   elapsed milliseconds, booleans, and the enum fields `stage`, `reason`,
   `auth_mode`, `outcome`, and `failure_kind`. It must never record Device IDs,
   user IDs, homeservers, tokens, UIAA sessions, passwords, recovery material,
   or raw SDK errors. The remaining cleanup contract is in state ownership
   "Device-to-device verification and device cleanup".
12. Credential-store health diagnostics are kind-only. Public state may report
   only `unknown`, `healthy`, `unavailable`, `locked_or_inaccessible`,
   `missing_credential`, or `reset_required`, with optional private-data-free
   remediation hints. Raw OS/keyring errors, local paths, account identifiers,
   key labels, local unlock secrets, SDK/search keys, and recovery material must
   stay inside `StoreActor`/adapter diagnostics gated for debug/test. Ownership
   of probes and `reset_local_data` is in state ownership "Credential health";
   the macOS Tier 2 lane is in [QA lanes](../agents/qa-lanes.md#credential-health-tiers).
13. Media/file diagnostics are metadata-minimized. Normal `Debug`, QA logs,
   errors, window-title tokens, and docs examples must not expose filenames,
   captions, bytes, MXC URIs from real accounts, encrypted media keys/hashes,
   room IDs, event IDs, or raw SDK errors. Download effects emit byte counts or
   app-owned handles only. Caption and staged-byte ownership is in state
   machines and overview.
14. Profile/avatar diagnostics are metadata-minimized. Normal `Debug`, QA logs,
   errors, window-title tokens, and issue evidence must not expose real display
   names, avatar MXC URIs, avatar bytes, local thumbnail paths, encrypted media
   keys/hashes, or raw SDK errors. Avatar MXC URIs, thumbnail state, and
   user-room avatar associations are account-scoped sensitive metadata; real
   values are redacted in Debug, logs, tests, QA tokens, fixtures, and issue
   evidence, including read-receipt reader names and avatars. Rust-owned
   people-facing label fields are optional; when one is absent React renders
   the localized unknown-user label instead of promoting the raw Matrix user id.
   Alias dialogs discard the unsaved draft on dismissal or target change, and
   browser-headless coverage for alias UI asserts both the typed command
   arguments and the Rust-projected `display_label` / `original_display_label`
   rendering. Avatar cache, alias, and label-resolution ownership is in
   overview and state ownership "Profiles and local aliases".
15. Message-action diagnostics are metadata-minimized: normal `Debug`, QA logs,
   errors, window-title tokens, and issue evidence must not expose real Matrix
   room IDs, event IDs, generated permalinks, message bodies, sender IDs,
   transaction IDs, or raw SDK errors. Action eligibility and source/forward
   ownership are in state ownership "Message interactions".

## Logging and Diagnostics

1. Diagnostics are structured and redacted
   (`core.sync.failed kind=http` style). Structured fields are enums/kinds;
   free-form string fields are prohibited because they eventually carry
   content.
2. Raw SDK errors may be printed only behind an explicit debug/test
   diagnostic switch. They must never reach `AppState`, committed logs,
   normal test fixtures, or release diagnostics.
3. Public boundary types (`koushi-protocol` commands/events/identities/failures/
   state updates, snapshot DTOs, Tauri DTO mirrors, and shared QA payloads) must
   treat `Debug` as artifact-facing. Use derived `Debug` only when every field
   is safe if copied into CI logs or a GitHub issue. Use custom redacted `Debug`
   for any field that may contain message bodies, search snippets/queries,
   attachment names, room/user/event/transaction IDs, local filesystem paths,
   URLs from rooms, raw SDK errors, or secrets. Redacted `Debug` may expose
   variant names, request ids, enum kinds, booleans, counts, lengths, and
   placeholders such as `Snippet(..)` or `RoomId(..)`.
4. QA asserts on `CoreEvent` and `AppStateSnapshot`, never on log output.
5. Real-account and real-homeserver QA output is tokenized before it becomes an
   artifact. Captured logs must not contain raw Matrix IDs, room IDs, event IDs,
   transaction IDs, user IDs, message bodies, search queries, local paths, or raw
   SDK errors. Failure output may name the failed step and a coarse failure
   kind. Producers should avoid formatting those values; wrappers must not
   write unredacted stdout/stderr and only then discover a leak.

## Async and Runtime

The general async design rules are overview "Async Design Rules"; the items
below are the policy those rules do not state.

1. No fixed sleeps in QA or product code waiting for Matrix effects — wait
   on events with timeouts. A multi-event waiter owns one monotonic absolute
   deadline for the whole operation and passes that same deadline through
   every loop iteration and nested phase. Never recreate a relative timeout
   around each received event: an unrelated continuous sync stream would then
   postpone failure forever. A waiter that combines an authoritative snapshot
   with an event stream treats the stream only as a wake signal: check the
   predicate before blocking and perform one final authoritative observation
   after timeout, closure, or lag, under the same deadline, before reporting
   failure. Timeout diagnostics identify the typed phase and private-data-free
   observed state needed to locate the missing transition.
2. Every spawned background task and live stream/subscription handle has one
   retained owner responsible for cancelling it (replacement, unsubscribe,
   account shutdown, app shutdown) and awaiting settlement. Tokio
   `JoinHandle::drop` detaches the task and is never an orderly teardown;
   actor handles retain their task, explicit shutdown is a cancel-and-await
   barrier, and unexpected owner drop aborts as a fail-safe. Partial-start and
   replacement error paths settle every child already created. No unbounded maps
   of owned tasks or live handles. The uncapped account-session room-ID intent
   set is bounded by rooms observed in that session and owns no per-room
   task/stream; it is allowed to preserve Sliding Sync subscription residency
   until explicit leave or session teardown. Browser listeners, observers,
   frames, and timers are permitted only for mounted presentation lifetime and
   are cancelled by the same React effect/controller on key change and unmount;
   product retries, backoff, correlation, and session cleanup remain Rust-owned.

   **Known deviation, deliberately unfixed.** `TimelineManagerActor::timelines`
   (`crates/koushi-core/src/timeline/manager.rs:493`) *is* an unbounded map of owned
   live handles: a Room key is never unsubscribed, so every room visited in a
   session keeps its timeline actor, its `navigation_items` (the full canonical
   item list, deliberately wider than the 120-item replay window), its media
   caches and its SDK `Timeline` handle. `LiveTailRefreshCoordinator::states` is
   likewise never pruned for a non-delayed room. This is a known violation of the
   sentence above, established structurally, not an oversight.

   It stays unfixed by maintainer decision after four designs failed independent
   pre-implementation review: bounding it needs the read-intent ownership move, and
   the actor-owned child tasks have no approved cancel-and-await teardown boundary
   either. The evidence, the rejected designs, the nine requirements and the eight
   open specification items are in
   `docs/plans/2026-10-09-issue1230-cold-room-retirement.md` and
   `docs/plans/2026-10-09-issue1230-read-intent-ownership.md` (both merged). Reopen
   that work only with a measurement showing the retained bytes matter; expect the
   read-receipt state machine to be the blast radius.
3. Timeline scrollback and gap repair are event-driven; the contract is
   overview "Timeline Viewport And Scrollback" and "Room Timeline Gap Repair".
   Do not add polling, fixed-delay retries, or a user-scroll latch to
   compensate for a missing transition. Room-entry live-edge repair must not
   reuse a baseline observation for an empty response. A stale descriptor
   permits one authoritative re-inspection, then closes and clears that
   checkpoint so a later committed response can be admitted. While that bounded
   attempt is unsettled, retain the latest newer checkpoint separately and
   promote it immediately after close/admission; delivery is at-most-once and
   another room update may never arrive to replay it. SyncService response
   identity combines subscription generation with the room event-cache
   observation sequence; one subscription generation spans many responses. The
   SDK assigns the process-local response sequence and publishes room updates in
   one critical section. Underfilled-initial pagination uses the settled height
   model and virtual range; a transient virtual DOM `scrollHeight` is not proof
   that a timeline with canonical overflow needs another page.
4. State-critical actor actions are reliable messages, not lossy hints. Do not
   ignore failed reducer-action sends for transitions that set or clear pending
   user-visible state. Await the send, retry through the owner, or emit a
   correlated operation failure that leaves no stuck pending state. Once a
   command accepts a user-intent submission, terminal observation and
   request/submission correlation must be owned by a component whose lifetime
   spans every replaceable presentation actor involved in that operation. An
   unsubscribe, room switch, actor crash, or resubscribe must not discard the
   accepted operation. Lossy observer lag becomes an immediate, correlated,
   private-data-safe failure rather than a fixed-delay wait. Shutdown drain
   order, owner-polled futures, and media enqueue ordering are specified in
   overview and state machines; per-worker graceful shutdown timeouts are
   forbidden because they make shutdown latency scale with worker count.
5. If a reducer returns an `AppEffect` that matters in production, the
   production runtime executes it or the behavior is redesigned as an explicit
   `CoreCommand`/actor command. Discarding such effects is allowed only for
   fixture/demo effects that are documented as non-production.
6. Core async channels are sized for large-account (100+ room) sync bursts via
   the named capacity constants in overview "Async Design Rules", never small
   magic literals. Delivery discipline, including when `try_send` is permitted,
   is owned by overview "Async Design Rules" (delivery discipline by payload
   type). Background workers that consume authoritative latest snapshots, such
   as the search history crawler's joined-room availability notification, must
   not block user-visible actor commands. Purely local navigation must not wait
   for a network-operation mailbox (overview "Async Design Rules"). Work that an
   action-batch commit dispatches to the AccountActor must not hold the AppActor
   loop either: generation-guarded or latest-wins dispatches (Activity
   resolution and its cancel, a Space-children reload after a live leave, and
   the search-crawler lane of rebuild, cache invalidation, and room
   availability) try the mailbox once and otherwise keep one deferred value per
   kind, which the AppActor loop delivers when a slot frees; a newer value
   replaces a deferred one. Every dispatch of such a kind goes through that
   path, command-originated ones included, so a held value is never delivered
   after a newer one; the crawler lane stays ordered, so a stale notification
   can never undo a caption or filename opt-out. Still awaiting admission:
   session-lifecycle effects (sync start/stop, trust and backup checks, session
   status), the settings-policy broadcasts that follow a settings change
   (read-receipt, display, and link-preview policy), and user commands routed to
   the AccountActor.
7. User-intent commands resolve to a correlated, observable terminal outcome —
   never a silent no-op. A foreground one-shot command
   (account restore/login/logout, `SelectRoom`/`SelectSpace`, send/edit/redact,
   pin/unpin, mark-read, invite accept/decline, start DM, join/leave) carries
   its `request_id` end to end and settles as exactly one of `committed`,
   `benign-noop(reason)`, or `failed-noop(reason)`. A reducer that returns
   `Vec::new()` for such a command (room absent from `state.rooms`, session not
   ready) MUST surface that as a correlated outcome, and the command waiter
   returns the specific reason, never a generic "did not complete" timeout. On
   the submit → route → project → reduce → settle path, discarding a failure
   with `let _ =`, `unwrap_or_default()`, `.ok()`, or a catch-all `_ => {}` is
   forbidden. Reducer projection is also an admission boundary: if the projected
   action is rejected in the current state, emit exactly one correlated typed
   failure and do not route the command. Separate operation-event and snapshot
   lanes may arrive in either order; a follow-up that depends on authoritative
   state must observe both its terminal and the required snapshot state before
   submitting.
8. Telemetry and diagnostics travel on a dedicated lane, not the product-state
   channel. Lifecycle/diagnostic events such as `CoreEvent::IntentLifecycle` are
   never folded into product `StateDelta`/`StateChanged`, never drive product
   state in the WebView, and never cause a user-intent to be dropped when a
   telemetry buffer saturates. A UI must not infer product success from a
   telemetry event; success comes from the projected snapshot.
9. Each submitted user-intent resolves exactly once, in submission order.
   Request-to-intent correlation state is drained per submission, so concurrent
   intents for the same target each receive their own terminal outcome. A
   single-value, target-keyed correlation map that overwrites an in-flight
   `request_id` — leaving the older request unsettled until its timeout — is
   forbidden; use a per-target FIFO queue or carry the `request_id` on the
   projected action.
10. Every user-intent command class ships a real-account-shaped scale stress
   test (about 110 rooms / 5 spaces / 57 DMs) that drives the real command path
   and asserts the lifecycle invariant — every submission reaches a terminal
   outcome, none vanishes. The runtime and reducer are exonerated for a scale
   bug only by such a test, and a transient room-list projection that drops a
   known-joined room is caught here, not in CI.
11. Verified-session sync ownership, verification transport, and key-query
   claims are specified in overview "Runtime Model" (sync ownership), overview
   "Security Model" (verification transport and key-query claims), and state
   machines. A QA device-readiness checkpoint must not expose its target in
   diagnostics. In
   addition: the pending to-device verification owner must have tests for
   missing, expired, duplicate, and capacity cases; its owner lock must not be
   nested with the request cache or held across async/fallible work; do not
   extend an existing public exhaustive SDK enum to carry internal dispatch
   state; and never infer sync-cursor validity from a server family, an empty
   room list, elapsed time, or a successful crypto gate. Multi-stage QA
   participant ownership is in
   [QA lanes](../agents/qa-lanes.md#multi-stage-qa-participant-ownership).
12. A bounded materialized view's diff stream is not proof that every source
   domain represented beside that view is unchanged. If authoritative state can
   commit outside the bounded window, the owning observer must consume a
   post-commit source signal as a wake-up and reconcile only the affected
   fingerprint. Coalesce queued wakes, perform one bounded reconciliation after
   lag, and keep auxiliary-channel closure from killing the authoritative
   observer. The auxiliary signal must not create a second network/sync owner,
   and high-frequency unrelated updates must be proven not to trigger full-view
   normalization.

## GUI Automation

GUI automation is a thin smoke layer, never the primary correctness gate.

1. UI behavior is verified headless by default: frontend tests run in a
   headless browser with mocked Tauri IPC and fake `CoreEvent` streams
   (QA Model layer 4). The canonical headless DOM gate is
   `npm --prefix apps/desktop run test:ui-headless` using Playwright against
   the Vite harness. The real Tauri app is launched only for the minimal
   native-integration smoke, and on macOS only attended — unattended agent
   sessions must not launch the GUI app (it opens real windows, reads the
   OS keychain, and surfaces crash dialogs on the user's desktop).
   Real-Tauri GUI automation by agents is allowed only under a virtual
   display (Linux Xvfb + `tauri-driver`; not available on macOS).
2. Browser tests assert the typed command first, prove command acceptance
   alone does not repair the view, then inject the Rust-owned result. Bugs and
   product-spec changes discovered during interactive browser/GUI exploration
   must be pinned by headless coverage whenever feasible before the work is
   considered complete. Prefer unit/component tests for pure state or
   rendering contracts, and browser-headless tests for user-visible DOM,
   scrolling, context-menu, drag/drop, and mocked IPC behavior. Native GUI lanes
   may remain final smoke evidence, but they do not replace a cheap headless
   regression test unless the behavior is inherently OS-window, keychain, menu,
   notification, or WebView-native.
3. Per-feature GUI boundaries are in state ownership. Test obligations that
   owner does not state: room-list section tests must prove tag-driven movement
   from Rust-shaped `RoomSummary.tags` snapshots, and shell tests must prove
   section order, counts, unread badges, and mention dots from Rust-shaped
   `SidebarModel` fields. The formatted-message renderer keeps direct list
   element children as `li`. `SettingsValues.display.hide_redacted` defaults to
   `true`. `SettingsValues.media` holds only
   `image_upload_compression_policy`: #305 retired the automatic-compression
   preference, and resize/format is chosen per attachment in the upload-staging
   dialog.
4. Operational GUI-smoke safety (FIFO credential entry, `Cmd+Q`, AppleScript
   process names, Keychain-suppressing environment, `--allow-empty-timeline`)
   is in [QA lanes](../agents/qa-lanes.md#real-account-lanes) and
   [troubleshooting](../agents/troubleshooting.md#macos-gui-smoke).

## Desktop Text Input And IME Safety

1. Text-entry components use `ImeTextField`, `SecureImeTextField`,
   `ImeTextArea`, `ImeOwnedTextArea`, or `ImeInlineMentionEditor` from
   `apps/desktop/src/components/ImeTextControl.tsx`. The inline editor is the
   only approved `contentEditable` owner and represents each semantic mention as
   one non-editable document atom. Forms containing text entry use `ImeSafeForm`.
   Do not duplicate composition handlers in feature components; new text-entry
   variants extend the shared primitive and its behavioral tests. File,
   checkbox, radio, date/time, and other non-text controls remain native.
2. While composing, and while a local edit has not been acknowledged, the DOM
   value and selection are authoritative. Snapshot-driven props may update the
   control only when they acknowledge the same value or when `syncKey` changes
   because the logical entity/field changed. Object identity alone is not a
   synchronization key.
3. `keydown` facts are sampled synchronously. If the composition epoch,
   `nativeEvent.isComposing`, or legacy IME key code identifies candidate
   confirmation, keep the browser default, skip the feature handler, and mark
   the nearest `ImeSafeForm` so its associated implicit submit is suppressed.
   Do not infer composition from a later async callback.
4. Text-changing async commands use a generation-guarded operation queue per
   logical field. The queue serializes active writes, skips superseded pending
   writes before dispatch, applies only the newest completion, and invalidates
   pending work when the field/entity is cleared or replaced. Independent
   fields may run concurrently; an older result must never settle or invalidate
   a newer generation.
5. Password/recovery inputs use `SecureImeTextField` without `value` or
   `defaultValue`. Read and clear them through a DOM ref at explicit submit or
   cancel boundaries. React state may store booleans such as `isFilled`, but
   not the secret string.
6. Behavioral tests cover composition plus a parent rerender, ordinary dirty
   draft plus a stale snapshot, acknowledgement, logical-key reset, selection
   preservation, candidate-confirmation Enter, associated-form submit fencing,
   and ordinary Enter. At least one upload-caption surface and one ordinary
   form/search surface must exercise the shared behavior.
7. `scripts/check-ime-text-inputs.mjs` AST-scans production TSX and rejects raw
   composable `input`, `textarea`, `form`, dynamic input types, and
   `contentEditable` outside the shared primitive. The gate is part of the
   desktop `lint` command; exclusions are limited to tests and the primitive
   implementation itself.

## Build, Dependencies, QA Gates

1. The checked-out `vendor/matrix-rust-sdk` submodule is the authoritative
   Matrix Rust SDK source for this workspace. All root workspace Matrix SDK
   dependencies (`matrix-sdk`, `matrix-sdk-base`, `matrix-sdk-search`,
   `matrix-sdk-test`, and `matrix-sdk-ui`) use their exact paths beneath
   `vendor/matrix-rust-sdk`; root `Cargo.toml` declarations using `git` or
   `rev` are prohibited. The submodule gitlink is the only SDK revision pin.
   Keep the checkout at that gitlink and keep
   `scripts/check-sdk-submodule.mjs` green before compiling or changing SDK
   code.
   Direct ports from Element X code preserve upstream license and copyright
   notices.
   Patches to the vendored SDK are limited to what is indispensable: a
   change is allowed only when the need cannot be met through the SDK's
   public API or a wrapper on our side. Each patch must be minimal
   (prefer additive accessors over behavioral changes), recorded in
   `docs/upstream/matrix-rust-sdk-feedback.md` with rationale and
   upstreaming intent, and reviewed at phase exit. In this repo the actual
   deltas live in the checked-out submodule and are pinned by its gitlink; local
   comments should point at the patch surface.
   Convenience patches are rejected; every patch increases the cost of
   tracking upstream. Every SDK gitlink bump must keep the guarded submodule
   checkout in sync and update the root `Cargo.lock` when dependency resolution
   changes.
2. Local Tuwunel toolchain caveats are tracked in
   [environment](../agents/environment.md) and the QA scripts, not hand-run.
3. Required local gates before merging product changes: the Rust lint gate
   (`cargo fmt --check` and the clippy commands in
   [verification](../agents/verification.md#rust-lint-gate)), the same
   feature-unified workspace test run CI uses (`cargo test --workspace` with
   CI's `--exclude` list from `.github/workflows/ci.yml`), frontend tests +
   typecheck, and `qa:headless-local -- --server=both`. During iteration, use
   focused checks first; this does not waive merge gates. Documentation-only
   changes that do not change executable code, dependencies, or QA runner
   contracts run the affected documentation checks and `git diff --check`;
   product suites are not local prerequisites for those changes. Required CI
   checks still apply.
4. Real homeserver QA is a release/preflight gate (network + approved
   credentials), not an every-CI gate. It runs after the local headless and
   Linux virtual-display lanes are green and cleanup behavior is proven. It is
   also required before GUI-level confidence claims and after changes that
   affect login, recovery, sync, encrypted restore, search, room cleanup, or
   logout.
5. Core crates stay platform-portable as specified in overview "Platform
   Portability". Test-only shared Core integration support belongs in
   non-default `koushi-core-testkit`, never a Core self-dev-dependency or
   production API.
6. Signed distribution builds must run the platform-specific credential gate
   before packaging. A macOS signed-DMG build validates the signing identity
   and all notarization credentials without requiring unrelated Windows
   credentials; post-build signature, notarization-ticket, and platform trust
   checks remain mandatory release evidence.

## Product Text And Localization

Details and mirrors: [i18n](../architecture/i18n.md).

- User-visible product text must not be embedded directly in React components,
  Rust core errors, Tauri commands, or tests that model production UI. Use a
  message catalog keyed by stable IDs, with interpolation for dynamic values.
- English-only product copy is acceptable while the localization system is
  still small, but it still goes through the catalog. Deferring Japanese or
  other locale quality does not permit hardcoded English/Japanese strings in
  product UI, menus, accessibility labels, placeholders, empty states, dialogs,
  or validation messages.
- Core and adapters return machine-readable kinds, codes, and structured
  non-secret data. They do not return English/Japanese prose for the UI to
  display, except for debug/test-only diagnostics.
- Rust-projected identity fields such as `display_label` and
  `original_display_label` are dynamic room/user data, not localized product
  prose. They must resolve from real alias/upstream/profile/MXID/room-id data;
  never use generic hardcoded labels such as `Member` as identity fallbacks.
- The UI boundary is responsible for resolving message IDs to localized text.
  Accessibility labels, button labels, menu labels, empty states, dialogs,
  toasts, and validation messages are user-facing text and use the same
  catalog.
- Locale/display behavior is Rust-owned. GUI code consumes the resolved
  `LocaleDisplayProfile` (`lang`, `dir`, catalog locale, pseudo-locale mode,
  platform, and modifier labels) and must not branch on raw persisted locale
  tags in feature components.
- QA tokens, protocol enum variants, log kinds, CSS class names, data-testid
  values, and synthetic fixture message bodies are not localized. Tests should
  prefer roles, stable test IDs, message IDs, or semantic state over localized
  prose when possible.
- A new feature that adds user-visible text adds or updates catalog entries,
  at least one default locale, and a pseudo-locale or missing-translation test
  before wiring the text into the UI.
- Locale-sensitive layout uses CSS logical properties by default. Unreviewed
  physical left/right spacing, borders, positioning, or text alignment in the
  desktop shell is a defect unless the physical direction is intentional and
  documented.
- CJK text fitting is a presentation contract. GUI code may use CSS line-break,
  word-break, hyphenation, wrapping, and ellipsis rules to fit Rust-owned room
  names, sender/member names, message bodies, thread labels, and snippets, but
  must not rewrite text, recompute sort keys, normalize queries, or repair
  highlights locally.

## Test Placement

- **Integration-style or projection tests belong under `tests/` per feature.**
  Tests that exercise reducer/command/event/runtime projection, DTO snapshots,
  or state-machine transitions are integration-style. Put them in
  `crates/<crate>/tests/<feature>.rs`, not inside a monolithic
  `#[cfg(test)] mod tests` block in a source file.
- **Pure unit tests may stay inline.** Small tests for a single pure helper,
  parser, or private algorithm may remain in the source file under
  `#[cfg(test)] mod tests`. A directly attached inline `cfg(test)` module is a
  large inline test module when it spans at least 200 physical lines, counting
  its attached attributes, declaration, body, and closing brace under the
  repository scanner's lexical definition; this is a hard ceiling, not
  permission for a 199-line integration-style test to remain inline. Private
  tests beyond one screen, tests that assert across modules, and all
  integration-style or public behavior tests move to sibling modules or the
  crate's `tests/` directory even when they are below that ceiling.
- **Source-structure assertions have one owner.** Assertions that read Rust
  source to require or forbid structure belong to the repository Rust test
  structure checker, not to Rust behavioral tests. Behavioral tests must drive
  callable behavior and assert its outcome; they must not inspect source text.
- **Do not add new tests to existing monolithic test files.** Add a new
  `tests/<feature>.rs` file instead. Current crate integration tests, including
  `crates/koushi-core/tests/`, are the placement example; existing monolithic
  files may be split opportunistically when they are touched for a new feature.
- **Tests never touch the real profile.** Runtime tests start through
  `CoreRuntime::start_isolated` (or `restart_isolated` for persistence), which
  owns temporary data and file-credential directories; there is no
  constructor that defaults to the user data directory. Under `test-hooks`,
  test/QA store constructors panic on the real Koushi profile, and unit tests
  ignore `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR`.
- **Test fixtures and fakes belong near their consumer.** A fake used by a
  single feature's tests lives in that feature's test module. Shared fakes live
  in `src/test_support.rs` or `tests/support/` and must be append-friendly.

## Concurrent Work Details

### Shared hot files

The main agent owns integration of the following shared surfaces. Subagents may
read them but must not append to them without main-agent coordination:

- `crates/koushi-state/src/{state.rs,action.rs,reducer.rs}`
- `crates/koushi-protocol/src/{command.rs,command/,event.rs,event/,state_update.rs}`
- `crates/koushi-core/src/runtime.rs`
- `apps/desktop/src-tauri/src/{dto.rs,dto/,commands/}`
- `apps/desktop/src/{App.tsx,components/TimelineView.tsx,i18n/messages.ts,styles.css}`
- `apps/desktop/src/domain/{types.ts,coreEvents.ts,coreEvents.generated.json}`
- Browser-headless GUI-operation specs, Tauri IPC mocks, and Linux GUI QA scripts

To reduce conflicts on these files:

- **Group related fields into nested structs/DTOs.** Instead of adding several
  top-level fields to `AppState` for one feature, add one nested struct
  (e.g., `AppState.account_management`, `AppState.room_list`). This confines
  most feature-specific diffs to the nested type and its mirror on the
  TypeScript side.
- **Avoid central re-export lists that every feature edits.** When a crate's
  `lib.rs` becomes a long list of per-feature re-exports, prefer re-exporting
  the feature module namespace (`pub mod account_management;`) or a
  feature-grouped prelude. Each feature then edits its own module's public API
  rather than a shared list.
- **Keep generated contract artifacts append-friendly.** Additions to
  `coreEvents.generated.json` go at the end of the relevant array/object
  without renumbering or reformatting unrelated entries. Do not regenerate the
  artifact with unrelated formatting churn in the same change.

### Parallel implementation

- **Serialize shared surface design before parallelizing implementation.** The
  main agent must decide module boundaries, enum variants, nested DTO shapes,
  and test file names before subagents begin coding. Subagents receive a bounded
  file allow-list and a shared-file deny-list in their prompt.
- **Do not parallelize two agents on the same hot file.** Cap concurrent
  subagents to disjoint territories (typically 2-3). If two features both need
  to change the same hot file, either split the work sequentially or have the
  main agent pre-apply the shared scaffold and let subagents fill module-local
  bodies.
- **Subagent output is a draft to integrate, not merged evidence.** Cheap
  implementation agents may write module-local code, tests, and docs. The main
  agent still integrates shared enums, reducers, command/event variants, Tauri
  DTOs, TypeScript wire, generated contract artifacts, and issue comments.
- **Merge integration branches before landing on `main`.** When multiple feature
  branches run in parallel, create a short-lived integration worktree, resolve
  conflicts and run the full gate there, then fast-forward `main`. Do not push
  a feature branch directly to `main` while another parallel feature is still
  open.

### Worktree cleanup

- **Remove temporary worktrees as soon as they are no longer needed.** A feature
  branch that has been merged into `main` or into an integration branch should
  not keep a worktree alive. Use `git worktree remove --force <path>` when the
  worktree contains submodules.
- **Delete worktree-local build intermediates before or during worktree removal.**
  Worktrees often accumulate large per-worktree artifacts:
  - Rust build artifacts under the worktree's `target/` directory when
    `CARGO_TARGET_DIR` is not shared or when the worktree overrides it.
  - Vite/Tauri dev caches under `apps/desktop/node_modules/.vite/`.
  - Per-worktree `node_modules/` when the worktree does not share the main
    workspace's dependency tree.
  These must be cleaned up because they can consume many gigabytes and are not
  needed after the branch is merged.
- **Do not delete shared build directories.** When `CARGO_TARGET_DIR` points to a
  shared location (e.g., the main workspace `target/`), confirm that the
  directory is shared across worktrees before deleting it. Shared target
  directories speed up rebuilds and must be preserved.
- **Verify cleanup.** After removing a worktree, confirm `git worktree list` and
  disk usage look reasonable. Large leftover artifacts are a hygiene defect.

## UI Presentation

### Property Display And Editing

- One property has one primary place. The current value of a setting or
  editable property, the control that changes or clears it, and the result of
  that change (saving, saved, failed, retry) live in the same row or card. Do
  not show a read-only "current X" in one section and an "X settings" form in
  another, and do not place unrelated actions (downloads, diagnostics,
  repair) between a property group's display and its editing.
- The same place covers every state: unset (offer to set it there), read-only
  (show the value with the reason it cannot be changed), in flight, and
  failed. A failure is never shown as success, and an unsaved draft stays
  distinguishable from the confirmed value.
- Values that mean different things stay separate even when they look alike:
  a shared Matrix value and a device-local override are two properties, each
  with its own card; merging display locations must not merge meaning or
  storage.
- Anything that looks like a setting leads somewhere. A menu entry or summary
  badge for a property on the same surface moves to that property, and its
  label matches its destination. An unimplemented feature is not presented as
  a disabled or dead-end setting.
- Summaries, aggregates, and non-editable status (header titles, member or
  unread counts, encryption state) may stay separate; explain the purpose of
  a separate display, and make a summary of a setting lead to it. Unifying
  display never removes a required confirmation or permission check.
- New or changed UI must not introduce a violation. An audit finding names
  the property, the display location, the edit location, and the effect, and
  is resolved by a fix or a recorded, reasoned exception. Existing violations
  in unrelated surfaces need not be fixed in the same change.

### macOS Native Window Controls And Overlay Layout

- Preserve the standard macOS close, minimize, and fullscreen buttons. Reserve
  a safe area above application content instead of hiding, replacing, or
  attempting to cover the native buttons to resolve an overlap.
- All in-app dialogs, confirmations, media viewers, and floating popovers must
  keep their content and interactive controls below the native-button area.
  This includes nested dialogs and surfaces shown before sign-in or during
  verification and recovery. A backdrop may cover the window, but the browser
  top layer and CSS z-index must not be treated as covering native controls.
- Shared layout primitives own the safe-area boundary and available viewport
  height. New surfaces must inherit that contract rather than add independent
  offsets. Platform detection must not depend on whether the signed-in shell's
  titlebar DOM is mounted; nested content must not add the same inset twice.
- Keep content and close, cancel, and confirm actions reachable after resizing
  or zooming, including short windows, display-scale changes, and fullscreen
  transitions. Verify computed placement and scrolling with headless tests;
  confirm native-button clearance separately on macOS. Do not impose the
  macOS inset on Windows or Linux.

### Right Panel And Composer Containment

- Every right-panel mode (thread, search, focused context, room info, people,
  profile, Space info and members, files, pinned, threads list, recovery)
  renders inside the one shared panel shell. A mode must not introduce its own
  outer container, width, or positioning.
- Layout tracks inside the shell and the composer shrink to the allocated
  inline size: grid columns are explicit `minmax(0, 1fr)` tracks (never an
  implicit `auto` column) and grid or flex children that may hold long text set
  `min-inline-size: 0`. A descendant's intrinsic width (a long Matrix ID, an
  unbroken URL, expanded localized text) must never widen the panel's or the
  composer's content past its own edge.
- The panel header with its close action, and the composer toolbar and footer
  with the send action, stay inside the panel at every supported width,
  density, and locale. When a toolbar row cannot fit its fixed controls, it
  wraps; it does not overflow or hide controls.
- A long account identity truncates with an ellipsis in place; the full value
  stays available through `title` and the accessible name.
- Fix containment at the shared shell and composer primitives. Do not hide
  overflow globally (on the shell, the app grid, or `body`) to mask an
  escaping child, and do not tune media-query breakpoints to dodge a width
  that overflows.
- Rust/Tauri keeps ownership of native window geometry (minimum window size,
  zoom, display scale); CSS containment must hold for any viewport the native
  layer allows.
- Headless regressions measure the panel and descendant rectangles, hit-test
  the close and send actions with `elementFromPoint`, and click them; a
  `toBeVisible()` check or a screenshot alone is not containment evidence.

## Search Index And Room-Key Export

- Manual room-key file export/import MUST use the Matrix key-export file
  format that Element clients use, including the encrypted Megolm session data
  header/footer handled by the public Matrix Rust SDK APIs. Do not introduce a
  product-specific JSON, archive, or wrapper file format for room-key transfer.
  Tests for this flow must use synthetic fixtures and assert interoperability
  without logging or snapshotting room-key file contents.
  If the public SDK export API does not return an exported-session count,
  reducer/DTO state must represent that count as unknown instead of decrypting,
  parsing, or re-wrapping the export file only to derive UI metadata.
- Ngram terms, token dictionaries, postings, highlight spans, and attachment
  filename matches are plaintext-derived data. Treat them with the same
  confidentiality as the original message text.
- Persistent local search for E2EE rooms MUST use an encrypted
  `matrix-sdk-search` index. Unencrypted search indexes are forbidden for E2EE
  content.
- The search index MUST NOT be the display source of truth. It may produce
  candidate event IDs only; snippets and highlights must be generated from the
  resolved visible event content loaded from the SDK store or network.
- Search result highlights MUST be exact, second-pass verified spans. Ngram
  candidates without a verified visible span must be dropped or shown only by
  an explicitly non-exact result mode.
- Edits, replacements, and redactions MUST be resolved before indexing or
  returning a result. An edit event downloaded before its target event MUST be
  stored as a pending relation, not indexed as an independent message.
- Redacted events and redacted attachments MUST be removed from the search
  index. File contents are out of scope until a separate security design is
  approved.
- Attachment filenames are searchable but confidential. They MUST follow the
  same encrypted-index, verified-highlight, and no-logging rules as message
  bodies.

## Crate Ownership And Projection Authority

- An index-based `VectorDiff` accumulator is advanced only by diffs from the
  stream that owns it, never replaced from an independently collected snapshot
  (such as `current_entries_snapshot()`, which is not the filtered, sorted, and
  paged order the indices refer to). Such a snapshot may supply one-shot
  inspection or scalar metadata only; a fresh ordered checkpoint comes from the
  adapter that emits the diffs.
- A projection derived from such an accumulator may claim authority only when
  its entry count equals its distinct-identity count; length equality alone never
  establishes authority, and a failing projection preserves the previously known
  state rather than publishing a lossy one.
- `koushi-sdk` is the low-level Matrix SDK adapter crate. It owns
  SDK-facing primitives only and may include feature-gated, direct-adapter
  smoke binaries and private-data-free smoke reports for adapter integration;
  product state, actor lifecycle, and product opinions stay in `koushi-core`
  and `koushi-state`.
- `koushi-store` owns native credential/vault and shared encrypted-file
  mechanics. It has no Matrix SDK, Tauri, async-runtime, Core, QA, or concrete
  OS-keyring dependency. Core's `StoreActor` remains the sole account/path/key,
  migration, generation-fence, SDK-store-configuration and coarse-failure policy
  owner; extracting persistence must never make the store fail open.
- `koushi-search` and `koushi-media` own only pure search/image algorithms.
  Actor/cache lifecycle, SDK queries, diagnostics, state projection and platform
  delivery remain in Core/adapters rather than moving to a leaf to reduce LOC.
- `koushi-qa` owns authoritative headless and real-homeserver product QA
  binaries/orchestration. It consumes `koushi-protocol` DTOs and narrow
  `koushi-core` test hooks, is not a default production package, and must not
  become a second product runtime or move QA-only channels into the public
  protocol.
- `koushi-core-testkit` is non-default, publish-disabled and test-only. It may
  enable Core `test-hooks` for shared integration targets, but production and QA
  code must not depend on it and private actor APIs must not be widened merely
  to make a fixture reusable.
