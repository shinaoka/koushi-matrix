# Engineering Rules

Status: normative detailed policy. This document extends the root durable rules
in [REPOSITORY_RULES.md](../../REPOSITORY_RULES.md) with concrete policy for
secrets, logging, QA automation, async/runtime behavior, GUI automation, and
build gates. AGENTS.md remains the operational how-to (permissions, install
caveats, recovery steps); durable rules discovered there are promoted to
REPOSITORY_RULES.md or this document.

Last amended: 2026-07-24.

## Design Simplicity

1. Add a guard, retry, fallback, duplicate state, or test hook only for a
   reproduced failure or named invariant.
2. Give each lifecycle state machine one owner and one explicit state model; do
   not synchronize parallel booleans.
3. When an artificial failure mechanism creates a boundary problem, remove it
   instead of adding boundary handling around it.

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
   environment (`KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1`,
   `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR`) instead.
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
   Link-preview global defaults are Rust-owned display settings with separate
   preferences for non-encrypted rooms (`url_previews_enabled`, default true)
   and encrypted rooms (`encrypted_url_previews_enabled`, default false). The
   encrypted-room default remains privacy-conservative, but it is an explicit
   user setting rather than a hidden UI-only special case.
   Per-room URL-preview overrides are not settings-file data because the key is
   a Matrix room identifier. They live in Rust-owned non-persisted
   `AppState.link_preview_settings.room_overrides`, are changed only through a
   typed room-override command, and are exposed to React as current snapshot
   state for rendering. `SettingsValues` / `SettingsPatch` must not gain
   room-, event-, or user-id keyed maps; if a durable per-identity preference is
   required later, define a dedicated privacy-reviewed local store instead of
   extending general settings JSON.
   Unsent composer drafts are not settings. If product behavior persists them
   across restart, they must be account-scoped encrypted local data owned by
   `StoreActor`/`AppActor`, derived from a dedicated local-unlock-secret key
   domain, debounced, size-bounded, and excluded from general settings JSON,
   logs, QA tokens, and full webview snapshots. React may render only the active
   composer DTO and dispatch typed draft commands. Accepted sends must advance
   a target-keyed causal revision, persisting an empty-draft tombstone only
   when no newer target draft exists; newer content must be preserved at the
   advanced revision. Bounded persistence must prioritize targets with
   non-empty draft content before revision-only tombstones so fences cannot
   evict the drafts they protect. Debounce timers are target-keyed: editing one
   room or thread must not cancel another target's pending encrypted-store
   write. Every debounced draft write and every operation that can accept a
   composer draft must capture its complete account owner (homeserver, user,
   and device). Account
   transitions cancel pending timers and invalidate late webview completions,
   while Tauri, AppActor, and AccountActor revalidate the captured owner
   against the ready session before routing, changing, or persisting draft
   state. The AccountActor check is the ordered final barrier after any
   account-switch message already queued in its mailbox.
   Room/thread target identity alone is insufficient because two accounts may
   share it.
   Submission commands that do not have a correlated submission outcome
   event (currently scheduled and prepared-upload sends) must wait for and
   return the authoritative accepted target revision; a merely enqueued command
   plus the current active-pane snapshot is not acceptance evidence. An
   accepted clear must also advance the target's IME synchronization key:
   composition-owned controls correctly ignore unacknowledged external values,
   so a render alone cannot distinguish an authoritative clear from a stale
   snapshot. Timer cancellation or a second racing empty save is not a
   correctness fence.
   `ComposerDraftRevision` is a checked Rust `u128` and an opaque canonical
   decimal string on every snapshot and IPC boundary. JavaScript `number`
   conversion, wrapping, and saturation are forbidden. Rust advances
   `last_accepted_clear_revision` only when an accepted operation actually
   clears current content; ordinary persistence and accepted preservation of
   newer input do not change it.
   Empty revision history is retained in lifecycle order. Only empty, inactive,
   zero-touch-lease targets are quiescent tombstones; retain 128 main and 256 thread
   tombstones. The live bound is protected targets plus those quotas.
   Non-empty, active, debounce/IPC/submission/schedule/upload-pending,
   command-pending, and touch-leased targets are protected and cannot be
   eviction victims. Quiescence waits only for the final touching
   activation/command lease. Store-pending persistence holds are non-touching
   collector guards: they may coexist with a remembered quiescent LRU position,
   block that target as a victim, and preserve its age and persisted order. A
   hold must not by itself enter the protected-empty persistence bucket or
   consume the eligible-quiescent quota. Collector scans skip held entries in
   place. A touch-to-store-pending transition enqueues newest once; hold release
   does not refresh age. Same-key replacement
   subtracts only the superseded pending hold contribution while projecting,
   acquires new holds before swap, and keeps the old pending save if admission
   fails. Every revision-bearing producer acquires the exact
   account/target/renderer-generation lease before it schedules or enters Core.
   Lease admission/release and victim selection are serialized, and a retired
   generation cannot deliver or recreate collected state. Diagnostics expose
   counts and coarse lifecycle outcomes only, never bodies, Matrix identifiers,
   revisions, leases, filesystem paths, or raw errors.
   Scheduled-send message bodies are also future unsent message content. The
   Rust state machine owns the queue, capability, cancellation, reschedule, and
   due-time dispatch semantics. Full scheduled-send backing state must not be
   serialized to the webview; only the selected-room projection may cross as
   visible UI state. Normal `Debug`, QA logs, issue evidence, and window-title
   tokens must redact scheduled bodies, room ids, server delayed-event handles,
   and transaction ids. MSC4140 support is detected in Rust through the SDK
   `/versions` unstable feature set, and server delayed-event create/cancel /
   reschedule operations are AccountActor-owned SDK/Ruma side effects. React
   must not run a local send-later timer or call raw Matrix delayed-event APIs.
11. E2EE trust diagnostics are kind-only. Verification, cross-signing,
   key-backup, and identity-reset commands/events may expose structured state to
   the UI, but normal `Debug`, QA logs, and window-title tokens must redact
   account keys, verification target user/device IDs, backup versions, raw SDK
   errors, identity-reset auth details beyond UIAA/OAuth/unknown, and all key
   material. Key-backup restore progress/copy must say joined-room hydration
   when that is the implemented scope; it must not imply exhaustive backup-wide
   restore. `KeyBackupRestoreSummary.scope` stays `JoinedRooms` for the MVP
   path unless a later upstream/public API decision explicitly broadens it.
   Secure-backup recovery keys may be produced by the SDK, but the product must
   not place the key in reducer state, Tauri DTO snapshots, React state, logs,
   QA tokens, screenshots, or issue comments. Desktop recovery-key delivery
   writes through a Rust/Tauri native artifact path and reports only a boolean
   or coarse status.
   Authentication alone is not session admission. Until the SDK
   current-device verification state is authoritatively `Verified`, the SDK
   session is AccountActor-owned and quarantined: no normal sync children,
   rooms/timelines/search, drafts/navigation/scheduled sends, notifications,
   native attention, main shell, active saved-session publication, or ordinary
   Matrix command may be exposed. Restricted crypto sync is internal and may
   process only trust-discovery, recovery, device-list/key-query, and to-device
   verification traffic. Recovery/SAS/bootstrap UI completion never substitutes
   for the authoritative trust observation. Provisional credentials are not
   persisted; rejection/logout erases local keyed stores and attempts server
   logout before projecting `SignedOut`.
   Gate method/target discovery exposes no additional raw device identifiers;
   current-device SAS selection uses actor-owned opaque handles. A generated
   recovery key is delivered only to a user-selected native destination and
   confirmed through coarse Rust state, never displayed or returned to the
   WebView. Current-device admission remains separate from peer trust:
   unverified eligible peer devices do not require a normal-mode send prompt,
   while blocked devices and cryptographic integrity/key-mismatch failures
   remain hard failures.
12. Credential-store health diagnostics are kind-only. Public state may report
   only `unknown`, `healthy`, `unavailable`, `locked_or_inaccessible`,
   `missing_credential`, or `reset_required`, with optional private-data-free
   remediation hints. Raw OS/keyring errors, local paths, account identifiers,
   key labels, local unlock secrets, SDK/search keys, and recovery material must
   stay inside `StoreActor`/adapter diagnostics gated for debug/test. OS
   credential calls go through the `koushi-key` credential backend
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
   Avatar MXC URIs, thumbnail state, and user-room avatar associations are
   account-scoped sensitive metadata. Debug, logs, tests, QA tokens, fixtures,
   and issue evidence must redact real values.
   Persistent automatic avatar bytes belong only to the account-keyed Matrix
   SDK SQLite media store. A separated SDK `cache_path` must retain the same
   required `MatrixClientStoreKey`, and SDK media retention is the sole
   persistent retention policy. Koushi may keep decrypted renderable bytes only
   in the account/session-scoped in-memory `koushi-thumbnail://` cache. A
   separate plaintext avatar/link-preview cache and automatic `file://` URLs are
   prohibited; legacy plaintext directories are cleanup-only.
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
3. Public boundary types (`CoreCommand`, `CoreEvent`, snapshot DTOs, Tauri DTO
   mirrors, and shared QA payloads) must treat `Debug` as artifact-facing. Use
   derived `Debug` only when every field is safe if copied into CI logs or a
   GitHub issue. Use custom redacted `Debug` for any field that may contain
   message bodies, search snippets/queries, attachment names, room/user/event/
   transaction IDs, local filesystem paths, URLs from rooms, raw SDK errors, or
   secrets. Redacted `Debug` may expose variant names, request ids, enum kinds,
   booleans, counts, lengths, and placeholders such as `Snippet(..)` or
   `RoomId(..)`.
4. QA asserts on `CoreEvent` and `AppStateSnapshot`, never on log output.
5. Real-account and real-homeserver QA output is tokenized before it becomes an
   artifact. Captured logs must not contain raw Matrix IDs, event IDs,
   transaction IDs, user IDs, message bodies, search queries, local paths, or raw
   SDK errors. Producers should avoid formatting those values; wrappers must not
   write unredacted stdout/stderr and only then discover a leak.

## Async and Runtime

1. No fixed sleeps in QA or product code waiting for Matrix effects — wait
   on events with timeouts. A multi-event waiter owns one monotonic absolute
   deadline for the whole operation and passes that same deadline through
   every loop iteration and nested phase. Never recreate a relative timeout
   around each received event: an unrelated continuous sync stream would then
   postpone failure forever. Timeout diagnostics identify the typed phase and
   private-data-free observed state needed to locate the missing transition.
2. Store-backed Matrix SDK clients must be dropped while a Tokio runtime
   context is entered; otherwise `deadpool-runtime` panics with
   "there is no reactor running".
3. Every spawned background task and subscription has an owner responsible
   for cancelling it (unsubscribe, account shutdown, app shutdown). No
   unbounded maps of live subscriptions.
4. Timeline scrollback is a split contract: core emits diffs and pagination
   state; React owns DOM anchoring. Product code must not issue automatic
   pagination loops before the previous diff has rendered and anchor
   restoration has completed. Backfill eligibility must come from one pure
   state evaluation over demand and blockers, and every state transition that
   can remove a blocker must explicitly schedule another evaluation. A prepend
   diff alone does not end the request epoch. `Paginating`, a front insertion,
   or a replacement `Reset` is acceptance evidence. An accepted `Idle` terminal and an expected oldest-edge projection
   may arrive in either order, so both must be observed before release. Core must
   report whether the SDK call changed the observable oldest edge; a confirmed
   no-prepend page settles on the terminal alone, and the terminal must be emitted
   only after actor task ownership is released. An unaccepted `Idle`, failure, or
   transport rejection releases the epoch but waits for its owning release
   condition instead of retrying itself. General failures may use a later
   external transition; an admission-rejected `Idle` is fenced until
   `GapRepairReleased`. `GapPositionsUpdated` is a projection
   wake but may be followed immediately by active repair; `GapRepairReleased`
   is the post-terminal wake that proves no queued or active gap work remains.
   End/resync releases directly. Programmatic
   scroll echoes are not genuine top-scroll demand. Do not add polling,
   fixed-delay retries, or a user-scroll latch to compensate for a missing
   transition.
   Room gap repair follows the same fence: one
   actor may own at most one unacknowledged repair projection batch, and it may
   continue only after the SDK's final actor/repair/publication tag is mapped
   to an exact desktop batch and receives a matching
   actor/timeline/repair/batch post-layout ACK. A rejected ACK transport call
   remains retryable; a gap-only cache chunk or fully filtered publication
   creates no render fence, while aggregation-only work must publish an
   observable tagged barrier. A resync marker suspends acknowledgements until
   the matching `InitialItems` replay has rendered. Observable settlement
   waits must be bounded so a lag-dropped SDK update becomes retryable failure,
   never a permanent repair owner.
   `ObserveViewport` may wake automatic gap inspection only when the selected
   projected gap candidate changes (viewport-intersecting first, otherwise
   nearest the live edge). An unchanged candidate is idle; a changed candidate
   remains queued across active work and projection/render ACK fences.
   Candidate-driven automatic repair keeps a zero cached-chunk budget.
   Room-entry live-edge repair is a separate bounded intent. On SyncService and
   legacy sync it must wait for committed backend provenance. When the current
   response contains the room, repair may select only the opaque persisted gap
   introduced by that response; an explicit no-update/no-gap closes the intent.
   Legacy event-cache publication also supplies a global response-commit fence
   after all room topology mutation. Only when that fence proves an active room
   was omitted from the incremental response may repair inspect authoritative
   persisted topology and select its newest gap as one bounded live-edge chain.
   It must not acquire repair ownership while provenance is pending, reuse a
   baseline observation for an empty response, infer omission from a timeout or
   pre-commit broadcast, or select any older persisted gap. Routing must match
   backend instance epoch, room key, actor generation, and backend-local
   response or subscription generation. Timeline build, initial projection,
   and ACK remain non-blocking while provenance is pending. A stale descriptor
   permits one authoritative re-inspection, then closes and clears that
   checkpoint so a later committed response can be admitted, but never permits
   arbitrary gap selection. While that bounded
   attempt is unsettled, retain the latest newer checkpoint separately and
   promote it immediately after close/admission; delivery is at-most-once and
   another room update may never arrive to replay it. SyncService response
   identity combines subscription generation with the room event-cache
   observation sequence; one subscription generation spans many responses.
   The SDK serializes process-local response sequence assignment and room
   update publication in one critical section, then advances the retained
   global commit fence only after event-cache handling completes. Legacy
   promotion carries the
   first successful response sequence
   and accepts only observations from that response or a later one; retained
   values from an earlier backend response and duplicate per-room commit
   sequences are stale by construction.
   Relay and render settlement fences are also bounded actor state. Overflow,
   an authoritative replacement that cannot contain the correlation, a
   generation replacement, or a matching deadline clears only the obsolete
   fence, retains the highest-priority queued trigger, and resumes through
   authoritative resync/re-inspection. Stale deadlines are ignored.
   The selected repair reveals at most one cached chunk per request, stops on
   unchanged topology or zero progress, and has a small per-generation batch
   ceiling. Repairing a projected descriptor must preserve this intent; only a
   joined/start-reached live-edge fallback may downgrade its continuation to
   ordinary automatic repair. It must not broaden viewport-driven repair or
   log its target.
   Underfilled-initial pagination must use the settled height model and virtual
   range; a transient virtual DOM `scrollHeight` is not proof that a timeline
   with canonical overflow needs another page.
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
   correlated operation failure that leaves no stuck pending state. Once a
   command accepts a user-intent submission, terminal observation and
   request/submission correlation must be owned by a component whose lifetime
   spans every replaceable presentation actor involved in that operation. An
   unsubscribe, room switch, actor crash, or resubscribe must not discard the
   accepted operation. Lossy observer lag becomes an immediate, correlated,
   private-data-safe failure rather than a fixed-delay wait. Shutdown must drain
   accepted operation futures while terminal admission remains live,
   synchronously drop cooperative futures remaining at the shared deadline,
   then drop terminal observers, drain their reliable handoff, and only then
   acknowledge the owning actor's teardown. The enqueue future itself shares
   that stable owner: a replaceable presentation actor must not await it, and
   dropping a raw task handle must not detach it. Prefer owner-polled boxed
   futures when unexpected synchronous teardown is required; isolate their
   panics with a fail-closed unwind boundary. Such futures must obey the async
   contract that every poll returns. After admission, the owner must drive the
   specific accepted future through its reducer permit to a one-shot signal at
   the start of payload-specific preflight before returning to unrelated command
   traffic; polling one arbitrary ready future is not causal proof for the new
   worker. Payload preflight may suspend, so this signal does not promise
   cross-worker SDK enqueue order. Later FIFO retry scheduling remains SDK-owned.
   An unexpected stable-owner drop closes
   terminal admission, synchronously drops operation futures, then drops the
   terminal observer. A waiter that combines an authoritative snapshot
   with an event stream treats the stream only as a wake signal: check the
   predicate before blocking and perform one final authoritative observation
   after timeout, closure, or lag before reporting failure. That final check
   uses the same monotonic absolute deadline and must not create a fresh wait
   interval. This prevents a commit that lands immediately after the last
   broadcast from becoming a false timeout. When a media enqueue succeeds, its
   manager-owned worker publishes `MediaSendQueued` before binding the SDK
   transaction to any terminal already retained by the completion coordinator.
   A pre-bind `Sent`/failure/cancel observation must never overtake the queue
   acknowledgement; select-branch priority is not an ordering fence.
10. If a reducer returns an `AppEffect` that matters in production, the
   production runtime executes it or the behavior is redesigned as an explicit
   `CoreCommand`/actor command. Discarding such effects is allowed only for
   fixture/demo effects that are documented as non-production.
11. Core async channels are sized for large-account (100+ room) sync bursts via
   named capacity constants (`COMMAND_INBOX_CAPACITY`,
   `ACTOR_MESSAGE_QUEUE_CAPACITY`, `ACTION_QUEUE_CAPACITY`,
   `EVENT_QUEUE_CAPACITY`), never small magic literals (16/64/256) — a too-small
   core inbox is invisible to small-account CI and only fails on real accounts.
   This makes rule 9 concrete: drop-on-full `try_send` (with an ignored `Err`)
   is forbidden for one-shot, non-re-projected actions — navigation
   (`SelectRoom`/`SelectSpace`/`ReorderSpaces`) and command-result projections —
   which MUST use reliable `send().await`. `try_send` is permitted only for
   high-frequency data re-projected on the next sync (room-list snapshots),
   where a dropped update self-heals. Silently dropping `SelectRoom` under a
   saturated `ACTION_QUEUE_CAPACITY` inbox was the large-account "room selection
   did not complete" / blank-timeline / unloaded-members regression; it passed
   every small-account headless lane.
12. User-intent commands resolve to a correlated, observable terminal outcome —
   never a silent no-op. A foreground one-shot command
   (account restore/login/logout, `SelectRoom`/`SelectSpace`, send/edit/redact,
   pin/unpin, mark-read, invite accept/decline, start DM, join/leave) carries
   its `request_id` end to end and
   settles as exactly one of `committed`, `benign-noop(reason)`, or
   `failed-noop(reason)`. A reducer that returns `Vec::new()` for such a command
   (room absent from `state.rooms`, session not ready) MUST surface that as a
   correlated outcome, and the command waiter returns the specific reason, never
   a generic "did not complete" timeout. On the submit → route → project →
   reduce → settle path, discarding a failure with `let _ =`,
   `unwrap_or_default()`, `.ok()`, or a catch-all `_ => {}` is forbidden. The
   #116 blocker was three stacked silent no-ops (`handle_select_room`
   `Vec::new()` → empty `build_state_delta` → neither `StateDelta` nor
   `StateChanged` → opaque 10s timeout) that collapsed four distinct failure
   modes into one undiagnosable string. Reducer projection is also an admission
   boundary: if the projected action is rejected in the current state, emit
   exactly one correlated typed failure and do not route the command. Separate
   operation-event and snapshot lanes may arrive in either order; a follow-up
   that depends on authoritative state must observe both its terminal and the
   required snapshot state before submitting.
13. Telemetry and diagnostics travel on a dedicated lane, not the product-state
   channel. Lifecycle/diagnostic events such as `CoreEvent::IntentLifecycle` are
   never folded into product `StateDelta`/`StateChanged`, never drive product
   state in the WebView, and never cause a user-intent to be dropped when a
   telemetry buffer saturates. A UI must not infer product success from a
   telemetry event; success comes from the projected snapshot.
14. Each submitted user-intent resolves exactly once, in submission order.
   Request-to-intent correlation state is drained per submission, so concurrent
   intents for the same target each receive their own terminal outcome. A
   single-value, target-keyed correlation map that overwrites an in-flight
   `request_id` — leaving the older request unsettled until its timeout — is
   forbidden; use a per-target FIFO queue or carry the `request_id` on the
   projected action. Ordered timeline shutdown gives the entire manager-owned
   enqueue-worker set one absolute, count-independent five-second graceful
   deadline while terminal ingress and its global observer remain live. On
   expiry, synchronously drop remaining manager-polled futures before dropping
   the observer and draining terminal ingress. A future that blocks inside
   `poll` violates the async contract and is not a supported worker shape;
   production enqueue futures must return `Pending` instead. Per-worker graceful timeouts remain forbidden because they
   make shutdown latency scale with worker count.
15. Every user-intent command class ships a real-account-shaped scale stress
   test (about 110 rooms / 5 spaces / 57 DMs) that drives the real command path
   and asserts the lifecycle invariant — every submission reaches a terminal
   outcome, none vanishes. #116 stayed invisible because every lane used small
   accounts; the runtime and reducer are exonerated for a scale bug only by such
   a test, and a transient room-list projection that drops a known-joined room
   is caught here, not in CI.
16. Verified-session admission has one classic-sync owner at a time. An
   optional restricted provisional lane must be cancelled and joined before
   Ready is projected, and normal SyncActor ownership begins only after the
   projection acknowledgement. Do not add an admission-only full-state sync,
   share an incompatible restricted token with normal sync, or retain a
   restricted lane beside normal sync. A manual `SyncOnce` is permitted only
   when no continuous or restricted owner is active for that SDK client; QA
   must await typed event/state conditions instead of overlapping cursor
   owners. Enforce that rule at both routing boundaries: reject before
   `SyncActor` routing while `AccountActor` owns restricted sync, and reject
   before the SDK one-shot call while a continuous owner or any of its task,
   service, backend, or stop-handle artifacts remains. Network catch-up failure
   does not revoke authoritative Verified trust; it is normal Ready-shell sync
   state. Replayed verification starts are
   idempotent: the SDK and owning actor adopt one SAS continuation per flow and
   never replace its handle, observer, timeout, or acceptance on replay.
   A QA device-readiness checkpoint may use the `qa-bin`-only read-only
   user-key refresh plus exact-device acknowledgement. It must not use a
   verification request/cancellation pair as a probe or expose the target in
   diagnostics. That receiver acknowledgement must precede a fresh device's
   verification send, and paired progress must wait on either event stream with
   one absolute deadline rather than fixed-interval polling. If a valid
   to-device verification request arrives before sender-device discovery, the
   crypto boundary retains a bounded, timestamp-valid, sender/flow-deduplicated
   pending entry, schedules only the existing key-query owner, and revalidates
   it after matching keys commit. Missing, expired, duplicate, and capacity
   cases must be tested. A failed schedule or replay must retain the original
   FIFO slot, while a successfully scheduled duplicate must not create another
   query. Recovery must not add a sync owner, blind resend, fixed sleep, raw SDK
   error, or protocol identifier to diagnostics. Successful deferred
   materialization and normal materialization queue the same stable SDK handle
   in one typed lease stream. Unknown pending entries, publications, subscriber
   generation, and active head claim share one owner lock and one total bound of
   32; an active lease consumes its slot, commit pops it, and drop releases it in
   place. Pending replay changes its existing slot into a publication under the
   same lock. Generation check and claim are one linearization point. Do not
   nest this owner lock with the request cache or hold it across async/fallible
   work. Capacity is strict FIFO: never evict an existing pending entry,
   publication, or active lease to admit a newcomer. At capacity, explicitly
   terminally cancel a newly materialized request and queue its protocol cancel;
   do not retain or schedule a key query for a newest unknown-device request.
   Never rely on server redelivery after cursor advancement for a materialized
   request. Same-flow insertion must not upgrade unrelated cached provenance.
   Koushi consumes only the typed to-device stream and commits after its product
   channel accepts an actionable handle; terminal heads commit immediately.
   Generic raw SDK handlers are independent compatibility fanout and may repeat
   after partial cancellation. Treat transport as at-least-once with stable
   `(sender, flow_id)` identity, and enforce product idempotence in
   `AccountActor` with the full `VerificationTarget` plus flow id: only the same
   peer, device, and flow is a no-op. A different target or flow, including a
   colliding flow id from another peer or device, is explicitly cancelled as a
   conflict. Treat an active own-user verification, including its pre-SAS
   request phase, as a conflict with every incoming request: it owns the shared
   continuation/observer slots and has no typed incoming identity for replay
   matching, so cancellation must precede handle or observer adoption. Replayed
   SAS adoption is also a no-op; distinct conflicting SAS handles must be
   rejected without raw error data. Do not
   extend an existing public exhaustive SDK enum to carry internal dispatch
   state. Replacing a session or observer requires stop-and-join of the old
   Account forwarder and SDK typed-lease worker before installing the new owner.
   Remove the old room handler before replacement; SDK sync dispatch owns and
   awaits any callback future already dispatched, so old `SyncActor`
   stop-and-join is the callback settlement barrier. Carry a
   dedicated session generation on observer-to-actor messages and reject stale
   or sessionless work before adoption. A mailbox send must race stop with stop
   priority; join uses a bounded timeout, then aborts and awaits settlement.
   Crypto/client delivery `Debug` output must not expose request, owner, client,
   account, device, transaction, or identity-key internals. Model pending query
   ownership explicitly rather than with one scheduling boolean. Acquire a
   response RAII claim before key-response processing; cancellation/error before
   or after durable commit returns only that token's claimed entries to
   `NeedsQuery`, while normal still-missing completion explicitly enters
   `WaitingForExternalUpdate`. Never perform a sender-wide retry reset: a newer
   same-sender response may own a distinct response or replay claim.
   Generated key queries must retain an exact, stable
   `request_id -> covered users` mapping, dirty-state sequence, and request until
   all outer response-associated verification recovery succeeds. Cancellation
   or recovery failure must preserve the original mapping and request ID for
   retry. Repeated, concurrent, or cancelled request collection must not clear
   or replace another in-flight mapping; reuse existing requests and create
   requests only for uncovered dirty users. Revalidate and register the final
   dirty snapshot while response cleanup is excluded; never insert from a stale
   pre-await snapshot. Serialize complete same-ID response-associated processing
   with the stable entry's async gate, awaited without any registry or store
   guard. Revalidate the pointer-matched entry after acquisition before taking
   coverage or claiming recovery. Success consumes that entry; failure or
   cancellation preserves it for the next waiter, and a waiter after successful
   consumption has no metadata obligation. Different request IDs must remain
   concurrent. Scope verification
   recovery to exactly the union of the response `device_keys`
   users and that request's covered users, including failure-only responses
   with no returned keys. Overlapping committed responses require a per-entry
   generation handoff: a replay owner must consume a deferred newer generation
   before entering `WaitingForExternalUpdate`. Do not log request IDs, covered
   users, sender/device/flow IDs, or raw response/error data on these paths.
   Multi-stage QA must thread participant ownership through typed helper inputs.
   A later stage borrows an already-live role without creating or cleaning up a
   duplicate; a focused stage may own one participant when none exists. It must
   not hard-code bootstrap for an initialized account, manufacture a duplicate
   device, stop another valid owner to avoid a protocol race, guess the gate
   from timing, or hide a setup mismatch with retries or a longer timeout.
   Ownership starts before fallible login submission and records enough phase
   to clean an unsubmitted runtime, a submitted provisional session, or a keyed
   logged-in session without guessing. Error paths attempt cleanup for every
   owned participant, preserve borrowed participants for their outer owner, and
   order logout confirmation before connection drop and runtime shutdown.
   Logout confirmation follows the authoritative-snapshot waiter rule above,
   including a final `SignedOut` read after timeout, lag, or closure under the
   original deadline. Failure-injection acceptance drives each ownership phase
   through the behavioral cleanup boundary and asserts logout/barrier/drop/
   shutdown order plus continuation to the remaining owners. Source-text
   inventory guards alone are not evidence.
   A verification-only filtered sync must use `NoToken` and the SDK's opt-in
   non-persisting-token mode. It still saves crypto/device/to-device state and
   invokes handlers, but it must not write its filter-scoped `next_batch` as the
   global room cursor. Do not compensate after the write with a process-local or
   separately persisted taint ledger. A fresh store stays tokenless; a restored
   canonical cursor survives restricted sync, runtime destruction, account
   switching, and store reopen and remains incremental for normal sync. Never
   infer cursor validity from a server family, an empty room list, elapsed time,
   or a successful crypto gate.
17. A bounded materialized view's diff stream is not proof that every source
   domain represented beside that view is unchanged. If authoritative state can
   commit outside the bounded window, the owning observer must consume a
   post-commit source signal as a wake-up and reconcile only the affected
   fingerprint. Coalesce queued wakes, perform one bounded reconciliation after
   lag, and keep auxiliary-channel closure from killing the authoritative
   observer. The auxiliary signal must not create a second network/sync owner,
   and high-frequency unrelated updates must be proven not to trigger full-view
   normalization.
18. A protocol version advertisement is not proof that the operation semantics
   needed by the product are complete. Probe the narrow authenticated behavior
   before creating its authoritative owner. The disposable authenticated probe
   receives no refresh token, so automatic refresh is impossible/disabled, and
   request retries are disabled; its single end-to-end two-second deadline
   covers disposable-client setup plus one transport request. Treat omitted
   required response structure, typed/malformed failure, and deadline expiry as unsupported;
   discard any cursor or product payload returned by the preflight. Capability
   checks must not fingerprint server families or become a second polling/sync
   owner. A non-authoritative authenticated probe must isolate session-change
   delivery, access-token-expiry state, token refresh, rotating refresh
   credentials, and persistent stores from the authoritative client. Supply no
   refresh token to a disposable probe client. Probe failure is a
   backend-selection fact and must not itself cause a product
   authentication-state transition. Cover success, omission, malformed/error,
   and timeout, plus behavioral proof that `M_UNKNOWN_TOKEN` causes zero refresh
   calls, no authoritative session-change/token mutation, and fail-closed
   `LegacySync` selection.
19. Foreground room navigation must not wait behind ordinary actor mailboxes or
   network/filesystem side effects. Reducer commit emits the exactly-once
   intent terminal; projection admission uses an owner-stable latest-value slot
   ordered by an internal monotonic generation and a bounded wake. The manager
   polls that wake before ordinary completions. Actor lifecycle controls use a
   separate bounded control lane, one absolute cancellation deadline, and
   generation fences so late old-room work cannot regress the active room.
   Persistence and read convergence run in owned workers and never delay the
   navigation terminal or cached projection.

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
   Bugs and product-spec changes discovered during interactive browser/GUI
   exploration must be pinned by headless coverage whenever feasible before the
   work is considered complete. Prefer unit/component tests for pure state or
   rendering contracts, and browser-headless tests for user-visible DOM,
   scrolling, context-menu, drag/drop, and mocked IPC behavior. Native GUI lanes
   may remain final smoke evidence, but they do not replace a cheap headless
   regression test unless the behavior is inherently OS-window, keychain, menu,
   notification, or WebView-native.
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
   Redacted-message visibility is also a Rust-owned display preference:
   `SettingsValues.display.hide_redacted` defaults to `true`, persists through
   the settings store, and is projected onto timeline DTOs as
   `TimelineItem.is_hidden`. React may omit rows only when this DTO flag is
   true; it must not remove redacted events from state or derive visibility
   from a React-local setting.
   Image upload compression preference is a Rust-owned media setting:
   `SettingsValues.media.image_upload_compression` defaults to `ask`,
   persists through the settings store, and is read by Tauri before building the
   upload command. React may run the browser/native pixel transform, but the
   mode, policy thresholds, selected/original variant metadata, metadata-strip
   assertion, and thumbnail-refresh assertion must cross the boundary through
   `UploadMediaRequest.compression`, not through React-local semantics.
   Received message formatting is a Rust-owned security projection:
   `TimelineItem.formatted` is sanitized from Matrix `formatted_body` before it
   crosses the WebView boundary, and carries only sanitized HTML plus derived
   plain text/code-block metadata. React must never render unsanitized server
   HTML or own sanitizer policy. TimelineView's formatted renderer is a
   presentation adapter only: it maps the Rust-owned DTO into React nodes,
   code-copy controls, search highlights, and `code_block_wrap` CSS. Ordinary
   HTML source whitespace must collapse without synthetic `br` nodes; explicit
   sanitized `br` remains a break, direct list element children remain `li`,
   inline spacing and pre/code text remain intact, and pretty/minified markup
   must have equivalent browser layout.
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
   AppleScript; check both the dev process name (`koushi-desktop`) and
   the product title (`Koushi`).
4. First-run GUI smoke sets `KOUSHI_SKIP_SAVED_SESSIONS=1`;
   real-login smoke additionally sets
   `KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1` and
   `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR`.
5. Keep the strict `timeline_items > 0` release signal; use
   `--allow-empty-timeline` only for sparse test accounts validating
   login/room-list/panel automation.

Operational setup (Accessibility/Automation/Screen Recording permissions,
PTY handling, prompt line order) is documented in `AGENTS.md`.

## Desktop Text Input And IME Safety

1. Text-entry components use `ImeTextField`, `SecureImeTextField`,
   `ImeTextArea`, or the existing externally-owned `ImeOwnedTextArea` from
   `apps/desktop/src/components/ImeTextControl.tsx`. Forms containing text
   entry use `ImeSafeForm`. Do not duplicate composition handlers in feature
   components.
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

0. New Matrix behavior is implemented and verified headless-first: it lands
   in `koushi-core`, is exercised via `CoreCommand`/`CoreEvent`
   against local Conduit/Tuwunel homeserver QA, and only then is wired into
   Tauri/React. GUI-first Matrix behavior is prohibited.
1. The vendored `matrix-rust-sdk` is consumed from one rev-pinned git
   dependency in the root workspace, preserving a reproducible dependency graph
   across branch switches. The local `vendor/matrix-rust-sdk` submodule remains
   the editable upstream-shaped checkout for preparing and reviewing SDK
   patches, but app crates must not depend on it by local path.
   Direct ports from Element X code preserve upstream license and copyright
   notices.
   Patches to the vendored SDK are limited to what is indispensable: a
   change is allowed only when the need cannot be met through the SDK's
   public API or a wrapper on our side. Each patch must be minimal
   (prefer additive accessors over behavioral changes), recorded in
   `docs/upstream/matrix-rust-sdk-feedback.md` with rationale and
   upstreaming intent, and reviewed at phase exit. In this repo the actual
   deltas live in pinned revisions of `github.com/shinaoka/matrix-rust-sdk-work`,
   and local comments should point at the patch surface.
   Convenience patches are rejected; every patch increases the cost of
   tracking upstream. Every SDK rev bump must update the root `Cargo.lock` and
   keep the guarded submodule pointer in sync with the pinned git revision.
2. Local homeserver toolchain caveats (Conduit/Tuwunel install flags such as
   `RUMA_UNSTABLE_EXHAUSTIVE_TYPES=1`, macOS `--no-default-features`) are
   tracked in `AGENTS.md` and the QA scripts, not hand-run.
3. Required local gates before merge: crate tests (`koushi-state`,
   `-auth`, `-core`), frontend tests + typecheck, and
   `qa:headless-local -- --server=both`.
4. Real homeserver QA is a release/preflight gate (network + approved
   credentials), not an every-CI gate.
   It is also required before GUI-level confidence claims and after changes
   that affect login, recovery, sync, encrypted restore, search, room cleanup,
   or logout.
5. Production Tauri paths must not execute fixture-backend behavior;
   `koushi-backend` is dev/demo only.
6. Core crates stay platform-portable (a future browser/wasm target must not
   be precluded): no Tauri/OS/filesystem types in `CoreCommand`/`CoreEvent`/
   `AppStateSnapshot`; task spawn and timers via executor abstractions, not
   direct `tokio::spawn`/`tokio::time` in actor logic; `keyring`, paths, and
   store config only behind `StoreActor`/adapter ports;
   `koushi-state` and `koushi-search` must compile for
   `wasm32-unknown-unknown`. See Platform Portability in
   `docs/architecture/overview.md`.
7. Japanese/CJK product semantics remain Rust-owned and platform-portable.
   Catalog completeness is tested in `apps/desktop/src/i18n`, but CJK
   normalization, display sort keys, search query variants, and highlight
   offsets live in `koushi-state`, `koushi-search`, and
   `koushi-core`. React may render the resolved catalog and Rust-owned
   ordering only; it must not compute local CJK normalization, collation, or
   highlight repair.
8. CJK GUI fitting is CSS-owned presentation. Shell/timeline/search surfaces
   that render Rust-owned CJK/user text must keep the text unchanged and use
   reviewed CSS contracts for strict line breaking, normal word breaking,
   disabled hyphenation, logical spacing, wrapping, and ellipsis instead of
   JavaScript text rewriting.
9. Signed distribution builds must run the platform-specific credential gate
   before packaging. A macOS signed-DMG build validates the signing identity
   and all notarization credentials without requiring unrelated Windows
   credentials; post-build signature, notarization-ticket, and platform trust
   checks remain mandatory release evidence.
10. Long-duration end-to-end and homeserver scenarios are integrated gates,
   not inner-loop probes. Complete the coherent assertion-driven scenario
   first, using compile checks, focused unit/integration tests, and bounded
   fail-fast checkpoints during implementation. Remove superseded fixture
   paths and review the finished diff before running the long scenario. Re-run
   that scenario only when its own evidence requires a change or after the
   final reviewed fix; repeatedly consuming the full timeout to discover one
   unfinished phase at a time is prohibited.

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
7. Concurrent Phase A work must follow the merge-conflict-avoidance rules in
   `REPOSITORY_RULES.md` (`Concurrent Work And Merge-Conflict Avoidance`).
   Subagents receive explicit allow/deny file lists; shared enums, reducers,
   command/event variants, Tauri DTOs, TypeScript wire, generated contract
   artifacts, and issue comments are integrated by the main agent. Monolithic
   inline test files must not accumulate new feature tests; use per-feature
   `crates/<crate>/tests/<feature>.rs` files instead.
8. Temporary worktrees must be removed promptly and their per-worktree build
   artifacts (unshared `target/`, `node_modules/.vite/`, per-worktree
   `node_modules/`) must be cleaned up at the same time. Shared build
   directories such as a shared `CARGO_TARGET_DIR` must not be deleted. See
   `REPOSITORY_RULES.md` `Worktree And Build Artifact Cleanup`.
9. Non-frontier-agent implementation requires frontier-model review at the end
   of substantial work, particularly after AgentSwarm or parallel Phase A
   changes. The review checks the diff against the canon and verification
   output, and its findings must be addressed before landing on `main`. The
   auditor must prioritize repository-rule consistency, Rust/Tauri best
   practices, and security/privacy risks, and must propose canon amendments
   when a finding is caused by a rule gap. See `REPOSITORY_RULES.md`
   `Review And Audit`.
