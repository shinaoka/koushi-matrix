# Matrix Desktop Design

Date: 2026-06-11
Status: Spike-validated; ready for full implementation planning

## Goal

Build a Windows/macOS desktop Matrix client prototype that follows Element X's Rust SDK direction while providing a Slack-like desktop user experience. The first version focuses on E2EE text chat, Spaces, room timelines, threads, desktop interaction, and ngram full-text search.

Video calls, voice calls, screen sharing, bots, widgets, and app integrations are out of scope for the MVP.

## Core Decisions

- Desktop shell: Tauri.
- Frontend: React + TypeScript.
- Backend: Rust, with Matrix state handled by `matrix-sdk` and `matrix-sdk-ui`.
- UI style: Slack-like multi-pane desktop app.
- Search: patch `matrix-sdk-search` with ngram tokenizer support as a prerequisite SDK milestone before full app implementation, then upstream feedback when stable.
- DM model: DMs are global account-level conversations, not duplicated under Spaces. The MVP uses the SDK DM classification configured to the Element X Android-style two-member definition, then applies one consistent classification for sidebar grouping, search filtering, and Space exclusion.
- Space model: Spaces drive the left rail and filter the room/channel list.
- Thread model: threads open in a right-side pane when width permits, otherwise as a drawer or focused view. Thread subscriptions and read-state behavior are conditional until proven against the SDK and homeserver support.
- Element X mobile code may be used as an implementation reference. Direct code ports must preserve upstream license and copyright notices.

## Prerequisite Spikes

Do not move directly from this design into a full app implementation plan. First prove the following blockers with small, testable spikes:

1. Search SDK capability patch.
   Add ngram tokenizer configuration to `matrix-sdk-search`, persist tokenizer/schema version information, define index rebuild behavior, and verify room/global search with Japanese/CJK mixed text.
2. Search correctness with Matrix event lifecycle.
   Prove edit, redaction, late decryption, event-cache lag recovery, encrypted index opening, and wrong-secret failure behavior with tests.
3. Desktop sidebar composition.
   Build a small Rust-side composition model over `RoomListService`, `SpaceService`, and room metadata that produces the Slack-like Space rail/sidebar DTOs, including global DMs.
4. Key and credential-store integration.
   Prove macOS Keychain and Windows credential storage access from Tauri/Rust, including secret creation, retrieval, namespacing, zeroization, logout deletion, and missing-secret recovery.

The full implementation plan should start only after these spikes establish viable APIs and test coverage.

## Repository Layout

```text
matrix-desktop/
  docs/
    superpowers/specs/
  THIRD_PARTY_NOTICES.md
  frontend/
    React application
  src-tauri/
    Tauri application and Rust backend
  vendor/
    matrix-rust-sdk/
      crates/matrix-sdk/
      crates/matrix-sdk-ui/
      crates/matrix-sdk-search/
```

`vendor/matrix-rust-sdk` is kept as a repository-local SDK checkout rather than flattening every SDK crate into the top-level Cargo workspace. This preserves upstream structure and makes feedback or future PRs easier. The Tauri backend should consume the patched SDK through path dependencies or `[patch]` entries.

## Upstream Reference and License Policy

Element X iOS and Element X Android should be used as primary references for how production apps consume the Rust SDK FFI, `RoomListService`, `Timeline`, Spaces, recovery, verification, and room-level services.

Reference use means studying architecture, API usage, state flow, and edge cases, then writing original code for this project. Reference-only use does not copy implementation text.

Direct porting means copying or closely adapting code, file structure, non-trivial functions, or tests from Element X mobile. Direct ports must follow these rules:

- keep the original copyright holders in the file header;
- keep the upstream SPDX license expression;
- record the upstream repository, file path, and commit SHA in `THIRD_PARTY_NOTICES.md`;
- keep local modifications clearly attributable through comments or commit history;
- do not remove AGPL or commercial-license notices from copied code;
- prefer porting small SDK-wrapper patterns over large UI modules.

As of the checked upstream files, Element X mobile source files carry:

```text
SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Element-Commercial.
```

The Matrix Rust SDK itself is Apache-2.0. SDK changes intended for upstream feedback should remain inside the SDK patch area and follow the SDK's licensing and contribution style.

Because this repository is private but may later produce distributed desktop binaries, implementation planning must include a license review before importing Element X mobile code verbatim. When practical, prefer using mobile code as a behavioral reference and reimplementing the desktop-specific layer independently.

## Architecture

```text
React UI
  room list
  timeline
  thread pane
  search UI
  context menus
  keyboard shortcuts
        |
        | Tauri commands/events with UI DTOs
        v
Rust backend
  session lifecycle
  Matrix client setup
  DTO mapping
  local encrypted storage
  search orchestration
        |
        v
matrix-rust-sdk
  matrix-sdk
  matrix-sdk-ui
    SyncService
    RoomListService
    Timeline
    Spaces
  matrix-sdk-search
    ngram tokenizer support
```

The frontend does not own Matrix state transitions. It renders UI state and sends user intent to Rust commands such as `login`, `select_room`, `send_message`, `open_thread`, `paginate_timeline`, and `search_messages`.

The Rust backend owns session state, room list subscriptions, timeline subscriptions, E2EE state, search indexing, and conversion from SDK models into stable UI DTOs.

`matrix-sdk-ui` should not be treated as a complete desktop UI state model. The backend needs a desktop composition layer that consumes SDK streams and produces app-specific DTOs for Space rail entries, Space-filtered room lists, global DMs, unread counts, selected room state, and thread pane state.

## UI Layout

The primary desktop layout has four panes:

1. Space rail: account home, Space icons, add/manage controls.
2. Sidebar: navigation, Space-filtered rooms, global DM section.
3. Timeline: selected room or DM, header, messages, composer.
4. Thread pane: selected thread, root message, replies, thread composer.

The top bar contains global navigation and search. Search can operate on the current room, current Space, all rooms, or DMs.

Responsive behavior:

- Wide: show all four panes.
- Medium: hide thread pane behind a drawer or toggle.
- Narrow: show one primary pane at a time, with sidebar/thread reachable by commands.

Desktop behaviors are first-class:

- right-click context menus for rooms and messages;
- hover actions for message operations;
- keyboard navigation and command palette;
- native menus where useful;
- no mobile long-press interaction as the primary path.

## Spaces and DMs

Spaces are treated as top-level navigation filters. Selecting a Space filters the sidebar room list to rooms in that Space.

The SDK Space filters may be limited in graph depth, so the MVP treats Space filtering as a best-effort view over the first supported levels exposed by the SDK. The desktop composition layer must define behavior for multi-parent rooms, nested Spaces, and rooms that are not reachable through the current Space filter.

DMs are global across the account. They appear in a global DM section regardless of active Space. Space-specific views may show recent or pinned DMs as convenience entries, but the canonical DM list is global. If a DM room is also linked under a Space, the room may be shown as a contextual shortcut, but it remains counted and searched as a DM.

Initial DM classification uses the SDK DM computation with the same two-member definition used by Element X Android. This choice must be used consistently for:

- global DM sidebar section;
- all-room vs DM search filters;
- Space room list exclusion;
- unread aggregation.

Unread counts are separated:

- Space unread counts include rooms under that Space.
- DM unread counts are global.
- Home shows aggregate activity across Spaces and DMs.

## Timeline and Threads

Room timelines are backed by `matrix-sdk-ui::Timeline`. The UI receives timeline item DTO updates through Tauri events.

Thread support should use SDK thread timeline support where available. The MVP should start with `TimelineFocus::Thread` for the right pane and keep server thread subscriptions disabled unless a spike proves they are stable for the target homeservers. The right pane is a separate focused timeline view bound to the selected thread root. If the window is too narrow, the thread opens as a drawer or replaces the main timeline temporarily.

If thread APIs, server support, or thread subscriptions are unavailable, the fallback UX opens a focused permalink-style context around the root event and replies instead of pretending full thread support exists.

The MVP should support:

- live timeline updates;
- backward pagination;
- sending text messages;
- editing and redacting own messages if SDK support is available;
- reply/thread open actions;
- read state display if stable enough.

## Search

Search is implemented by modifying `matrix-sdk-search`, not by embedding Seshat into the app. This work uses the SDK's `experimental-search` feature gate and should stay inside `vendor/matrix-rust-sdk/crates/matrix-sdk-search` plus the SDK search integration layer.

The patched search layer must support:

- default upstream-compatible tokenizer behavior;
- configurable ngram tokenizer, initially `min_gram = 2`, `max_gram = 4`;
- persisted tokenizer/schema metadata and deterministic rebuild behavior when the config changes;
- Japanese/CJK mixed text search;
- room search;
- global search;
- edit and redaction handling;
- redaction removal that is robust even when the original event is not present in the current event cache;
- indexing decrypted E2EE timeline events;
- late-decryption indexing for events that become decryptable after initial indexing;
- event-cache lag detection and reindex/recovery behavior;
- encrypted index open failure with the wrong secret;
- tests that are suitable for upstream feedback.

The app should select ngram search by default. Upstream-specific changes must remain UI-independent and isolated inside SDK/search crates.

Initial search scope:

- `m.room.message` text body;
- search result event id, room id, sender, timestamp, score, and a snippet/highlight strategy. Current SDK search returns event IDs/scores, so snippet/highlight generation must be explicitly designed rather than assumed.

Later search scope:

- file names;
- image/file captions;
- sender filters;
- date filters;
- Space filters;
- encrypted index key rotation;
- rebuild and repair UI.

## Backend Interfaces

The Tauri command layer should expose intent-oriented commands instead of raw SDK objects.

Initial commands:

- `login`
- `restore_session`
- `logout`
- `start_sync`
- `select_space`
- `select_room`
- `subscribe_timeline`
- `paginate_timeline`
- `send_text_message`
- `edit_message`
- `redact_message`
- `open_thread`
- `search_messages`
- `rebuild_search_index`

Initial event streams:

- `session_state_changed`
- `room_list_updated`
- `timeline_updated`
- `timeline_pagination_changed`
- `thread_updated`
- `search_index_state_changed`
- `search_results_updated`
- `error_reported`

DTOs should be versioned or isolated in a dedicated module so frontend changes do not force SDK leakage into React.

## Error Handling

Errors should be grouped by user action and recoverability:

- Authentication errors: show login/session recovery flow.
- Network/sync errors: show non-blocking banner and retry state.
- E2EE/decryption errors: render UTD state and allow retry where SDK supports it.
- Search index errors: allow rebuild; keep chat usable even if search is degraded.
- Storage errors: block affected session and show clear recovery options.

Search index rebuild or tokenizer migration must be explicit enough to avoid silent data loss. If the tokenizer configuration changes, the app should detect schema/config mismatch and rebuild the index.

## Security and Storage

The app should use encrypted local storage for Matrix state and search index data. Search indexing must only process decrypted events available to the local session.

Secrets and session keys should be stored through platform-appropriate secure storage where practical. The MVP can start with SDK-supported persistent stores, but the design must not require plaintext search indexes long term.

## Key Management

Matrix protocol key management is delegated to `matrix-sdk` and its crypto store. The app should not implement Olm/Megolm, room key sharing, cross-signing, secret storage, verification, or key backup logic itself.

SDK-owned responsibilities:

- device keys and one-time keys;
- Olm/Megolm sessions;
- inbound and outbound room keys;
- cross-signing state;
- device verification;
- secret storage and recovery;
- server-side room key backups;
- encrypted SQLite crypto store persistence.

App-owned responsibilities:

- creating or retrieving the local store unlock secret;
- storing that unlock secret in platform secure storage;
- passing the unlock secret to SDK store initialization;
- exposing recovery, verification, and backup state through UI;
- deciding when to prompt the user for recovery key/passphrase;
- deleting local secrets on logout or session reset.

For the desktop MVP, generate a high-entropy per-session local store secret at first login and store it in the OS credential store:

- macOS: Keychain;
- Windows: Credential Manager or DPAPI-backed credential storage.

The same local secret can be used to open the SDK SQLite store and encrypted search index, but it should be namespaced before use so store and search encryption do not share the exact same input string. For example:

```text
sdk_store_secret = HKDF(local_secret, "matrix-desktop:sdk-store")
search_secret = HKDF(local_secret, "matrix-desktop:search-index")
```

The Tauri backend should pass `sdk_store_secret` to `ClientBuilder::sqlite_store(..., Some(secret))` and configure `SearchIndexStoreKind::EncryptedDirectory(..., search_secret)` for the search index.

Implementation planning must decide whether each derived secret is treated as a passphrase string or a raw key. If the SDK exposes a raw-key path through lower-level store configuration, prefer that over forcing high-entropy random bytes through a passphrase API. Derived secrets must be zeroized after use where possible.

If secure storage is unavailable, the app should fail closed or ask for a user passphrase; it must not persist the store unlock secret in plaintext.

User-facing recovery remains separate from local unlock. Matrix recovery key/passphrase recovers cross-signing secrets and room backup keys from Matrix secret storage/backups. The local unlock secret only opens this device's local encrypted stores.

Credential-store records must use stable namespaced identifiers that include homeserver, user ID, and device ID. Logout and session reset must delete the corresponding credential-store records and local encrypted stores. If a local encrypted store exists but the credential is missing, the app should offer recovery by deleting local state and logging in again; it should not silently create a new unlock secret for an existing encrypted store.

## Desktop Platform Requirements

The MVP must account for Windows/macOS platform behavior before packaging:

- single-instance handling so two app instances do not open the same SDK stores;
- cross-process store locking behavior from the SDK;
- macOS code signing and notarization;
- Windows code signing;
- auto-update trust model and signing keys, if auto-update is enabled;
- OIDC/deep-link login redirect handling;
- native notification integration;
- platform-specific data, cache, media, and log directories;
- crash/rageshake-style diagnostic logs that avoid leaking secrets;
- proxy and custom certificate behavior;
- long-running SDK/search work moved off blocking Tauri command paths;
- clear behavior for Windows and macOS credential-store failures.

## Testing

Backend tests:

- ngram tokenizer behavior for Japanese and mixed English/Japanese;
- search result stability for edits and redactions;
- late-decryption indexing behavior;
- search index rebuild after tokenizer/schema migration;
- search index lag recovery;
- room/global search behavior;
- command-to-SDK DTO mapping;
- session restore and error transitions where feasible.
- local store unlock secret creation, retrieval, namespacing, and deletion;
- search index encrypted-open failure with the wrong secret.

Frontend tests:

- layout rendering for wide, medium, and narrow widths;
- right-click context menu behavior;
- thread pane open/close behavior;
- keyboard navigation basics;
- search panel state transitions.

Integration tests:

- mock SDK event streams into React;
- Tauri command smoke tests;
- local account/session smoke test against a test homeserver when available.

## MVP Non-Goals

- Voice/video calls.
- Screen sharing.
- Rich widgets.
- Bot framework.
- Full Element Web feature parity.
- Mobile UI.
- Multi-account support unless it falls out cheaply from the session model.

## Open Questions for Implementation Planning

- Whether `vendor/matrix-rust-sdk` should be a git submodule, subtree, or copied vendor checkout for the first iteration.
- Whether the first real login milestone should ship password login first, or add OIDC/SSO discovery in the same milestone.
- Recovery key/security phrase input is post-login E2EE recovery, not login. The first recovery milestone still needs a separate design for secret handling and verification UX.
- Which test homeserver setup to use for local integration testing.
- How much of thread support is stable enough in `matrix-sdk-ui` for MVP.
- Whether the first release should build Windows artifacts only in CI or also require early Windows manual testing.
