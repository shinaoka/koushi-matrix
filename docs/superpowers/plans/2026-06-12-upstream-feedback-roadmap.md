# Matrix Desktop To Upstream Feedback Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Advance the Matrix desktop prototype from the post-login foundation through desktop product hardening, with Element Desktop/Web-like UX on a Tauri and Rust SDK backend, isolated SDK feedback patches, tests, and concrete API feedback.

**Architecture:** Keep app-specific desktop composition in this repository, and keep upstreamable work isolated inside `vendor/matrix-rust-sdk`. The Rust backend owns Matrix session, E2EE recovery, sync, timeline, and search state; the React UI renders DTOs and sends user intent through Tauri commands. Element Desktop/Web is a UX reference, not an architecture reference: do not port Electron IPC, Element Web's JavaScript SDK state model, or Seshat integration.

**Tech Stack:** Rust, Tauri, React, TypeScript, Matrix Rust SDK, `matrix-sdk-ui`, patched `matrix-sdk-search`, OS credential storage, encrypted SDK/search stores.

**Security Constraints:** Follow `REPOSITORY_RULES.md`. Do not put real personal information, real account identifiers, tokens, recovery keys, passwords, decrypted message bodies, attachment names, or institution names into tests, fixtures, logs, or docs. Synthetic values must use neutral examples such as `example.invalid`.

**UX Reference Constraints:** Match Element Desktop/Web behavior where practical for the three-pane layout, thread/right-panel flow, Space menus, settings placement, message actions, composer behavior, room navigation, search, and keyboard shortcuts. Shortcut parity should default to exact Element behavior; every platform, Tauri, or MVP-scope mismatch must be recorded as `adapted` with a concrete reason. Full pixel parity is not required. Reusing Element/Compound assets or icons requires license review and attribution before copying.

---

## Current Position

- Password login is wired through the Matrix Rust SDK and deferred outside the Tauri backend mutex.
- Persistable SDK session material is stored through the OS credential store boundary.
- E2EE recovery submission is wired to `client.encryption().recovery().recover(...)`.
- Recovery prompting can be driven by SDK recovery state, with `Incomplete` treated as the prompt condition.
- SDK recovery state is observed through `client.encryption().recovery().state_stream()` after login/restore and routed into the app reducer.
- SDK SQLite store configuration uses a raw 32-byte local unlock key derived from an OS-managed local secret.
- SDK search index configuration can use `SearchIndexStoreKind::EncryptedDirectory` with a separately derived search key.
- The local search adapter already treats ngram as a candidate generator and verifies exact visible spans before showing highlights.
- `vendor/matrix-rust-sdk/crates/matrix-sdk-search` contains the ngram tokenizer spike.

The prototype is now past the code-level roadmap items through Milestone 9.
Phase 10+ work moves the product surface forward while preserving the
headless-first rule. The next execution plan is
`docs/superpowers/plans/2026-06-13-phase-10-ui-headless-product-surface.md`.
Remaining distribution gates are live OS credential-store tests on Windows,
macOS signing/notarization, Windows signing, and trust UX work for device
verification/cross-signing.
Real-homeserver QA now covers password login, recovery completion, encrypted
store restore, message body restore, room list/timeline behavior, send/edit/
redaction/search smoke, synthetic QA room leave/forget cleanup, and logout.

## Milestone 1: Harden The Login And Recovery Boundary

- [x] Keep `submit_login` and `submit_recovery` asynchronous and outside the backend mutex.
- [x] Verify recovery errors never include the submitted secret in `Display`, `Debug`, reducer state, frontend DTOs, or logs.
- [x] Observe `client.encryption().recovery().state_stream()` after login and restore, then dispatch a state action when the SDK transitions into `Incomplete`, `Enabled`, or `Disabled`.
- [x] Treat `Unknown` as "do not prompt yet" until sync/account-data observation has had a chance to run.
- [x] Keep password login, recovery key, and security phrase as UI inputs only; never persist them.
- [x] Run:

```bash
cargo test -p matrix-desktop-sdk e2ee_recovery
cargo test -p matrix-desktop-backend sdk_state_recovery_mode_does_not_prompt_before_sdk_reports_incomplete --test fake_backend
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml deferred_recovery_request_only_returns_request_while_recovering
```

## Milestone 2: Encrypted Local Store Model

- [x] Define app data directories for SDK SQLite store, SDK cache, and search index. Use OS-specific application data paths rather than project-relative paths.
- [x] Generate or load one OS-managed local unlock secret per account/device namespace.
- [x] Derive separate namespaced secrets for the SDK SQLite store and the search index.
- [x] Build the SDK client with `SqliteStoreConfig::key(...)` so the SDK SQLite store uses the raw local store key.
- [x] Configure search with `SearchIndexStoreKind::EncryptedDirectory(path, search_secret)`.
- [x] Add wrong-secret tests for SDK store opening and search index opening.
- [x] Add logout cleanup that deletes OS credentials and encrypted local stores for the selected session.
- [x] Run ignored live OS credential-store tests on macOS before distribution packaging.
- [ ] Run ignored live OS credential-store tests on Windows before distribution packaging.
- [x] Run:

```bash
cargo test -p matrix-desktop-key
cargo test -p matrix-desktop-sdk session
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml encrypted
```

## Milestone 3: Sync Service And Session Lifecycle

- [x] Start SDK sync only after login restore, recovery decision, and encrypted store initialization are complete.
- [x] Stop sync before logout, account switch, or app shutdown.
- [x] Convert sync lifecycle events into reducer actions: starting, running, failed, reconnecting, stopped.
- [x] Surface failures without including homeserver tokens, request bodies, or secrets.
- [x] On restore, rebuild the SDK client from the persisted Matrix session and encrypted store secret, then resume sync.
- [x] Run:

```bash
cargo test -p matrix-desktop-backend session
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml session
```

## Milestone 4: Matrix-SDK-UI Desktop Composition And Element-Like Shell

- [x] Add a backend service that consumes `matrix-sdk-ui` room list and timeline streams.
- [x] Produce stable DTOs for left navigation, Space-filtered rooms, global DMs, selected room, unread counts, selected thread, and contextual right-panel state.
- [x] Keep DMs global across all Spaces. A Space can show a DM shortcut only as a view convenience, not as ownership.
- [x] Replace the current non-Element shell with an Element Desktop/Web-like three-pane model: left navigation/room list, center timeline, contextual right panel.
- [x] Add Element-like right-panel modes for thread, room info, Space info, search context, and settings context.
- [x] Add Space header/menu entry points for Space home, preferences, settings, and invite. The first implementation may render lightweight placeholder panels where SDK data is not available yet.
- [x] Add user menu entry points for user settings, keyboard settings, account/session actions, and logout.
- [x] Add a shortcut registry and a read-only Keyboard settings view grouped similarly to Element's Composer, Room, Room List, Navigation, Autocomplete, and Accessibility categories.
- [x] Add an Element shortcut parity table with `same`, `adapted`, `deferred`, and `not applicable` statuses. Treat call/labs shortcuts as out of scope until those features exist.
- [x] Add tests for multi-parent rooms, rooms outside the selected Space, global DMs, unread aggregation, and account home.
- [x] Add frontend tests for right-panel mode switching, Space menu actions, user/settings menu actions, and the Keyboard shortcuts view.
- [x] Run:

```bash
cargo test -p matrix-desktop-backend room
npm --prefix apps/desktop test
npm --prefix apps/desktop run typecheck
```

## Milestone 5: Timeline, Composer, Edit, Redaction, And Threads

- [x] Subscribe to the selected room timeline through `matrix-sdk-ui::Timeline`.
- [x] Add backward pagination and preserve scroll anchors in the frontend model.
- [x] Send text messages through the SDK.
- [x] Edit and redact own messages through SDK APIs where available.
- [x] Keep edited-message display search-safe: the canonical displayed body is the latest valid replacement event.
- [x] Handle an edit arriving before the original event by storing a pending edit relation and applying it when the original arrives.
- [x] Open threads in the Element-like contextual right panel when width permits, and fall back to a focused view or drawer on narrow windows.
- [x] Open a thread from the message action bar, show the root event and replies, and keep the center timeline selected on wide screens.
- [x] Match Element-like composer shortcuts where implemented: `Enter` send, `Shift+Enter` newline, `Ctrl/Cmd+B`, `Ctrl/Cmd+I`, `Ctrl/Cmd+Shift+L`, `Esc` cancel reply/edit, undo/redo, and upload shortcut when upload exists.
- [x] Match Element-like room/timeline shortcuts where implemented: page up/down timeline scrolling, jump to first/latest message, room navigation, unread-room navigation, and right-panel close/toggle.
- [x] Run:

```bash
cargo test -p matrix-desktop-state timeline
cargo test -p matrix-desktop-backend timeline
npm --prefix apps/desktop test
```

## Milestone 6: Real Search Integration

- [x] Wire the app search path to SDK search instead of fake candidates.
- [x] Configure ngram search as the app default with `min_gram = 2` and `max_gram = 4`.
- [x] Keep ngram results as candidates only. Show a result only after exact visible-span verification against the canonical rendered message or attachment filename.
- [x] Index attachment filenames as a distinct searchable field.
- [x] Apply edits, redactions, and pending edit relations before generating snippets and highlights.
- [x] Add a late-decryption queue so events that become decryptable after initial sync are indexed later.
- [x] Add event-cache lag detection and a room reindex path.
- [x] Add encrypted index rebuild behavior when tokenizer/schema config changes.
- [x] Match Element-like search entry points where implemented: `Ctrl/Cmd+K` for room/search navigation, `Ctrl/Cmd+F` for current-room search, and scoped search UI for current room, current Space, all rooms, and DMs.
- [x] Keep search results in the Element-like right panel or search context surface where it best matches the active layout, without exposing unverified ngram candidates.
- [x] Run:

```bash
cargo test -p matrix-desktop-search
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml ngram
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search,sqlite,e2e-encryption
```

## Milestone 7: Security, Privacy, And License Gate

- [x] Scan docs, tests, fixtures, and UI mocks for personal information, institution names, real Matrix IDs, tokens, passwords, recovery keys, and realistic private meeting content.
- [x] Confirm `.local-secrets/` is ignored, empty by default, and not force-added.
- [x] Confirm `AuthSecret`, stored sessions, derived keys, and recovery inputs have redacted debug output.
- [x] Confirm decrypted message bodies and attachment filenames are only persisted inside encrypted SDK/search stores.
- [x] Confirm Element mobile and Element Web/Desktop code was used only as reference, or copied code/assets/icons have original copyright, SPDX license, upstream path, and commit SHA recorded.
- [x] Run:

```bash
git diff --check
git diff --cached --name-only
git ls-files --stage -- .local-secrets
find .local-secrets -type f
```

## Milestone 8: Upstream Feedback Packet

- [x] Split SDK-only changes from app changes. The upstreamable patch should live only under `vendor/matrix-rust-sdk` and avoid desktop UI assumptions.
- [x] Minimize the `matrix-sdk-search` ngram API to a general SDK configuration surface.
- [x] Include tests for default tokenizer behavior, ngram tokenizer behavior, invalid ngram config, Japanese/CJK mixed query, encrypted index opening, edit/redaction handling, and late-decryption reindex gaps.
- [x] Write a feedback note that separates finished patch material from API questions.
- [x] Report concrete SDK issues discovered by the desktop integration, especially recovery state timing, late-decryption indexing hooks, and thread timeline stability.
- [x] Keep UI/UX-only Element compatibility work out of the upstream SDK patch. Upstream feedback should cover SDK APIs and behavior, not desktop visual choices.
- [x] Run the SDK-focused verification set:

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk-search/Cargo.toml
cargo test --manifest-path vendor/matrix-rust-sdk/crates/matrix-sdk/Cargo.toml search_index --features experimental-search,sqlite,e2e-encryption
git -C vendor/matrix-rust-sdk diff --check
```

## Milestone 9: Desktop Product Hardening After Feedback

- [x] Add account switching with separate encrypted stores per account/device.
- [x] Add homeserver URL validation that accepts explicit `https://`, defaults to HTTPS when omitted, and preserves custom ports.
- [x] Add native menu items, window state persistence, context menus, and keyboard shortcuts.
- [x] Map Element-compatible shortcuts into Tauri menu accelerators where native handling is required, including macOS `Cmd+,` for user settings and platform-standard close/quit behavior.
- [x] Resolve shortcut conflicts across macOS, Windows, Linux, browser text editing, Tauri native menus, and React handlers. Record every mismatch from Element as `adapted` with the reason.
- [x] Flesh out user settings, room settings, Space settings, and Keyboard settings beyond entry-point placeholders.
- [x] Add Element-like context menus for rooms, messages, Spaces, and account/user actions where the underlying command exists.
- [x] Add installer/signing preparation for macOS and Windows.
- [x] Add crash-safe recovery for corrupted local stores and search index rebuild.
- [x] Add manual QA scripts for login, restore, recovery, search, edit, redaction, logout, account switch, shortcut parity, right-panel behavior, settings placement, and Space info/settings flows.

Implementation evidence is tracked in `docs/qa/milestone-9-completion-audit.md`.
Distribution gates still required before shipping signed binaries are macOS signing/notarization,
Windows signing, live OS credential-store ignored tests on Windows, and
real-homeserver QA as release/preflight evidence.

## Milestone 10: Headless UI Contract And Harness Hardening

- [ ] Treat `npm --prefix apps/desktop run test:ui-headless` as the canonical
  GUI-free DOM gate.
- [ ] Extend the Vite/Playwright harness and `TauriIpcMock` so the real app
  shell, not only isolated timeline pieces, can be mounted with synthetic
  snapshots and fake `CoreEvent` streams.
- [ ] Add command-shape assertions for room selection, right-panel open/close,
  search entry, settings entry, and logout entry.
- [ ] Keep `@wdio/tauri-service` browser mode behind a spike until the
  installed package proves it can run browser mode without a Tauri binary,
  native driver, native window, or OS keychain access.
- [ ] Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run test:ipc-contract
```

## Milestone 11: Product UI Layout And Interaction Surface

- [ ] Build the Element-like three-pane desktop surface in React over the
  existing snapshot/event contract.
- [ ] Verify right-panel modes, settings, search, shortcut UI, focus return,
  narrow-width behavior, and scroll anchoring in Vitest/Playwright headless
  tests.
- [ ] Keep new Matrix behavior out of React. If a UI action requires runtime
  behavior that core does not expose, design the core command/event first and
  verify it headlessly.
- [ ] Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run qa:secret-scan
```

## Milestone 12: Runtime Transport Integration Hardening

- [ ] Add or update Rust tests for every Tauri command that forwards to
  `CoreRuntime`.
- [ ] Keep production Tauri command paths free of fixture-backend behavior.
- [ ] Rerun local homeserver QA whenever Matrix behavior changes.
- [ ] Rerun real-homeserver QA before GUI-level confidence, release-preflight
  claims, or changes affecting login, recovery, sync, encrypted restore,
  search, room cleanup, or logout.
- [ ] Run:

```bash
cargo test -p matrix-desktop-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run qa:headless-local -- --server=both
npm --prefix apps/desktop run qa:real-homeserver
```

## Milestone 13: Native GUI Smoke And OS Integration

- [ ] Launch the real Tauri app only for native behavior that headless tests
  cannot prove: real IPC, native window lifecycle, OS menu accelerators,
  WebView integration, and keychain/system-dialog behavior.
- [ ] Keep macOS native GUI smoke attended. Unattended agents must not launch
  the GUI app.
- [ ] Continue credential entry only through FIFO or approved debug/test
  credential-store paths.
- [ ] Run, only after coordination with the user:

```bash
npm --prefix apps/desktop run qa:mac-gui
```

## Milestone 14: Distribution, Trust UX, And Release Hardening

- [ ] Run live OS credential-store ignored tests on Windows.
- [ ] Prepare signed macOS and Windows release builds, including notarization
  and installer verification.
- [ ] Design device verification and cross-signing under `AccountActor` before
  claiming E2EE trust UX completeness.
- [ ] Review vendored SDK patches at phase exit and remove any patch that is
  no longer indispensable.

## Immediate Next Work

1. Execute Milestone 10 from
   `2026-06-13-phase-10-ui-headless-product-surface.md` using the subagent
   reviewer/implementer pattern.
2. Keep real-homeserver QA in Milestone 12 and release preflight, not in
   ordinary React-only headless UI loops.
3. Defer native GUI smoke to Milestone 13 after headless UI, runtime transport,
   local homeserver, secret-scan, and real-homeserver gates are green.
