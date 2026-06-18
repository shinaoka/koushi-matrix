# Element X-Informed Roadmap Design Batch

Date: 2026-06-17
Status: approved direction for design batching; pending user review before
implementation planning.

## Goal

Group the remaining umbrella #12 tasks that need product or architecture design
approval, check them against Element X where it is useful, and record one
binding design surface before implementation resumes.

This design does not close any child issue. It prevents per-issue design
re-approval churn by approving the architecture boundaries, dependency order,
and issue grouping for the remaining roadmap.

## Inputs

- Umbrella issue #12 open inventory, refreshed on 2026-06-17.
- Element X iOS `develop` branch, especially secure backup service boundaries:
  <https://github.com/element-hq/element-x-ios/blob/develop/ElementX/Sources/Services/SecureBackup/SecureBackupControllerProtocol.swift>
- Element X Android `develop` branch, especially encryption, room-list, and
  OAuth references:
  <https://github.com/element-hq/element-x-android/blob/develop/libraries/matrix/api/src/main/kotlin/io/element/android/libraries/matrix/api/encryption/EncryptionService.kt>
  <https://github.com/element-hq/element-x-android/blob/develop/libraries/matrix/api/src/main/kotlin/io/element/android/libraries/matrix/api/roomlist/RoomListService.kt>
  <https://github.com/element-hq/element-x-android/blob/develop/docs/oauth.md>
- Existing repository rules:
  `REPOSITORY_RULES.md`, `AGENTS.md`,
  `docs/architecture/overview.md`, and
  `docs/architecture/state-machine.md`.

Element X code is a behavioral and boundary reference only. Do not copy or
closely port source, tests, styling, strings, or assets without a separate
license review and attribution plan.

## Design-Approval Scope

The following remaining umbrella #12 tasks need design approval before
implementation because they introduce new product semantics, protocol models,
auth/secret handling, persistent state, privacy policy, platform capability
decisions, or final QA scope:

- Auth, security, recovery: #37, #38, #45, #46, #47.
- Sync, rooms, spaces: #39, #48, #61.
- Content extensions and indexes: #50, #51, #52, #56, #57, #59, #60.
- Safety, notifications, shell, QA, branding: #40, #44, #49, #9, #31, #53.

The following are not part of this design-approval batch:

- #54 license and attribution: execution/hygiene work, not product design.
- #66 and #67: evidence-only native compatibility tasks.
- #62: living distinctive-feature index; promoted child issues are the
  acceptance units.

## Binding Architecture Rules

1. Phase A remains Rust/state-machine/headless first. Phase B is GUI and
   browser-headless/Linux virtual-display evidence.
2. React renders Rust-owned snapshots and dispatches typed commands. It may own
   ephemeral presentation state only when product facts already come from Rust.
3. Secret-bearing data is command-boundary only. Passphrases, recovery keys,
   access tokens, refresh tokens, room-key files, raw device IDs, IP addresses,
   URLs from private rooms, room IDs, event IDs, and message content do not
   appear in reducer state, DTOs, logs, QA output, or issue comments.
4. Platform differences are typed capability/profile data or native adapter
   calls. Do not scatter OS-specific product semantics through React
   components.
5. Shared Rust files remain serialization points for the main agent:
   `state.rs`, `action.rs`, `reducer.rs`, `command.rs`, `event.rs`,
   `runtime.rs`, Tauri DTOs, TypeScript wire artifacts, i18n, and state-machine
   docs.
6. Cheap or mini agents may investigate and patch module-local code, but the
   main agent owns shared contracts, review, docs, issue status, and closure.

## Element X Reference Decisions

Element X validates the direction of pushing protocol and crypto semantics into
SDK-backed services instead of UI state.

- Secure backup/recovery is split into recovery state, key-backup state, setup
  progress, reset/recover operations, and backup upload steady-state. Kagome
  mirrors that separation in Rust-owned state rather than forcing all
  operations into the existing `key_backup` enum.
- Element X Android exposes recovery-key setup progress with a final recovery
  key. Kagome deliberately diverges at the GUI boundary: recovery keys may be
  handled as one-shot native/Rust artifacts, but they must not be projected into
  React state, DTO snapshots, logs, or QA tokens.
- Element X Android's room-list service requires the sync service to be running
  and provides dynamic lists from the live service. Kagome already has the same
  constraint: use the single live `SyncService`/`RoomListService` source, never
  ad-hoc room-list services for product projections.
- Element X Android's OAuth notes show redirect URI, dynamic client metadata,
  and Rust SDK OIDC usage as service-level concerns. Kagome keeps OIDC/MAS
  discovery, PKCE state, token exchange, refresh, revocation, and account
  management URLs in Rust/Tauri auth state, not React.

## Package A: Auth, Security, Recovery

Issues: #37, #38, #45, #46, #47.

### State Boundaries

Introduce account/auth sub-states for:

- OIDC/MAS discovery and login: provider metadata, capability facts,
  web-delegated registration URL, account-management URL, redirect wait state,
  and coarse failure kind.
- Token lifecycle: access token and refresh token stay inside the session/store
  actor; snapshots expose only `password`, `oidc`, `softLogout`, or
  `reauthRequired` style account mode facts.
- Shared UIA: one Rust-owned UIA flow model reused by device delete, password
  change, account deactivation, 3PID changes, and identity reset continuation.
- Device/session manager: device list, rename state, destructive sign-out
  state, inactive/verified classifications, and soft-logout re-auth state.
- Secure backup/key management: split room-key file export, room-key file
  import, secure-backup setup, passphrase change, and backup upload progress
  from the existing restore state.
- QR login: rendezvous capability, displaying/scanning/verified/failed states,
  and E2EE bootstrap handoff.

### #46 Detailed Split

Keep #46 as one issue, but implement it internally as:

1. A1 room-key file export/import.
   Use `matrix-rust-sdk` public `export_room_keys` and `import_room_keys` APIs.
   Rust/Tauri owns file paths and passphrases. State contains request id,
   running/succeeded/failed, counts, and coarse failure kind only. The file
   format is the Matrix key-export format used by Element clients, including
   the encrypted Megolm session data header/footer; Kagome must not add a custom
   JSON, archive, or wrapper format around it.
2. A2 secure-backup setup and passphrase change.
   Use SDK recovery APIs such as `enable`, `reset_key`, and
   `recover_and_reset`. Recovery-key material is delivered through a one-shot
   Rust/Tauri artifact path or native save operation, not through React state.
3. B security settings GUI.
   GUI renders Rust-owned status and operation states and dispatches typed
   commands. Browser-headless tests use private-data-free tokens and synthetic
   fixtures.

### Dependency Order

1. #37 OIDC/MAS login and account-management URL model.
2. #38 shared UIA and device/session state, including soft logout.
3. #46 secure backup and key-management state.
4. #47 account management, reusing #37 account-management delegation and #38
   UIA.
5. #45 QR login after the auth and E2EE bootstrap boundaries are stable.

## Package B: Sync, Rooms, Spaces

Issues: #39, #48, #61.

### State Boundaries

Use the live sync-owned room-list service as the only source for room-list
projection. The Rust layer owns:

- Sync mode and capability state: simplified sliding sync supported/running,
  legacy fallback, initial-sync state, and private-data-free failure kind.
- Room-list filters: unread, people, rooms, favourites, invites, activity sort,
  and pagination/load-more state.
- Mark read/unread: fully-read marker advancement, `m.marked_unread`, and
  stale/failed command behavior.
- Room upgrade/archive: tombstone and predecessor/replacement links,
  follow-to-replacement, left-room/archive projection, and forget command.
- Spaces: space child/parent management, suggested rooms, hierarchy browse,
  add-existing-room state, and loop-safe traversal.
- Threads list: per-room aggregate thread projection, source selection from
  SDK MSC3856 support or relation-derived fallback, ordering, pagination, and
  open-thread command.

### Dependency Order

1. #39 sync mode, filters, and read/unread foundation.
2. #48 room upgrade, spaces management, and archived rooms over the stable room
   and space projection.
3. #61 threads list after existing thread pane and focused-context navigation
   are the source of truth.

## Package C: Content Extensions And Indexes

Issues: #50, #51, #52, #56, #57, #59, #60.

### State Boundaries

Create Rust-owned feature sub-states and query surfaces for new content types:

- #50 polls: `m.poll.start`, `m.poll.response`, `m.poll.end`, last-vote-wins
  aggregation, disclosed/undisclosed tally policy, own vote, ended flag, and
  creator end command.
- #51 location: static `m.location` send/render first; live beacons are a
  deferred sub-state with start/stop/current-position projection. QA tokens
  never print precise coordinates.
- #52 stickers/custom emoji: account and room image packs, shortcode
  resolution, sticker send command, pack CRUD/import/export, and reaction/
  composer integration through Rust-owned pack DTOs.
- #56 attachment/file browser: extend the encrypted search/index store with
  attachment metadata and query by room, direct child space, or account-wide
  scope. Completeness is limited to synced/indexed history and must be stated
  in docs and UI.
- #57 URL previews: fetch/cache preview data in Rust, obey global and per-room
  settings, default off in encrypted rooms, store viewer-local hidden-preview
  state in Rust, and never fetch private-room URLs without explicit opt-in.
- #59 chat export: Rust serializes text and JSON first; HTML can follow after
  the artifact path is stable. E2EE export decrypts locally and writes a
  plaintext artifact only after an explicit warning. QA output contains counts
  and artifact status only, not content.
- #60 saved messages/bookmarks: use a namespaced account-data event as the
  default cross-device store. The Rust projection owns save/unsave/list and
  jump-to-source facts. A local-only mode can be a later explicit preference,
  not the default.

### Dependency Order

1. #56 attachment index foundation, because #42 media gallery and future export
   options reuse one attachment store.
2. #57 URL preview privacy/cache model, because it affects timeline rendering
   and scroll stability.
3. #59 chat export artifact model, using timeline/store/attachment facts.
4. #60 saved messages, because it reuses focused-context and message-action
   surfaces.
5. #50 polls, #51 location, and #52 stickers/custom emoji as independent event
   model slices. They may be developed in parallel if shared timeline DTO files
   are integrated by the main agent.

## Package D: Safety, Notifications, Shell, QA, Branding

Issues: #40, #44, #49, #9, #31, #53.

### State Boundaries

- #40 ignore/block users and report content:
  `m.ignored_user_list` is Rust-owned account data. Suppression applies in
  timeline, invite, profile/member, presence, and notification projections.
  Report commands expose only coarse success/failure state.
- #44 notification mode and privacy opt-out:
  Per-room notification mode is push-rule-backed Rust state. Global privacy
  toggles decide whether read receipts and typing notifications are sent at the
  command layer. React must not independently suppress commands.
- #49 app shell polish:
  Preferences, labs flags, app-lock state, clear-cache effects, density/layout,
  time format, font scale, command-palette data, and breadcrumbs are Rust-owned.
  The palette UI may be React presentation, but selection navigates through
  Rust state.
- #9 expanded gates:
  Product walkthroughs become stable gates after child feature evidence exists.
  Tokens are private-data-free and checked by scripts, not merely printed.
- #31 integration matrix:
  This is the final edge-case pass. Keep it synchronized with
  `docs/qa/integration-edge-cases.md` and run it after feature-specific gates.
- #53 branding / #82 rename:
  The shipped product name is **Koushi**. The repository codename remains
  `matrix-desktop`. Internal identifiers were migrated from Kagome to Koushi
  (`chat.koushi.desktop`, `koushi-desktop`, `app.koushi.local_aliases`) with
  read-old-write-new migration for persisted keychain and account-data entries.
  Do not use Matrix branding in shipped product names.

### Dependency Order

1. #40 and #44 before QA/product walkthrough claims, because they alter
   suppression, privacy, and notification behavior across surfaces.
2. #49 shell preferences after shared settings state and before final #9
   walkthroughs.
3. #53 availability check before public distribution metadata changes.
4. #9 and #31 close last.

## Implementation Planning Model

After this spec is reviewed and approved, write a new implementation plan rather
than trying to implement from this design directly.

Recommended plan structure:

1. Package A plan first, because #46 is the next umbrella-order candidate and
   auth/recovery touches secrets.
2. Package B plan next, because sync/room-list state supports multiple later
   surfaces.
3. Package C plan in smaller event-model chunks, allowing independent agents
   only when shared timeline DTO files are serialized.
4. Package D plan last except for #40/#44, which may be pulled earlier if they
   unblock notification/privacy QA.

Each package plan must include:

- current issue refresh before work starts;
- Phase A tests and private-data-free QA tokens;
- state-machine and docs updates in the same change;
- Phase B GUI serialization notes;
- issue-comment and close criteria.

## Acceptance For This Design Batch

- Element X has been checked and used only as a boundary/reference source.
- Design-approval-needed issues are identified and grouped.
- #46 is no longer a standalone design blocker; it is part of Package A with
  an explicit A1/A2/B split.
- Evidence-only and living-index issues are excluded from product design
  approval.
- Implementation remains blocked until the follow-up implementation plan is
  written and approved.
