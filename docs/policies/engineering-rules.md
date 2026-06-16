# Engineering Rules

Status: normative detailed policy. This document extends the root durable rules
in [REPOSITORY_RULES.md](../../REPOSITORY_RULES.md) with concrete policy for
secrets, logging, QA automation, async/runtime behavior, GUI automation, and
build gates. AGENTS.md remains the operational how-to (permissions, install
caveats, recovery steps); durable rules discovered there are promoted to
REPOSITORY_RULES.md or this document.

Last amended: 2026-06-16.

## Secrets and Private Data

Never log, print, commit, or store in fixtures:

- access tokens, passwords, recovery keys or recovery codes
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
3. QA credentials enter processes via FIFO (`MATRIX_DESKTOP_QA_LOGIN_PIPE`)
   or the gated file credential store — never via argv, never typed by
   coordinates, never echoed to a terminal, never in screenshots or logs.
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
   environment (`MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`,
   `MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR`) instead.
9. OS notifications, badge labels, and QA window-title tokens are
   private-data-minimized surfaces. By default they may include only a safe
   room display label, notification kind (`mention`, `dm`, `message`), and
   aggregate unread/highlight counts. They must not include message bodies,
   sender identifiers, room IDs, event IDs, transaction IDs, raw SDK errors,
   or secrets. Native attention candidates and platform capability profiles are
   Rust-owned DTOs; React and platform adapters must not add account/content
   fields while mapping them to macOS, Windows, Linux, tray, sound, badge, or
   no-op behavior. React attention helpers must consume
   `AppState.native_attention` directly; they must not aggregate room lists,
   diff previous room snapshots, or synthesize dedupe/focus/mute semantics.
   Persistent native effects (window title, badge count, Windows overlay, tray
   count, and zero-badge clearing) are snapshot-state mappings. Transient sound
   and activation effects are candidate-scoped and must run only from a
   Rust-owned notification candidate plus the Rust-owned capability DTO, never
   from every unread/badge snapshot refresh.
   Passive notification dispatch may check existing OS permission, but must not
   prompt for permission except through an explicit user or onboarding action.
   Native notification clearing is best-effort adapter work triggered by
   Rust-owned zero-badge/logout state; clearing failures must not mutate Matrix
   state or surface as React rendering failures. Platform capability profiles
   for native attention are resolved in Rust from the shared `DisplayPlatform`
   model before reaching React; React components and notification helpers must
   not add their own macOS/Linux/Windows capability branches. Windows taskbar
   overlay routing uses the Rust-owned `overlay_icon` capability field, not
   React-side OS detection.
   Space rail attention badges come from the Rust-owned `SidebarModel.space_rail`
   projection, and timeline thread chips come from Rust-projected row
   `thread_summary` DTOs. React may render those fields and dispatch typed
   navigation/open-thread commands, but it must not scan rooms or timeline rows
   to invent space/thread attention semantics. Pane-level thread attention is
   `AppState.thread_attention`, mirrored through the Tauri/TypeScript DTO; React
   may render it but must not derive it from visible timeline rows or local
   thread chips. Core producers may update it from remote live thread timeline
   events; backfill/prepend diffs and the current user's own messages must not
   create notification markers.
   GUI thread indicators such as the Threads nav badge/markers render only the
   Rust-owned `notification_count`, `highlight_count`, and
   `live_event_marker_count` fields from `AppState.thread_attention`; they must
   not be derived from room-list totals, `TimelineItem.thread_summary`, or
   visible thread rows. Notification sound policy is the Rust-owned
   `SettingsValues.notifications.sound` value; React may pass that DTO to the
   native adapter so sound is skipped, but it must not keep a separate
   notification preference or mutate native attention state.
10. Device-local settings are non-secret product state, but they are still a
   privacy boundary. Settings files may contain only typed preferences such as
   locale, theme, font/emoji choice, keyboard behavior, and notification
   policy. They must not contain Matrix credentials, tokens, recovery material,
   local unlock secrets, SDK/search keys, raw Matrix session JSON,
   room/event/user IDs, message bodies, attachment filenames, search queries,
   or raw SDK errors.
   Derived display profiles for these settings, including
   `LocaleDisplayProfile` and `TypographyDisplayProfile`, are also
   non-secret profile data only. They may carry platform/capability and
   asset-status tokens, but not account identifiers, content, local paths, raw
   errors, or credentials.
   Notification preferences live in Rust-owned `SettingsValues.notifications`
   and are persisted through the settings store. Older settings JSON files that
   predate notification preferences must deserialize with safe defaults instead
   of forcing React to invent a local notification policy. Notification
   settings UI controls may dispatch typed `SettingsPatch.notifications`
   updates, but browser-headless coverage must drive the visible controls and
   assert the `update_settings` payload plus the returned Rust-shaped snapshot
   state; React must not repair switch state locally after dispatch.
11. E2EE trust diagnostics are kind-only. Verification, cross-signing,
   key-backup, and identity-reset commands/events may expose structured state to
   the UI, but normal `Debug`, QA logs, and window-title tokens must redact
   account keys, verification target user/device IDs, backup versions, raw SDK
   errors, identity-reset auth details beyond UIAA/OAuth/unknown, and all key
   material. Key-backup restore progress/copy must say joined-room hydration
   when that is the implemented scope; it must not imply exhaustive backup-wide
   restore. `KeyBackupRestoreSummary.scope` stays `JoinedRooms` for the MVP
   path unless a later upstream/public API decision explicitly broadens it.
12. Credential-store health diagnostics are kind-only. Public state may report
   only `unknown`, `healthy`, `unavailable`, `locked_or_inaccessible`,
   `missing_credential`, or `reset_required`, with optional private-data-free
   remediation hints. Raw OS/keyring errors, local paths, account identifiers,
   key labels, local unlock secrets, SDK/search keys, and recovery material must
   stay inside `StoreActor`/adapter diagnostics gated for debug/test. OS
   credential calls go through the `matrix-desktop-key` credential backend
   abstraction and the StoreActor path; GUI code may only dispatch typed probe
   or reset commands and render `AppState.local_encryption`. Debug/test file
   credential stores remain behind debug/test-only cfg, and release builds must
   ignore those environment variables. `reset_local_data` is a Rust-owned
   AccountActor/StoreActor operation: it clears current-account local
   persistence and returns the app to a local signed-out snapshot. React must
   not implement local-data cleanup through a UI-only logout path. macOS Tier 2
   Keychain evidence must run only through the env-gated temporary-keychain
   test on a real macOS session. The previous manual
   `macos-keychain-tier2.yml` workflow is temporarily disabled and preserved
   under `.github/workflows.disabled/`; do not dispatch it until it is
   deliberately re-enabled. Any re-enabled lane must not use the debug/test file
   credential store, must not require the private vendored Matrix SDK submodule,
   and its output remains private-data-free.
13. Media/file diagnostics are metadata-minimized. `CoreCommand` may carry
   filename, caption, mimetype, dimensions, and bytes when sending media, and
   `TimelineItem.media` may expose safe render metadata. Normal `Debug`, QA
   logs, errors, window-title tokens, and docs examples must not expose
   filenames, captions, bytes, MXC URIs from real accounts, encrypted media
   keys/hashes, room IDs, event IDs, or raw SDK errors. Download effects emit
   byte counts or app-owned handles only; downloaded bytes stay in Rust-owned
   effects or platform ports. Media captions belong to the Rust-owned media
   upload request and incoming timeline projection; GUI code must not implement
   caption semantics by sending a separate `m.text` event after media upload.
14. Profile/avatar diagnostics are metadata-minimized. Display names and avatar
   bytes may cross only the typed command or snapshot boundaries needed for the
   UI; normal `Debug`, QA logs, errors, window-title tokens, and issue evidence
   must not expose real display names, avatar MXC URIs, avatar bytes, local
   thumbnail paths, encrypted media keys/hashes, or raw SDK errors. React must
   render avatar images only from Rust/platform-owned ready source URLs and must
   fall back to generated initials for MXC, loading, or failed thumbnail states.
   Personal local user aliases are private account-data-backed profile state:
   they must not be sent as Matrix profile updates, room events, message content,
   notification text, QA tokens, logs, issue evidence, or normal `Debug` output.
   Rust reducers own alias set/clear/list and display-name resolution; React
   must not maintain an alias cache separate from `AppState.profile`.
   `UserProfile.display_label`, `UserProfile.original_display_label`, and
   `UserProfile.mention_search_terms` are the person/mention DTO contract.
   React may use those projected fields but must not recompute alias
   precedence from `local_aliases` or derive original names by stripping an
   alias.
   Timeline sender surfaces use the same contract:
   `TimelineItem.sender_label`, `ReplyQuote.sender_label`, and
   `ThreadSummaryDto.latest_sender_label` are Rust-projected display fields.
   Raw sender ids stay identity/source data and must not be used as normal
   timeline display labels when a projected label is present.
   Existing timeline rows are relabeled only from Rust-provided
   `TimelineEvent::DisplayLabelsUpdated` patches. React may apply the supplied
   `user_id -> display_label` values to already-loaded rows by matching raw
   identity fields, but it must not resolve aliases, inspect `local_aliases`,
   or invent fallback labels locally.
   Room title surfaces use `RoomSummary.display_label`, not `display_name`.
   `RoomSummary.original_display_label` is the alias-free context label for
   tooltips/profile surfaces. DM room labels resolve in Rust from `dm_user_ids`
   through local alias, upstream room name, profile/own-profile, then MXID;
   non-DM room labels use trimmed upstream room name, then room id.
   `display_label`/`original_display_label` are room/user data and are not a
   place for catalog prose or generic English fallbacks such as `Member`. React
   must not infer DM targets from room titles.
   Alias set/edit/clear GUI controls may own only dialog visibility and input
   draft text. They dispatch the typed `set_local_user_alias(user_id,
   alias|null)` command and wait for Rust-shaped snapshots or
   `TimelineEvent::DisplayLabelsUpdated` before visible names change. Browser
   headless coverage for alias UI must assert both the typed command arguments
   and the Rust-projected `display_label` / `original_display_label` rendering.
   Read-receipt reader avatars use the same boundary: `AppState.live_signals`
   carries Rust-resolved reader labels, avatar DTOs, read timestamps, capped
   reader ordering, and overflow counts; React must not resolve reader profiles
   locally or include real reader names/avatar MXC URIs in QA tokens, logs, or
   issue evidence.
15. Message-action diagnostics are metadata-minimized. `TimelineItem.actions`
   may expose coarse affordance booleans and synthetic/test permalinks through
   the typed timeline DTO, but normal `Debug`, QA logs, errors, window-title
   tokens, and issue evidence must not expose real Matrix room IDs, event IDs,
   generated permalinks, message bodies, sender IDs, transaction IDs, or raw SDK
   errors. React must not generate Matrix permalinks or decide action
   eligibility locally; it may render/copy only the Rust-owned DTO values.
   `TimelineMessageSource` is a safe UI projection, not a raw event dump, and
   forward commands must send Rust-projected content rather than React-copied
   message bodies. GUI message-action menus may own only presentation state
   such as popover visibility. They must wait for `MessageSourceLoaded` before
   displaying source details and must not synthesize source/forward/copy
   content from raw event data.

## Logging and Diagnostics

1. Diagnostics are structured and redacted
   (`core.sync.failed kind=http` style). Structured fields are enums/kinds;
   free-form string fields are prohibited because they eventually carry
   content.
2. Raw SDK errors may be printed only behind an explicit debug/test
   diagnostic switch. They must never reach `AppState`, committed logs,
   normal test fixtures, or release diagnostics.
3. QA asserts on `CoreEvent` and `AppStateSnapshot`, never on log output.
4. Real-account and real-homeserver QA output is tokenized before it becomes an
   artifact. Captured logs must not contain raw Matrix IDs, event IDs,
   transaction IDs, user IDs, message bodies, search queries, local paths, or raw
   SDK errors. Producers should avoid formatting those values; wrappers must not
   write unredacted stdout/stderr and only then discover a leak.

## Async and Runtime

1. No fixed sleeps in QA or product code waiting for Matrix effects — wait
   on events with timeouts.
2. Store-backed Matrix SDK clients must be dropped while a Tokio runtime
   context is entered; otherwise `deadpool-runtime` panics with
   "there is no reactor running".
3. Every spawned background task and subscription has an owner responsible
   for cancelling it (unsubscribe, account shutdown, app shutdown). No
   unbounded maps of live subscriptions.
4. Timeline scrollback is a split contract: core emits diffs and pagination
   state; React owns DOM anchoring. Product code must not issue automatic
   pagination loops before the previous diff has rendered and anchor
   restoration has completed.
5. In Tauri production, the QA title `timeline_items` token is the legacy
   `AppState.timeline` snapshot length, not the event-driven `TimelineView`
   DOM row count. Local GUI lanes that exercise timeline row controls must wait
   for CoreEvent-rendered DOM state such as `.message`, `data-event-id`, or the
   typed action control they intend to click.
6. QA runners must clean up their full process group on failure or
   interruption. Verify `lsof -nP -iTCP:5173 -sTCP:LISTEN` is empty before
   retrying a GUI run; a stale Vite/`tauri dev` process breaks the next run.
7. QA binaries must attempt logout cleanup after any post-login failure
   unless `--keep-session` was explicitly requested; otherwise failed runs
   leave live devices on the homeserver.
8. Avoid repeated destructive real-account login cycles while debugging
   automation; reuse the running session and restart only when the script
   or Tauri capability changes require it.
9. State-critical actor actions are reliable messages, not lossy hints. Do not
   ignore failed reducer-action sends for transitions that set or clear pending
   user-visible state. Await the send, retry through the owner, or emit a
   correlated operation failure that leaves no stuck pending state.
10. If a reducer returns an `AppEffect` that matters in production, the
   production runtime executes it or the behavior is redesigned as an explicit
   `CoreCommand`/actor command. Discarding such effects is allowed only for
   fixture/demo effects that are documented as non-production.

## GUI Automation

GUI automation is a thin smoke layer, never the primary correctness gate.

0. UI behavior is verified headless by default: frontend tests run in a
   headless browser with mocked Tauri IPC and fake `CoreEvent` streams
   (QA Model layer 4). The real Tauri app is launched only for the minimal
   native-integration smoke, and on macOS only attended — unattended agent
   sessions must not launch the GUI app (it opens real windows, reads the
   OS keychain, and surfaces crash dialogs on the user's desktop).
   The repository's canonical headless DOM gate is currently
   `npm --prefix apps/desktop run test:ui-headless` using Playwright against
   the Vite harness. `@wdio/tauri-service` browser mode may be adopted only
   after a spike proves the installed package can run the frontend in a
   normal browser without a Tauri binary, native driver, native window, or
   OS keychain access.
   Real-Tauri GUI automation by agents is allowed only under a virtual
   display (Linux Xvfb + `tauri-driver`; not available on macOS). The goal
   is that agents carry GUI design and testing as far as headless/virtual
   harnesses allow; only macOS-specific native behavior remains attended.
   i18n GUI work must also prove the Rust-resolved `LocaleDisplayProfile`
   reaches the DOM root (`lang`, `dir`, catalog, pseudo mode), remote/user text
   keeps `dir="auto"`, pseudo/CJK/RTL samples do not overflow the shell, and
   raw user-facing strings/logical-CSS contracts are covered by headless tests.
   Headless helpers that seed fake event-driven timeline rows must wait for the
   resulting DOM identity (`data-item-id`) and fail if the CoreEvent was not
   applied; fixed-count fire-and-forget event emission is not valid evidence.
   Room-list section tests must prove tag-driven movement from Rust-shaped
   `RoomSummary.tags` snapshots, not from React-local menu state after
   `set_room_tag` or `remove_room_tag` is clicked.
   Room-list shell tests must also prove Element-aligned section order,
   section counts, unread badges, and mention dots from Rust-shaped
   `SidebarModel` fields (`unread_count` / `highlight_count`), not from local
   React-derived notification state.
   Display preferences such as code-block wrapping live in Rust-owned
   `SettingsValues.display` and are persisted through the same settings store.
   Older settings JSON must backfill safe display defaults before React reads a
   snapshot. GUI code may map `code_block_wrap` to CSS only and must not keep a
   separate local wrap policy.
   Received message formatting is a Rust-owned security projection:
   `TimelineItem.formatted` is sanitized from Matrix `formatted_body` before it
   crosses the WebView boundary, and carries only sanitized HTML plus derived
   plain text/code-block metadata. React must never render unsanitized server
   HTML or own sanitizer policy. TimelineView's formatted renderer is a
   presentation adapter only: it maps the Rust-owned DTO into React nodes,
   code-copy controls, search highlights, and `code_block_wrap` CSS.
   Composer mention GUI tests must use Rust-shaped `ProfileState.users` member
   profiles for autocomplete candidates. React may render the popover/pills and
   pass a typed `MentionIntent`, but it must not synthesize Matrix
   `m.mentions`, formatted HTML, slash semantics, or fallback send behavior.
   Timeline mention pills are display-only decoration over Rust-owned
   timeline body/profile snapshots; React must not infer mention semantics from
   rendered text.
   Room-management GUI tests must render room settings, avatar URL, member
   actions, and role editors from `AppState.room_management.settings`,
   including the room-scoped `members` projection with Rust-projected
   `display_label`, power levels, and roles. React must not use the global
   profile cache as the room member list, recompute alias precedence from
   `local_aliases`, locally change role labels after a select change, or
   locally remove a member row after kick/ban; the Rust reducer owns those
   snapshot transitions. Linux room-management GUI QA output is limited to
   private-data-free tokens and must not print room/user IDs, room
   names/topics, avatar URLs, moderation reasons, or raw SDK errors.
   Activity GUI tests must render Rust-shaped `AppState.activity` Recent and
   Unread streams. React may switch tabs, request pagination, open focused
   context, and dispatch `mark_activity_read`, but it must not sort rows,
   filter rows, infer unread membership, clear unread state, or remove rows
   until a later Rust-owned snapshot does so. Linux Activity GUI QA output is
   limited to private-data-free tokens and must not print room IDs, event IDs,
   message bodies, pagination cursors, or raw SDK errors.
   Settings/Security GUI tests must render Rust-shaped
   `AppState.local_encryption` snapshots and Rust-owned platform profiles.
   React may display the coarse health state, show recovery/reset affordances,
   and dispatch `probe_local_encryption_health` / `reset_local_data`, but it
   must not read OS/keyring errors, infer fail-open behavior, locally repair
   health after a click, or clean stores through React-local logout/cleanup.
   GUI-only tooltips may own hover/focus timing, placement, and Escape
   dismissal in React only when the visible label comes from an existing
   Rust-owned snapshot field. Styled reusable tooltips must use
   `role="tooltip"` and `aria-describedby`; do not use native `title=` for
   product surfaces that need deterministic headless coverage.
   Fixed-format GUI geometry such as rails, icon buttons, badges, avatars,
   counters, and tooltip offsets should use named CSS custom properties or
   existing design tokens. Hard-coded `px` values are acceptable only behind a
   named token for deliberately fixed controls; avoid scattered `px` literals
   in TSX presentation code. Repeated React icon sizes, including Lucide
   `size` props, must be centralized in a local constant map.
1. Never drive login or any credential entry by fixed window-relative
   coordinates (a 2026-06-12 run typed a password into the username field).
   Use the FIFO credential path.
2. Never use `Cmd+Q` to stop the app from automation; focus slips can send
   it to the controlling agent. Use the script's process-group cleanup.
3. Resolve processes as `first process whose name is <variable>` in
   AppleScript; check both the dev process name (`matrix-desktop-app`) and
   the product title (`matrix-desktop`).
4. First-run GUI smoke sets `MATRIX_DESKTOP_SKIP_SAVED_SESSIONS=1`;
   real-login smoke additionally sets
   `MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE=1`.
5. Keep the strict `timeline_items > 0` release signal; use
   `--allow-empty-timeline` only for sparse test accounts validating
   login/room-list/panel automation.

Operational setup (Accessibility/Automation/Screen Recording permissions,
PTY handling, prompt line order) is documented in `AGENTS.md`.

## Build, Dependencies, QA Gates

0. New Matrix behavior is implemented and verified headless-first: it lands
   in `matrix-desktop-core`, is exercised via `CoreCommand`/`CoreEvent`
   against local Conduit/Tuwunel homeserver QA, and only then is wired into
   Tauri/React. GUI-first Matrix behavior is prohibited.
1. The vendored `matrix-rust-sdk` is consumed via path/`[patch]`
   dependencies, preserving upstream structure for upstreaming patches.
   Direct ports from Element X code preserve upstream license and copyright
   notices.
   Patches to the vendored SDK are limited to what is indispensable: a
   change is allowed only when the need cannot be met through the SDK's
   public API or a wrapper on our side. Each patch must be minimal
   (prefer additive accessors over behavioral changes), recorded in
   `docs/upstream/matrix-rust-sdk-feedback.md` with rationale and
   upstreaming intent, and reviewed at phase exit. In this repo the actual
   deltas live on the `github.com/shinaoka/matrix-rust-sdk-work`
   submodule branch (`shinaoka/search-ngram`), and local comments should
   point at the patch surface.
   Convenience patches are rejected; every patch increases the cost of
   tracking upstream.
2. Local homeserver toolchain caveats (Conduit/Tuwunel install flags such as
   `RUMA_UNSTABLE_EXHAUSTIVE_TYPES=1`, macOS `--no-default-features`) are
   tracked in `AGENTS.md` and the QA scripts, not hand-run.
3. Required local gates before merge: crate tests (`matrix-desktop-state`,
   `-auth`, `-core`), frontend tests + typecheck, and
   `qa:headless-local -- --server=both`.
4. Real homeserver QA is a release/preflight gate (network + approved
   credentials), not an every-CI gate.
   It is also required before GUI-level confidence claims and after changes
   that affect login, recovery, sync, encrypted restore, search, room cleanup,
   or logout.
5. Production Tauri paths must not execute fixture-backend behavior;
   `matrix-desktop-backend` is dev/demo only.
6. Core crates stay platform-portable (a future browser/wasm target must not
   be precluded): no Tauri/OS/filesystem types in `CoreCommand`/`CoreEvent`/
   `AppStateSnapshot`; task spawn and timers via executor abstractions, not
   direct `tokio::spawn`/`tokio::time` in actor logic; `keyring`, paths, and
   store config only behind `StoreActor`/adapter ports;
   `matrix-desktop-state` and `matrix-desktop-search` must compile for
   `wasm32-unknown-unknown`. See Platform Portability in
   `docs/architecture/overview.md`.
7. Japanese/CJK product semantics remain Rust-owned and platform-portable.
   Catalog completeness is tested in `apps/desktop/src/i18n`, but CJK
   normalization, display sort keys, search query variants, and highlight
   offsets live in `matrix-desktop-state`, `matrix-desktop-search`, and
   `matrix-desktop-core`. React may render the resolved catalog and Rust-owned
   ordering only; it must not compute local CJK normalization, collation, or
   highlight repair.
8. CJK GUI fitting is CSS-owned presentation. Shell/timeline/search surfaces
   that render Rust-owned CJK/user text must keep the text unchanged and use
   reviewed CSS contracts for strict line breaking, normal word breaking,
   disabled hyphenation, logical spacing, wrapping, and ellipsis instead of
   JavaScript text rewriting.

## Documentation

1. `REPOSITORY_RULES.md` is the root durable rule book for this repository.
   This document is the detailed policy extension for the domains it covers.
2. `docs/architecture/overview.md` is the long-term blueprint. Dated specs
   and plans implement it; when implementation reveals a design problem,
   amend the overview first.
3. Durable rules discovered during operations are promoted from `AGENTS.md`
   into `REPOSITORY_RULES.md` or this document; AGENTS.md keeps the
   troubleshooting detail.
4. Docs, examples, and fixtures use synthetic data only (see Secrets rules).
5. State-machine diagrams are normative. Every reducer state machine in
   `reduce(AppState, AppAction)` — its states, transitions, and guards — is
   documented as a Mermaid `stateDiagram-v2` in
   `docs/architecture/state-machine.md`. A change to a reducer state machine
   must update the matching diagram and its guard notes in the same change; a
   transition present in code but not in the diagram (or the reverse) is a
   defect. Phase-exit docs-sync verifies diagram↔reducer agreement. Design new
   state transitions as explicit guarded state machines (events distinct from
   states, invalid/stale inputs rejected), not ad-hoc field assignments.
   Actor-owned product state that is exposed through DTOs or guarded commands
   follows the same rule: add the diagram/guard table with the implementation
   change, even when the state is not stored directly in `AppState`.
6. For umbrella issue work, each child issue completion must record
   implementation discoveries in the right place: durable architecture/rule
   changes in `docs/architecture/`, `REPOSITORY_RULES.md`, or this document;
   operational setup/failure notes in `AGENTS.md`; and QA scenario contracts in
   `docs/qa/`. Closing an issue without syncing the learned rule is a process
   defect.
