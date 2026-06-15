# Agent Notes

This is the operational entry file for agents and QA automation in this local
environment. It records setup commands, troubleshooting, and environment
footguns. Durable repository rules do not live here.

## Read Order

1. [REPOSITORY_RULES.md](REPOSITORY_RULES.md) - root durable rules for this
   repository.
2. [docs/architecture/overview.md](docs/architecture/overview.md) - long-term
   architecture, layer ownership, runtime, security, and QA model.
3. [docs/architecture/state-machine.md](docs/architecture/state-machine.md) -
   normative reducer state-machine diagrams and guard notes.
4. [docs/architecture/i18n.md](docs/architecture/i18n.md) - Rust-owned
   locale/display profile, catalog, pseudo-locale, RTL, and i18n gates.
5. [docs/policies/engineering-rules.md](docs/policies/engineering-rules.md) -
   detailed policy extension for secrets, logging, QA automation, and gates.
6. The relevant dated implementation plan under `docs/superpowers/plans/`.

When an operational note here hardens into a durable rule, promote it to
`REPOSITORY_RULES.md` or `docs/policies/engineering-rules.md` and keep only the
local how-to detail here.

## Current Implementation Plans

All agents implementing the headless core runtime follow
[docs/superpowers/plans/2026-06-12-headless-core-runtime-implementation.md](docs/superpowers/plans/2026-06-12-headless-core-runtime-implementation.md).
All agents implementing the Phase 10+ product surface and release roadmap
follow
[docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md](docs/superpowers/plans/2026-06-13-roadmap-phases-10-18.md).
All agents implementing local GUI room/space/reply operations follow
[docs/superpowers/plans/2026-06-13-local-gui-basic-operations.md](docs/superpowers/plans/2026-06-13-local-gui-basic-operations.md).
All agents planning or implementing the remaining umbrella #12 work follow the
Core Batch A / GUI Batch B split in
[docs/superpowers/specs/2026-06-15-remaining-core-phase-a-batch-design.md](docs/superpowers/specs/2026-06-15-remaining-core-phase-a-batch-design.md):
batch Rust-owned Phase A contracts first, then serialize the shared GUI
surface, then run the #9/#31 integration gate.
The umbrella #12 implementation batch follows
[docs/superpowers/plans/2026-06-15-remaining-core-phase-a-batch-implementation.md](docs/superpowers/plans/2026-06-15-remaining-core-phase-a-batch-implementation.md)
after that plan is approved.
All agents implementing media/file timeline support follow
[docs/superpowers/plans/2026-06-15-media-phase-a.md](docs/superpowers/plans/2026-06-15-media-phase-a.md)
for Phase A Rust/headless work before Phase B GUI wiring.
All agents implementing read receipts, read markers, typing, and presence
follow
[docs/superpowers/plans/2026-06-15-live-signals-phase-a.md](docs/superpowers/plans/2026-06-15-live-signals-phase-a.md)
for Phase A Rust/headless work before Phase B GUI wiring.
Phase B GUI/browser-headless work for the same issue follows
[docs/superpowers/plans/2026-06-15-live-signals-phase-b-gui.md](docs/superpowers/plans/2026-06-15-live-signals-phase-b-gui.md).
All agents implementing E2EE trust Phase A state-machine contracts follow
[docs/superpowers/plans/2026-06-14-e2ee-trust-phase-a.md](docs/superpowers/plans/2026-06-14-e2ee-trust-phase-a.md).
All agents implementing Rust-owned settings Phase A follow
[docs/superpowers/plans/2026-06-14-rust-owned-settings-phase-a.md](docs/superpowers/plans/2026-06-14-rust-owned-settings-phase-a.md).
All agents implementing the headless i18n substrate follow
[docs/superpowers/plans/2026-06-14-i18n-substrate-phase-a.md](docs/superpowers/plans/2026-06-14-i18n-substrate-phase-a.md).
All agents implementing the i18n GUI wiring follow
[docs/superpowers/plans/2026-06-14-i18n-substrate-phase-b.md](docs/superpowers/plans/2026-06-14-i18n-substrate-phase-b.md).
All agents implementing cross-platform font/emoji substrate Phase A follow
[docs/superpowers/plans/2026-06-15-font-emoji-phase-a.md](docs/superpowers/plans/2026-06-15-font-emoji-phase-a.md)
before any Phase B font asset or CSS wiring.
Phase B GUI/browser-headless work for the same issue follows
[docs/superpowers/plans/2026-06-15-font-emoji-phase-b-gui.md](docs/superpowers/plans/2026-06-15-font-emoji-phase-b-gui.md).

## Core Batch A DTO Mirrors

- When `AppState` gains a Core Batch A field, update the hand-maintained Tauri
  `FrontendAppState` DTO, TypeScript `AppState`, browser fake snapshots, app
  harness snapshots, and Tauri IPC mock snapshots in the same change. The real
  WebView consumes the Tauri DTO, while headless tests often consume the
  TypeScript fakes; updating only one side can leave a green browser tier and a
  crashing Tauri lane.
- Focused checks for the shared skeleton are
  `cargo test -p matrix-desktop-state --test core_batch_a_state`,
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact`,
  and `npm --prefix apps/desktop run typecheck`.

## User Profiles Phase Notes

- Own-profile state, per-user profile cache, room avatars, and space avatars
  are Rust-owned DTOs. React renders them and dispatches `set_display_name` /
  `set_avatar`; do not add React-local profile success/failure semantics.
- `SetAvatar` may carry image bytes only through the typed command boundary.
  Debug output, QA logs, screenshots, issue comments, and docs examples must not
  contain real avatar bytes, real avatar MXC URIs, local thumbnail paths, or raw
  SDK errors.
- `AvatarImage.mxc_uri` is metadata, not a render URL. GUI code renders an
  `<img>` only for `AvatarThumbnailState::Ready.source_url`; otherwise it uses
  the colored-initial fallback. This keeps the current #15 media contract intact
  because timeline `download_media` emits byte counts only.
- When adding or changing `AppState.profile` or avatar fields, update the
  hand-maintained Tauri DTO (`apps/desktop/src-tauri/src/dto.rs`), TypeScript
  domain types, browser fake API, Tauri IPC mock, app harness snapshots, and DTO
  serialization-contract tests in the same change. Browser fakes do not inherit
  Rust snapshot fields automatically.
- Profile update completion settles a user-visible pending state. Actor code
  must deliver `ProfileUpdateSucceeded` / `ProfileUpdateFailed` reliably via
  the action channel, not as a best-effort notification that can leave settings
  controls stuck in a saving state.

## Room Tags Phase A Notes

- `RoomSummary.tags` is the Rust-owned source of truth for Matrix `m.tag`
  favourite and low-priority state. React may render tag affordances and dispatch
  `set_room_tag` / `remove_room_tag`, but it must not keep local tag membership
  or repair room-list sections after the fact.
- Favourite and low-priority are mutually exclusive in
  `matrix-desktop-state`. Keep this reducer rule in sync with the SDK wrappers:
  use `matrix-desktop-sdk`'s `set_room_tag` / `remove_room_tag`, which delegate
  to `Room::set_is_favourite` and `Room::set_is_low_priority`; do not patch the
  vendored SDK for this behavior.
- Tag command success must not immediately request a room-list refresh. The SDK
  tag calls send account-data changes to the homeserver, and the local SDK room
  snapshot can remain stale until the next sync. Project the successful command
  through `RoomTagSet` / `RoomTagRemoved` reducer actions, then let the next
  sync snapshot become canonical.
- When adding fields to `RoomSummary`, update every projection and fake snapshot
  in the same change: `matrix-desktop-core::room::normalize_rooms`,
  `matrix-desktop-state::sidebar::RoomListItem`, `apps/desktop/src-tauri/src/dto.rs`,
  `apps/desktop/src/domain/types.ts`, `browserFakeApi.ts`,
  `appHarnessMain.tsx`, and any Rust/TS fixtures that construct `RoomSummary`.
- New tag command/event variants must keep all three IPC surfaces in sync:
  `serialize_core_event`, `apps/desktop/src/domain/coreEvents.ts`, and
  `apps/desktop/src/domain/coreEvents.generated.json`. Verify with
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact`.
- Phase B room-list sections (Favourites / People / Rooms / Low priority) must
  derive from Rust snapshots (`RoomSummary.tags` + `is_dm`). Do not introduce
  React-only section membership while wiring context menus or browser-headless
  tests.

## Outbound Send Queue Notes

- Retry/cancel is driven by SDK `SendHandle`, not by a direct
  `RoomSendQueue::retry(transaction_id)` API. `TimelineActor` must keep a
  transaction-id keyed handle registry initialized from
  `RoomSendQueue::subscribe()` local echoes and updated by
  `RoomSendQueueUpdate::NewLocalEvent`.
- Recoverable SDK send errors disable the room send queue. `RetrySend` must
  call `room.send_queue().set_enabled(true)` before `SendHandle::unwedge()`;
  successful `CancelSend` must also re-enable the room queue after
  `SendHandle::abort()` so successors are not stranded behind a removed failed
  item.
- `TimelineItem.send_state` is a Rust-owned DTO projection. React may render it
  and dispatch `retry_send` / `cancel_send`, but must not infer send legality
  from `TimelineItemId::Transaction` or repair queue state locally.
- `TimelineItemId::Transaction` is a stable identity for local echoes, not a
  UI state. A transaction row without `send_state` must not be labeled unsent;
  failed/sending/cancelled affordances come only from `send_state`.
- Phase B send-queue GUI tests should seed Rust-shaped CoreEvent timeline items
  in `appHarnessMain.tsx` / `basic-operations.spec.ts`, click the visible
  controls, and then push a CoreEvent diff to prove the UI reflects Rust-owned
  state changes. Do not update React state directly after `retry_send` or
  `cancel_send`.
- `RoomSendQueueUpdate::SendError` carries raw SDK errors. Project only coarse
  recoverable/unrecoverable status into DTOs and QA tokens; do not print raw SDK
  errors, transaction ids, Matrix ids, or message bodies in QA output.
- The `headless-core-qa` `send_queue` scenario injects offline failure through
  a stdlib TCP proxy inside the Rust QA binary and must be run with
  `--features qa-bin`; plain `cargo test` does not compile that binary. Verify
  both SyncService and LegacySync legs when changing retry/cancel semantics.
- New timeline item DTO fields must keep
  `apps/desktop/src/domain/coreEvents.ts`,
  `apps/desktop/src/domain/coreEvents.generated.json`, and
  `apps/desktop/src-tauri/src/lib.rs`'s core-event wire contract test in sync.

## Live Signals Phase A Notes

- `AppState.live_signals` is the Rust-owned source of truth for read receipts,
  fully-read markers, typing users, and presence. React may render it and
  dispatch typed commands only; do not add React-local receipt, marker, typing,
  or presence semantics.
- Timeline live-signal commands route through `TimelineCommand` and the
  subscribed `TimelineActor`: `SendReadReceipt`, `SetFullyRead`, and
  `SetTyping`. Account presence routes through `AccountCommand::SetPresence`.
  Keep SDK handles and sync policy in Rust actors.
- The current Phase A presence implementation records and emits the requested
  Rust-owned presence state. Full network presence propagation remains a sync
  backend decision because the legacy SDK path exposes `SyncSettings` presence
  while the current `SyncService` builder does not expose a direct setter.
- The Tauri snapshot is a hand-maintained DTO. When `AppState.live_signals` or
  another `AppState` field is added, update `apps/desktop/src-tauri/src/dto.rs`,
  `apps/desktop/src/domain/types.ts`, `browserFakeApi`, `tauriIpcMock`, app
  harness snapshots, and DTO serialization-contract tests in the same change.
  Headless browser mocks do not inherit Rust fields automatically.
- New `CoreEvent` variants need the Rust wire-contract test, generated
  `apps/desktop/src/domain/coreEvents.generated.json`, and
  `apps/desktop/src/domain/coreEvents.ts` updated together. Do not hand-write
  a TypeScript shape that is not proven by the Rust contract artifact.
- The local core QA `live_signals` scenario is token-only:
  `read_receipt=ok`, `fully_read=ok`, `typing=ok`, `presence=ok`,
  `live_signals=ok`. Do not print Matrix room IDs, event IDs, user IDs, message
  bodies, or raw SDK errors for this stage.
- If the `live_signals` scenario reaches `fully_read=ok` and then times out at
  typing on a probed SyncService local homeserver leg, first verify the legacy
  backend. In this environment, legacy receives the typing notification
  continuously, while the SyncService local leg needs a bounded debug/test
  `SyncOnce` on the observer account after `SetTyping` is acknowledged to wake
  the same Rust-owned typing observer. Do not replace this with React polling or
  local UI timers.

## Live Signals Phase B Notes

- The full-app browser harness (`apps/desktop/src/test/appHarnessMain.tsx`)
  must import `../styles.css`, matching production `main.tsx`. Otherwise
  visibility/layout assertions can pass against unstyled DOM and miss real
  production CSS issues.
- Event-driven `TimelineItemRow` uses the same `.message` grid contract as the
  legacy snapshot `MessageArticle`: direct child `.avatar`, `.message-main`,
  and row-level `.message-actions`. Keep direct-child grid placement explicit;
  pre-placing the actions without placing the main cell can push message
  content into the 44px avatar column and hide media titles.
- React may use refs only to suppress duplicate viewport-triggered command
  dispatches such as mark-read/read-receipt sends. Receipt, read-marker,
  typing, and presence values themselves remain Rust-owned
  `AppState.live_signals`.

## E2EE Trust Phase A Notes

- Device verification SDK handles are actor-private resources. Keep
  `VerificationRequest` and `SasVerification` wrapped in
  `matrix-desktop-sdk` opaque handles and store them only inside
  `AccountActor`; snapshots, Tauri DTOs, TypeScript types, and React state get
  only `VerificationFlowState` plus private-data-free SAS emoji DTOs.
- Verification progress is Rust-owned. `AccountActor` listens to SDK
  request/SAS state streams and projects `VerificationSasPresented`,
  `VerificationCompleted`, or `VerificationFailed`; GUI code must not infer
  SAS readiness, completion, or cancellation from local React state.
- SAS mismatch is not a generic UI cancel. Route it as
  `VerificationCancelReason::Mismatch` so the reducer settles
  `VerificationFlowState::Failed { kind: Mismatch }` and `AccountActor` calls
  the SDK `SasVerification::mismatch()` path. Plain user decline/cancel uses
  `VerificationCancelReason::User` and returns the reducer to `Idle`.
- Incoming verification requests are discovered by the Rust `AccountActor`
  observer, not by GUI code. Follow-up verification commands must pass the
  Rust-owned `flow_id` from `AppState`; their command `request_id` is separate
  and is used only for command submission/failure correlation.
- Incoming verification observers may report the same SDK verification flow
  more than once as sync catches up. `AccountActor` must ignore duplicate
  incoming requests with the same SDK `flow_id`; only a different active flow
  should be cancelled/rejected.
- SAS peer acceptance is driven by SDK SAS state, not by React state or the SDK
  `we_started` flag. In this wrapper, `Started` is the peer side that must call
  `accept_sas_verification`; `Created` is the local side after `start_sas` and
  must not be auto-accepted.
- In same-user two-device SAS QA, keep the request direction A2 -> A and let
  the requester A2 start SAS after A accepts. Starting SAS from the accepting
  device reproduced Tuwunel `m.key_mismatch` cancellation before emoji
  presentation, while the requester-start sequence is stable across local
  Conduit and Tuwunel.
- During the local SAS proof, do not overlap continuous SyncService delivery
  with manual `SyncOnce` nudges. Start the verification request while sync is
  running so device data is fresh, then pause both sync loops and drive SAS
  request/ready/start/key/done with bounded `SyncOnce` polling. Overlap
  reproduced pre-SAS key-mismatch flakes.
- Identity-reset auth continuation follows the same separation: GUI commands
  must use a fresh command `request_id` for submission correlation and pass the
  Rust-owned identity-reset `flow_id` from
  `AppState.e2ee_trust.identity_reset`. Do not reuse React-local pending ids or
  infer the flow from button state.
- Verification observers and SDK handles must be stopped/cancelled on logout,
  account switch, and actor shutdown before dropping the Matrix session.
- `BootstrapCrossSigning` may carry a UIAA password `AuthSecret` only inside
  the `CoreCommand::Account` command boundary. Its reducer action, effect,
  event, snapshot, logs, and `Debug` output must remain secret-free.
- `EnableKeyBackup` may carry an optional recovery passphrase `AuthSecret`
  only inside the `CoreCommand::Account` command boundary. Use it for
  passphrase-backed local proof or future product input, but never project the
  passphrase or returned recovery key into reducer state, DTOs, logs, or QA
  output.
- `RestoreKeyBackup` is secret-bearing only at the `CoreCommand::Account`
  boundary. Its reducer projection, `AppEffect`, `CoreEvent`, Tauri DTO, and
  React state must never carry the recovery secret.
- `RestoreKeyBackup` must not be runtime gated to `SessionState::Ready` only.
  A newly logged-in device can become `NeedsRecovery` after sync discovers
  secret storage, and key-backup restore is the operation that gets it out of
  that state. Let `AccountActor` enforce that a store-backed Matrix session
  exists; `SignedOut` still fails as `SessionRequired`.
- The vendored SDK's backup-wide all-room-key download helper is private.
  Current Phase A restore code must use public SDK APIs only: recover/import the
  secret, then hydrate currently joined rooms with
  `Backups::download_room_keys_for_room`. Do not patch vendored SDK just to call
  `download_all_room_keys` unless that patch is separately justified and
  recorded in the upstream feedback ledger.
- Key-backup restore progress in the current public-API slice counts joined-room
  hydration attempts. Do not describe it as exhaustive backup-wide restore until
  a local homeserver QA lane proves the exact all-session behavior.
- The local core QA `e2ee_trust` scenario logs the same synthetic user into a
  second data directory/device and proves cross-signing bootstrap, encrypted
  seed-room key-backup upload, wrong-secret restore failure, successful
  passphrase restore on the second device, SAS device verification, and
  identity reset through `CoreCommand`/`CoreEvent` only. Its stdout must stay
  token-only for these checks; do not print account keys, verification target
  user/device ids, backup versions, room ids, event ids, recovery secrets, or
  raw SDK errors.
- The local headless runner registers separate synthetic users for the SDK lane
  and each core backend leg. Keep E2EE trust proofs isolated per core leg so
  unrelated smoke-test devices do not become part of the account's device graph.
- Room-list space classification can lag behind room/space create or join on
  local homeservers, especially Conduit. Headless core QA should perform a
  bounded `SyncOnce` after A creates/invites and after B joins before asserting
  `rooms` vs `spaces`; otherwise a valid space can temporarily appear as a plain
  room and make aggregate lanes flaky.
- Invite and DM membership state is Rust-owned. `AppState.invites` is projected
  by `RoomActor` from SDK invited rooms; React must render it and dispatch
  typed commands (`AcceptInvite`, `DeclineInvite`, `StartDirectMessage`) instead
  of maintaining local invite lifecycle state. In the SyncService backend, the
  live room-list entries adapter must use the non-left filter so invited-room
  diffs wake the projection loop; a joined-only filter leaves
  `invite_recv=ok` stuck with zero invites even after sync succeeds.
- The local core QA `invites_dm` scenario proves incoming room/space invite
  receipt and accept, invite decline, and DM start/invite projection through
  token-only stdout (`invite_recv=ok`, `invite_accept=ok`,
  `invite_decline=ok`, `dm_start=ok`). Do not print Matrix room IDs, user IDs,
  or raw SDK errors for this stage.
- Run the local proof with the SyncService/probed core leg while iterating:
  `npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=e2ee_trust --core --core-backend=probed --timeout-ms=240000`.
  The runner supports `--core-backend=legacy|both` for non-E2EE backend
  coverage, but the Phase A E2EE trust proof is the probed SyncService leg.

## E2EE Trust Phase B GUI Notes

- Trust GUI controls are transport clients only. Add Tauri commands as thin
  `CoreCommand::Account` submitters and keep SDK calls, UIAA/OAuth continuation
  handles, and verification handles inside Rust actors.
- React must render `snapshot.state.e2ee_trust` and dispatch typed API methods.
  Do not add React-local pending/success/failure state for verification,
  cross-signing, key backup, or identity reset. Button-click feedback must come
  back through the Rust-owned snapshot/event path.
- Verification and device DTOs include user/device ids for Rust correlation,
  but the GUI should not display those ids by default. Use ordinal/status labels
  (`Device 1`, `Verified`, etc.) unless a Rust-owned redacted display model is
  added. Playwright/Vitest assertions must not print verification targets,
  account keys, backup versions, recovery secrets, or raw SDK errors.
- Identity-reset password/UIAA input may exist only as transient DOM input that
  is immediately sent to Tauri. Clear the input after submit, and verify the
  mocked IPC layer records password fields as `[REDACTED]`.
- When adding trust GUI tests, update `apps/desktop/src/test/appHarnessMain.tsx`
  with Rust-shaped `e2ee_trust` fixtures and command responses. Do not test
  trust success by mutating React component state; assert the returned snapshot
  changed and the expected Tauri command name/flow id was invoked.
- All visible trust labels/status text must go through `apps/desktop/src/i18n/messages.ts`.
  SDK-provided SAS emoji descriptions are not catalog strings; render emoji
  symbols or add a Rust-owned localized DTO before showing descriptions.

## Rust-Owned Settings Notes

- Settings product state lives in `matrix-desktop-state::AppState.settings`.
  GUI work may render it and dispatch `update_settings`, but must not make
  locale, theme, font/emoji, or composer-send shortcut preferences a React or
  localStorage source of truth.
- Locale/display behavior is resolved by
  `matrix_desktop_state::resolve_locale_display_profile`. GUI components may
  consume the resulting `lang`, `dir`, catalog locale, pseudo-locale mode,
  platform, and modifier labels, but must not parse raw language tags or own
  fallback locale rules.
- `LocaleDisplayProfile` is a snapshot contract field, not a browser-only
  convenience. When it changes, update `apps/desktop/src-tauri/src/dto.rs`,
  `apps/desktop/src/domain/types.ts`, `browserFakeApi`, `tauriIpcMock`, app
  harness snapshots, and the DTO serialization-contract tests together.
- `TypographyDisplayProfile` follows the same DTO rule. It is resolved in
  Rust from `SettingsValues.typography` plus the platform profile and exposes
  only font/emoji preference and asset-status tokens. GUI code may apply those
  tokens to root attributes/CSS; it must not invent Inter/Twemoji/system
  fallback behavior per component.
- Font asset loading is Phase B. Inter and Twemoji COLR are bundled-preferred
  choices with system fallbacks, and any included font package must update
  `THIRD_PARTY_NOTICES.md` with version, local path, license, and provenance.
  The current Twemoji COLR package (`twemoji-colr-font@15.0.3`) is pinned but
  npm marks it deprecated; do not upgrade or replace it without checking the
  rendered family name, license stack (package/font/artwork), and browser
  COLR/CPAL behavior.
- Keep the root font stack as a single resolved custom property, e.g.
  `font-family: var(--font-ui)`. A 2026-06-15 Phase B attempt used
  `font-family: var(--font-ui), var(--font-emoji)` with list-valued variables;
  headless Chromium rendered the page, but Playwright `locator.click()` hung at
  the actionability "visible, enabled and stable" step for ordinary buttons.
  Fold emoji fallbacks into `--font-ui` / `--font-message` instead of chaining
  list-valued font variables at the declaration site.
- Root `lang`/`dir` and active catalog selection come from
  `snapshot.state.locale_profile`. Raw visible strings in React components
  should fail the catalog gate unless they are reviewed structured registry
  data or synthetic fixture content.
- Composer key behavior belongs to the Rust-owned resolver in
  `matrix-desktop-state`, shared by main, thread, and edit composer surfaces.
  GUI code normalizes DOM/native key input into typed resolver facts and then
  dispatches/renders the returned action.
- When `AppState.settings` or any settings enum changes, update the Tauri DTO,
  TypeScript domain types, `browserFakeApi` defaults, `tauriIpcMock`, app
  harness snapshots, and the DTO serialization-contract test in the same
  change. Headless mock snapshots do not automatically inherit Rust fields.
- The settings file is a non-secret JSON store under the core data directory
  (`settings/settings.json`). Do not route it through the credential store and
  do not add Matrix IDs, message content, raw SDK errors, credentials, tokens,
  recovery material, SDK store keys, or search-index keys to it.

## Local Gates Setup

- Enable the repo pre-commit hook once per clone:
  `git config core.hooksPath .githooks`. It runs the secret scan on staged
  files (`scripts/desktop-secret-scan.mjs --staged`).
- Gate commands (from `apps/desktop`): `npm run qa:secret-scan`,
  `npm run qa:wasm-check` (requires
  `rustup target add wasm32-unknown-unknown`), `npm run qa:release-gates`
  (structural credential-gate check plus `cargo check --release`; the compile
  step is slow on a cold target dir — use
  `node ../../scripts/desktop-release-gate-check.mjs --no-compile` for the
  quick structural pass).
- There is no hosted CI in this repo yet; these gates run locally and in
  `release:preflight`. Wire them into CI when CI infrastructure appears.

## Headless UI (Playwright) Flakes

- `e2e/basic-operations.spec.ts:81` ("submitting the composer in reply mode
  invokes send_reply, not send_text") is flaky in the FULL `test:ui-headless`
  run but passes reliably when that spec file is run in isolation
  (`npx playwright test e2e/basic-operations.spec.ts`). Root cause is a
  test-layer timing race, not a product bug: the App's snapshot refresh
  (`get_snapshot`) returns the harness's static Plain `readySnapshot`, which can
  land after the reply-target click and momentarily reset the composer mode to
  Plain so the submit dispatches `send_text`. It reproduces on a clean checkout
  (predates the 2026-06-14 rules-compliance remediation) and is amplified by
  parallel-file worker contention on the shared Vite harness server. Workaround
  while it is unfixed: run the reply specs in isolation, or `--workers=1`.
  A durable fix should make the harness `get_snapshot` response consistent with
  the reply lifecycle (or have the App refresh from owned state, not a static
  mock). The `reply send does not repair product state by cancelling reply mode`
  regression added in that remediation passes deterministically in isolation.
- For i18n headless tests that first push a locale/profile snapshot and then
  mutate the event-driven timeline, prefer updating the already-seeded room row
  with `ItemsUpdated.Set` at generation `1`. A one-off `InitialItems` emitted
  around the same snapshot refresh can be swallowed by harness timing and leave
  the seed row visible, even though the root `lang`/`dir` update succeeded.
- File attachment GUI tests must not open a native file dialog. Use the
  Composer's hidden `input[type=file][aria-label="Attach file input"]` and
  Playwright `setInputFiles()` with synthetic bytes. The visible button should
  be located with `getByRole("button", { name: "Attach file", exact: true })`
  because browsers expose file inputs with button semantics and the input label
  contains the button label as a prefix.
- Transaction timeline rows use `timelineItemDomId`, so local echoes render
  with `data-item-id="txn:<transaction_id>"`. Headless media-progress specs
  should target that canonical id instead of the raw transaction id.
- Media GUI rendering is DTO-only. React may display `TimelineItem.media`
  filename/mimetype/size/dimensions/encrypted flag and
  `MediaUploadProgress`, but it must not parse Matrix event content, render MXC
  URIs, store downloaded bytes, or synthesize upload/download lifecycle state.

## Linux GUI QA Container

- Build the committed lane image with
  `docker build -f docker/linux-gui.Dockerfile -t matrix-desktop-linux-gui:basic-ops .`
- The committed image includes `conduit`, `tuwunel`, and `zstd` so the
  `--scenario=local-login` and `--scenario=local-send` lanes can run against
  local homeservers entirely inside the container.
- The Docker recipe pins Rust toolchain `1.96.0` for reproducibility.
- The lane image includes `libnss-wrapper` so the numeric container UID can be
  given a temporary passwd/group entry during DBus-authenticated GUI smoke.
- Run the lane from the repo root with the workspace mounted at `/work`:
  `docker run --rm -it --shm-size=2g -u "$(id -u):$(id -g)" -v "$PWD:/work" -v /tmp/matrix-desktop-cargo-home:/tmp/cargo-home -v /tmp/matrix-desktop-gui-target:/tmp/matrix-desktop-gui-target -v /tmp/matrix-desktop-npm-cache:/tmp/npm-cache -w /work -e HOME=/tmp -e RUSTUP_HOME=/opt/rustup -e CARGO_HOME=/tmp/cargo-home -e CARGO_TARGET_DIR=/tmp/matrix-desktop-gui-target -e NPM_CONFIG_CACHE=/tmp/npm-cache -e PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin matrix-desktop-linux-gui:basic-ops bash -c 'export RUSTC="$(rustup which rustc)"; export RUSTDOC="$(rustup which rustdoc)"; npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=/work/artifacts/linux-gui-local-send-docker --timeout-ms=180000'`
- The runner writes artifacts to `artifacts/linux-gui-local-send-docker/` inside the mounted
  repo. Keep that directory ignored and inspect the run log and screenshots
  there when a lane fails.
- Faster Ubuntu 24.04 host loop:
  one-time package install still needs `sudo`/root, but tests and smoke then
  run as a normal user. Install the host packages with
  `sudo apt-get update && sudo apt-get install -y --no-install-recommends build-essential ca-certificates curl dbus-x11 file fontconfig fonts-dejavu-core fonts-noto-color-emoji fonts-noto-core git libayatana-appindicator3-dev libnss-wrapper libssl-dev libwebkit2gtk-4.1-dev libxdo-dev librsvg2-dev pkg-config webkit2gtk-driver xvfb`, then install the driver with `cargo install tauri-driver --locked`. Fast checks are
  `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`,
  `node scripts/desktop-linux-gui-qa.mjs --check-tools`, and
  `npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-login --server=conduit --artifact-dir=artifacts/linux-gui-local-login-host --timeout-ms=180000` or
  `npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=artifacts/linux-gui-local-send-host --timeout-ms=180000`.
  Docker remains the reproducible release/CI gate.

## Fast Linux GUI Inner Loop

- After the one-time host package install, run the GUI QA lanes as a normal
  user; no `su` or root shell is needed for the fast loop.
- Prepend the local homeserver binaries when iterating so the host lanes use
  the checked-in QA binaries first:
  `export PATH=/tmp/matrix-desktop-local-qa-bin:$PATH`
- Build the debug app once, then reuse it with `--skip-build` (optionally
  `--app-binary=PATH`) so each scenario trial skips the full Tauri rebuild:
  `npm --prefix apps/desktop run tauri build -- --debug --no-bundle`, then
  `PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-room --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-create-room-fast --timeout-ms=180000`
- Settings/composer-shortcut GUI changes use the same fast loop with
  `--scenario=local-settings`. This opens the real Settings UI, changes the
  Rust-owned composer shortcut and theme settings, verifies the E2EE trust
  settings section renders in the real Tauri WebView, and waits for
  `aria-pressed="true"` / `data-theme="dark"` from the snapshot-driven UI:
  `PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-settings --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-settings-fast --timeout-ms=180000`
- Media GUI iteration has a focused virtual-display lane:
  `--scenario=local-media`. It writes a synthetic fixture file under the
  scenario artifact directory, sets that path on the Composer's hidden file
  input, uses a `DataTransfer` fallback when WebKit leaves `input.files` empty,
  waits for `timeline_room=true` and the Rust-owned `TimelineItem.media` row in
  the real Tauri WebView, clicks Download, and prints `gui_local_media=ok`:
  `PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-media --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-media-fast --timeout-ms=180000`
- When you only need a quick window-state sanity check, use the lane's cheap
  QA title helpers such as `--qa-title-ready` and `--qa-title-send-ready`
  before starting a full scenario run.
- Use focused scenarios first. Keep the artifact directories scenario-specific
  so retries do not blur login and send results:

  ```bash
  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    node scripts/desktop-linux-gui-qa.mjs --check-tools

  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    node scripts/desktop-linux-gui-qa.mjs --list

  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    npm --prefix apps/desktop run qa:linux-gui -- \
      --scenario=local-login \
      --server=conduit \
      --artifact-dir=artifacts/linux-gui-local-login-host \
      --timeout-ms=180000

  PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
    npm --prefix apps/desktop run qa:linux-gui -- \
      --scenario=local-send \
      --server=conduit \
      --artifact-dir=artifacts/linux-gui-local-send-host \
      --timeout-ms=180000
  ```
- Invite/DM GUI iteration has a focused virtual-display lane:
  `--scenario=local-invites-dm`. It seeds a second synthetic local user, accepts
  a real invite through the Invites pane, and starts a DM through the New DM
  dialog. The lane waits for `data-room-kind="dm"` in the real room list, so
  keep `RoomButton`'s data attributes in sync with the Rust-owned sidebar
  snapshot if the room list markup changes. This lane intentionally forces the
  legacy sync backend for deterministic WebDriver smoke; keep using the core
  `invites_dm` QA for SyncService/legacy invite-projection correctness.
- Reuse the existing Cargo, npm, and GUI target caches during the inner loop;
  do not rebuild the Docker image for every trial.
- Run Docker only when you need the committed reproducible lane or want to
  prove the release/CI recipe end to end. It is not the default fast
  iteration path.

## Linux GUI Local Operation Failures

- `--skip-build` reuses an existing debug binary, but the QA window-title
  tokens (`matrix-desktop qa session=...`) are baked into the frontend at build
  time behind `VITE_MATRIX_DESKTOP_QA_TITLE=1`. A binary built without that env
  shows the normal product title (e.g. `matrix-desktop · 1 unread`) instead, so
  the lane's `waitForLocalLoginReady` times out with "local GUI login did not
  reach a ready state. Last title: matrix-desktop · 1 unread". The runner's own
  build sets this env; when pre-building manually for `--skip-build`
  (`npm --prefix apps/desktop run tauri build -- --debug --no-bundle`), also set
  `VITE_MATRIX_DESKTOP_QA_TITLE=1`, or run one lane without `--skip-build` first
  to produce a QA-title binary the remaining `--skip-build` lanes can reuse.
- The Tauri snapshot is a hand-maintained DTO
  (`apps/desktop/src-tauri/src/dto.rs`, `FrontendAppState` / `From<AppState>`),
  NOT a passthrough of `AppState`. When `AppState` gains a field (e.g.
  `basic_operation`), the DTO must be extended in the same change, or the
  serialized snapshot silently omits it and the React UI crashes the moment it
  reads the missing field. Symptom: clicking a control blanks the WebView and
  `window.onerror` reports `undefined is not an object (evaluating
  'e.state.basic_operation.kind')`. Headless tests that use the browser fake or
  mock IPC will NOT catch this (they build their own snapshots); only the real
  Tauri lane or the `dto.rs` serialization-contract test does. Extend that test
  when adding `AppState` fields.
- The TypeScript snapshot shape is also hand-maintained in
  `apps/desktop/src/domain/types.ts`, `browserFakeApi.ts`,
  `tauriIpcMock.ts`, and `appHarnessMain.tsx`. When adding Rust `AppState`
  fields such as `e2ee_trust` or `invites`, update all mock/default snapshots
  in the same change and run `npm --prefix apps/desktop run typecheck`.
- New `CoreEvent` variants must be wired through the Tauri adapter's
  `serialize_core_event`, TypeScript `coreEvents.ts`, and the checked-in
  `coreEvents.generated.json` contract artifact. The src-tauri
  `core_event_wire_format_matches_checked_in_contract_artifact` test catches
  drift.
- E2EE trust Phase A commands/events are Rust-owned contracts only until the
  AccountActor SDK implementation lands. The fixture/demo backend should return
  typed unavailable/failure actions for trust effects and must not silently
  discard them.
- In production, `CoreCommand::Account` E2EE trust commands must be projected
  through the reducer before `AccountActor` routing. If this is skipped, the
  GUI can only infer pending trust state locally, violating the Rust-owned state
  machine rule.
- `matrix-desktop-sdk` is the SDK-facing boundary for E2EE trust operations.
  It maps SDK cross-signing/backup states into private-data-free
  `matrix-desktop-state` DTOs and redacts SDK error details in `Debug`. Do not
  let raw SDK trust errors, account keys, verification targets, or backup
  version identifiers leak through normal core events or QA output.
- Matrix identity reset can complete immediately or return an SDK auth
  continuation. Model that as Rust-owned `IdentityResetState`
  (`Idle`, `Resetting`, `AwaitingAuth`, `Failed`), not as React-local state or a
  nullable request id. `AwaitingAuth` exposes only UIAA/OAuth/unknown auth type;
  the SDK handle stays inside `AccountActor` and must be cancelled on logout,
  account switch, and actor shutdown. Auth continuation submission must be a
  `CoreCommand::Account` path that projects `ResetIdentityAuthSubmitted`
  through the reducer before actor routing; the GUI must not own SDK/UIAA/OAuth
  continuation semantics.
- If an E2EE trust `CoreCommand::Account` operation has already projected
  pending reducer state but the actor cannot complete it (session mismatch,
  unavailable local encryption, or an unimplemented SDK path), the actor must
  also send the matching reducer failure action. An `OperationFailed` event
  alone leaves Rust-owned pending state stuck and pushes recovery semantics
  toward the GUI.
- WebDriver `waitForDisplayed`/`click` does NOT reveal hover-gated controls.
  Timeline row actions (`.message-action` inside `.message-actions`) are
  `opacity:0` until `.message:hover`/`:focus-within`, so a direct
  `waitForDisplayed` on the reply button times out ("still not displayed") even
  though the headless Playwright tier passes (its click implicitly hovers).
  Move the pointer first: `await el.waitForExist(); await el.moveTo(); await
  el.waitForDisplayed(); await el.click();`.
- A reply must target a MESSAGE event, not a state event. The timeline includes
  state events (room create, membership) that carry no body; the SDK's
  `make_reply_event` rejects them (app stderr `make_reply_event failed:
  StateEvent`, surfaced as `send=failed`). `TimelineItemRow` therefore gates the
  reply affordance on `item.body !== null`, so only message rows are replyable.
  A `local-reply` lane must send/target a message and reply to that row, not the
  first event row in a fresh room (whose first events are state events).
- Timeline reactions are Rust-owned projection state. React must only dispatch
  typed `SendReaction` / `RedactReaction` commands; do not implement toggle
  semantics in the UI, because `Timeline::toggle_reaction` is only an internal
  Rust delegation detail behind the typed boundary. When `AppState` fields or
  the command surface changes, keep the Tauri DTO, TypeScript domain types, IPC
  mock, browser fake, and serialization-contract tests in sync in the same
  change.
- `local-media` must not use the visible Attach button to open a native file
  dialog. WebDriver should write an ignored synthetic fixture file in the
  scenario artifact directory, set that path on
  `input[type=file][aria-label="Attach file input"]`, fall back to
  `DataTransfer.files` if WebKit does not populate `input.files`, then wait for
  `timeline_room=true` and a Rust-owned media row. Do not monkeypatch
  `window.__TAURI_INTERNALS__` from WebDriver; WebKit driver execution contexts
  do not provide a reliable app-world command recorder. If the lane fails,
  inspect the scenario-specific artifact run log; the lane uses synthetic
  filenames/content only and must not write real/private media data.

## macOS GUI Smoke Failures

- `npm --prefix apps/desktop run qa:mac-gui` controls the Tauri window through
  macOS `System Events`. If it fails with `AppleScript timed out while
  controlling System Events`, grant Accessibility permission to the app running
  the agent, such as Codex, Terminal, or iTerm, then restart that app.
- If Accessibility is already enabled but the same timeout repeats, check
  Privacy & Security > Automation and allow the same app to control
  `System Events`. Restart the agent app after changing either permission.
- A repeated timeout can also be caused by AppleScript code, not permissions.
  In this repo, `process <variable>` hung when resolving the Tauri process.
  Use `first process whose name is <variable>` for variable process names.
- If screenshot capture is blocked, also grant Screen Recording permission to
  the app running the agent.
- In Tauri dev mode the macOS process name can be `matrix-desktop-app`, while
  the product/window title is `matrix-desktop`. GUI automation must check both
  names.
- Failed GUI smoke runs must clean up the full process group. A stale Vite
  process leaves port `5173` occupied and makes the next `tauri dev` fail.
- If a GUI smoke run is interrupted manually with Ctrl-C, verify that
  `lsof -nP -iTCP:5173 -sTCP:LISTEN` is empty before retrying. A stale
  `npm run tauri dev` process group can survive interruption and make the next
  run fail before the app reads the QA login FIFO.
- Do not pass the parent shell environment wholesale into GUI smoke child
  processes. Filter out secret-like variables such as API keys, tokens, and
  passwords before spawning `npm run tauri dev`.
- First-run GUI smoke should set `MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=1`.
  Otherwise opening User Settings can read the macOS Keychain and show a
  confirmation prompt, which blocks unattended automation.
- Do not use `Cmd+Q` to stop the Tauri app from GUI smoke. If focus slips, the
  shortcut can reach Codex and trigger the "Quit Codex?" confirmation dialog.
  Let the script's process-group cleanup stop `tauri dev` and the app instead.

## Real Account Smoke Failures

- If `password-login-smoke --real-account-qa` fails at sync but
  `--check-room-list` succeeds, isolate the restore path first. A no-store
  `restore_session` can diverge from the product path; real-account QA should
  restore with a temporary encrypted SQLite SDK store, cache path, and encrypted
  search index path.
- The smoke CLI must try logout cleanup after any post-login QA failure unless
  `--keep-session` was explicitly requested. Otherwise failed sync/timeline QA
  can leave a live smoke device on the homeserver.
- `qa:real-homeserver` writes `qa.log` synchronously before leak checks and
  exit handling. If the log is missing after a fast successful exit, treat it
  as a regression in the runner.
- Store-backed Matrix SDK sessions must be dropped while a Tokio runtime context
  is entered. Dropping a sqlite-backed SDK client after the runtime context is
  gone can panic in `deadpool-runtime` with `there is no reactor running`.
- In this environment, starting `qa:mac-gui -- --real-login-from-stdin` through a
  non-interactive `exec_command` can deliver immediate stdin EOF. Use a PTY with
  terminal echo disabled, such as `stty -echo; npm --prefix apps/desktop run
  qa:mac-gui -- --real-login-from-stdin; exit_code=$?; stty echo; exit $exit_code`,
  then send the credential lines through stdin.
- Do not drive real-account login by fixed window-relative coordinates. A
  2026-06-12 GUI smoke attempt clicked the wrong login field and placed the
  password in the username field. Real-login GUI smoke should pass credentials
  through `MATRIX_DESKTOP_QA_LOGIN_PIPE`, which contains only a FIFO path in the
  environment and keeps the credential payload out of argv, logs, screenshots,
  and committed files.
- Real-login GUI smoke must set `MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`.
  `MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=1` only prevents saved-session reads; a
  successful login can still prompt macOS Keychain during session persistence or
  encrypted SDK store key creation.
- The `password-login-smoke` prompt order is homeserver, username, device name,
  then password. The `qa:mac-gui -- --real-login-from-stdin` order is
  homeserver, username, password, device name, then optional recovery code.
  Leave the fifth line empty to accept `needsRecovery` as a post-login sync QA
  state; provide it only when verifying recovery completion to `ready`.
- When driving `qa:mac-gui -- --real-login-from-stdin` through a PTY, send all
  five newline-terminated lines. Without the fifth blank or recovery line, the
  reader waits for more input and the Tauri window is never launched.
- Do not store post-login real-account screenshots. They can contain room names,
  Matrix IDs, message bodies, or attachment names. Real-account GUI automation
  should rely on private-data-free QA window-title tokens instead. Use
  `--allow-private-screenshots` only for explicitly approved test accounts whose
  post-login room and message data may be written to ignored artifacts.
- Some sparse QA accounts have valid room-list sync but no visible timeline
  items in the automatically selected room. Keep the strict
  `timeline_items > 0` release signal for normal real-account smoke, but use
  `qa:mac-gui -- --allow-empty-timeline` for sparse test accounts when the goal
  is validating login, room-list sync, and GUI panel automation.
- Avoid repeated destructive real-account login cycles while debugging GUI
  automation. Prefer preserving the same running Tauri session while iterating
  on panel/menu checks, and only restart when the script or Tauri capability
  changes require it.
- Use `qa:mac-gui -- --qa-profile=<name>` when a real-account GUI run should
  preserve SDK SQLite store, cache, search index, saved session, and incremental
  sync state across runs. Profile names must be synthetic and non-secret; data is
  stored under ignored `.local-secrets/qa-profiles/<name>/data`.
- The default `qa:mac-gui -- --real-login-from-stdin` path is intentionally
  disposable and sets `MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`.
  `--qa-profile=<name>` is the opt-in path for persistent restore/sync QA and
  must set `MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR` so unattended runs do
  not prompt macOS Keychain. This env-controlled file credential store must stay
  behind a debug/test-only compile-time gate; release builds must ignore it and
  use the OS credential store. If a profile run shows a Keychain prompt, treat
  it as an automation failure and verify that env var is present in
  `--child-env`.
- If synthetic send smoke reaches `send=failed` while login, sync, and timeline
  are otherwise ready, check that the product room list excludes non-joined
  rooms before QA timeline sampling. Matrix SDK `Room::send` requires joined
  room state, and a left room with visible history can otherwise become the
  active QA room.

## Local Homeserver QA Failures

- Installing Conduit or Tuwunel from source with `cargo install --git` must set
  `RUMA_UNSTABLE_EXHAUSTIVE_TYPES=1`. Without it, Ruma marks many public API
  structs as non-exhaustive and both homeservers fail to compile with
  `E0639: cannot create non-exhaustive struct using struct expression`.
- On macOS, install Tuwunel with `--no-default-features` unless a Linux-oriented
  build profile is intentional. The default feature set includes deployment
  features such as `systemd`/`io_uring` that are not useful for local desktop QA.
