# QA Lane Catalog

Every runnable verification lane, its command shape, and the private-data-free
tokens that count as evidence. Setup is in [environment.md](environment.md);
the discipline that decides which lane to reach for is in
[verification.md](verification.md).

## Command contract

One Element X-compatible Simplified Sliding Sync engine. There is no backend
selection, probing, or forcing, so no lane takes a backend argument.

- `--server` accepts `tuwunel`, `synapse`, or `both` for headless lanes
  (default `both`). Linux GUI lanes accept `tuwunel` only.
- `--core-backend`, `KOUSHI_QA_FORCE_SYNC_BACKEND`, `conduit`, and
  `timeline_legacy_*` scenarios are rejected by the runners. If you find one in
  a doc, plan, or transcript, it is a historical artifact — see
  [history.md](history.md).
- `--server=synapse` needs a working Docker runtime; the runner verifies it
  before starting.

## Output must be private-data-free

This applies to every lane, every token, every artifact, and every issue
comment. Success output is tokens and counts only.

Never print Matrix room IDs, event IDs, user/sender IDs, device IDs, room
names or topics, message bodies, captions, filenames, avatar URLs or MXC URIs,
aliases or local alias text, moderation reasons, pagination tokens, transaction
IDs, backup versions, account keys, recovery secrets, passphrases, credentials,
tokens, or raw SDK errors.

Do not store post-login real-account screenshots: they can contain room names,
Matrix IDs, message bodies, or attachment names. Rely on private-data-free QA
window-title tokens instead.

## Headless core lane

The primary functional gate. Command shape:

```bash
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:headless-local -- \
    --server=tuwunel \
    --scenario=<name> \
    --core \
    --timeout-ms=240000
```

The runner registers separate synthetic users for the SDK lane and the core
lane. Keep E2EE trust proofs isolated so unrelated smoke-test devices do not
become part of an account's device graph. Within one core lane, every fixture
that represents another device of the same user must also use its own file
credential-store subdirectory; sharing the process-wide saved session silently
restores the primary device and invalidates all multi-device evidence. Restart
fixtures deliberately keep the shared credential store because they prove
restoration of that exact device.

`--scenario=all` runs the aggregate lane. `--scenario=timeline_stress` requires
`--core` and must be the only scenario in the run. The `send_queue` scenario
injects offline failure through a stdlib TCP proxy inside the Rust QA binary and
must be built with `--features qa-bin`; plain `cargo test` does not compile that
binary.

| Scenario | Proves | Evidence tokens |
| --- | --- | --- |
| `safety` | preflight guards | — |
| `login_sync` | login and first sync | — |
| `session_status` | session state projection | — |
| `credential_health` | credential-store health probe under the debug/test file credential-store guard; must refuse to touch the OS keychain | — |
| `native_attention` | notification/badge projection | `notification_candidate=ok`, `badge_state=ok`, `suppress_focus=ok`, `clear_badge=ok` |
| `e2ee_trust` | cross-signing bootstrap, key-backup upload, wrong-secret restore failure, passphrase restore on a second device, SAS verification, identity reset | token-only |
| `device_cleanup` | remote-first device removal, audited against the raw homeserver device list from a separate short-lived audit session, which is then deleted | token-only |
| `gate_restore`, `gate_negative`, `gate_no_proof` | verification-gate paths | `gate_new_identity_bootstrap=ok` |
| `invites_dm` | invite receipt/accept/decline and DM start | `invite_recv=ok`, `invite_accept=ok`, `invite_decline=ok`, `dm_start=ok` |
| `room_space` | room and space classification | — |
| `directory` | public directory query and alias join | `directory_query=ok`, `directory_join=ok` |
| `room_management` | settings edit, permission guard, moderation, using a disposable management room so it cannot disturb other stages | `room_settings=ok`, `permission_guard=ok`, `moderation=ok`, plus cleanup tokens |
| `room_people_projection` | member projection | — |
| `timeline` | timeline projection and navigation | `timeline_nav=ok` |
| `timeline_reconnect` | unsubscribes, sends 21 offline events past the room-subscription limit, reopens the room | `live_catchup_checkpoint=ok`, `live_catchup_gap_repaired=ok` |
| `timeline_stress` | sustained timeline load (needs `--core`, run alone) | — |
| `activity` | account-wide Recent/Unread activity | `activity_recent=ok`, `activity_unread=ok`, `activity_resolution=ok`, `activity_markread=ok` |
| `composer` | mentions, markdown, slash commands, IME guard | `mention_send=ok`, `markdown_send=ok`, `slash_command=ok`, `ime_guard=ok` |
| `reply` | reply quote and pin lifecycle | `reply_quote=ok`, `pin_event=ok`, `pinned_state=ok`, `unpin_event=ok` |
| `media` | upload staging, captions, compression, receive, gallery | `upload_staging=ok`, `media_gallery=ok`, `send_media=ok`, `media_caption=ok`, `image_compress=ok`, `recv_media=ok`, `media_caption_edit=ok` |
| `live_signals` | receipts, read markers, typing, presence | `read_receipt=ok`, `fully_read=ok`, `typing=ok`, `presence=ok`, `live_signals=ok` |
| `thread` | thread projection | — |
| `edit_redact_search` | edit, redact, search | — |
| `redact_edit_convergence` | redaction/edit room-latest, Activity, unread/navigation, and thread convergence across restored snapshots | `redact_edit_convergence=ok` |
| `search_crawler` | crawler-fed search index | — |
| `scheduled_send` | schedule, reschedule, cancel, fire | `scheduled_capability=local_fallback`, `scheduled_create=ok`, `scheduled_reschedule=ok`, `scheduled_cancel=ok`, `scheduled_fire=ok` |
| `send_queue` | retry/cancel across injected offline failure (`--features qa-bin`) | — |
| `restore_cleanup` | session restore and logout cleanup | — |
| `link_preview` | link preview projection | — |
| `cache_restore` | deep-history anchor restored from cache within a bounded number of backward-paginate cycles while the network is blocked | — |
| `read_state_convergence` | local viewed boundary advances while receipt/read-marker writes are held or failed, then converges through the bounded Rust dispatcher | `read_state_convergence=ok` |

Key-backup scope: `joined_room_restore=ok` is the #30 MVP proof token for
recovery-secret import plus currently joined-room key hydration. It is not proof
of exhaustive backup-wide restore. `KeyBackupRestoreSummary.scope` must remain
`JoinedRooms` unless docs/policies and upstream SDK feedback record a broader
public API or a reviewed vendored patch decision.

Room-list space classification can lag behind room/space create or join on local
homeservers. Headless core QA should perform a bounded `SyncOnce` after A
creates/invites and after B joins before asserting `rooms` vs `spaces`;
otherwise a valid space can temporarily appear as a plain room and make
aggregate lanes flaky.

Headless logout cleanup must observe both the exact correlated `LoggedOut` event
and an authoritative `SessionState::SignedOut` snapshot before issuing a
follow-up restore. Those independent lanes may arrive in either order. Event
waiters use one monotonic absolute deadline across all unrelated events and
phases; recreating a relative timeout inside the receive loop is forbidden.

If `live_signals` reaches `fully_read=ok` and then times out at typing, the
observer needs a bounded debug/test `SyncOnce` on the observer account after
`SetTyping` is acknowledged, to wake the same Rust-owned typing observer. Do not
replace this with React polling or local UI timers.

## Linux virtual-display GUI lane

Real Tauri WebView driven through WebDriver under Xvfb. Command shape:

```bash
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=<name> \
    --server=tuwunel \
    --skip-build \
    --artifact-dir=artifacts/linux-gui-<name>-fast \
    --timeout-ms=180000
```

`node scripts/desktop-linux-gui-qa.mjs --list` is the authoritative scenario
list. For a quick window-state sanity check, use the cheap QA title helpers such
as `--qa-title-ready` and `--qa-title-send-ready` before starting a full
scenario run. See [environment.md](environment.md#reusing-a-debug-build) for the
`--skip-build` footgun.

| Scenario | Drives | Evidence tokens |
| --- | --- | --- |
| `signed-out` | signed-out shell | — |
| `local-login` | login to ready | — |
| `local-send` | send a message | — |
| `local-create-room`, `local-create-space` | creation flows | — |
| `local-logout-relogin` | logout and re-login | — |
| `local-spaces-nav` | space navigation | — |
| `local-invites-dm` | accepts a real invite through the Invites pane and starts a DM through the New DM dialog; waits for `data-room-kind="dm"` in the real room list | — |
| `local-reply` | reply to a message row | — |
| `local-media` | staged attachment, caption, download, gallery, viewer | `gui_local_media_stage=ok`, `gui_local_media=ok`, `gui_local_media_caption=ok`, `gui_local_media_viewer=ok` |
| `local-image-compression` | sets Compress images to Always, attaches a synthetic wide PNG, waits for the compressed `.jpg` filename, `image/jpeg`, and selected dimensions | `gui_local_image_compress=ok` |
| `local-room-tags` | real room row context menu, waits for the row to move between Rooms and Favourites | `gui_local_room_tag_set=ok`, `gui_local_room_tag_removed=ok` |
| `local-room-management` | topic edit (waits for `AppState.room_management.settings.topic`), role change through the Rust-owned power-level command, kick, waiting for the room-scoped `settings.members` snapshot to remove the row | `gui_local_room_topic=ok`, `gui_local_room_role=ok`, `gui_local_room_kick=ok` |
| `local-activity` | Activity rail entry and tab switching | `gui_local_activity_open=ok`, `gui_local_activity_unread_tab=ok`, `gui_local_activity_recent_tab=ok` |
| `local-explore` | real Explore search and Join over a synthetic public-room fixture | `gui_local_explore_query=ok`, `gui_local_explore_join=ok` |
| `local-message-actions` | hover-gated action menu, source/forward, redaction, `Hide deleted messages` toggle to `TimelineItem.is_hidden` | — |
| `local-pins` | pin affordances | — |
| `local-message-types` | injects `m.emote`, `m.notice`, and formatted spoiler events; checks `data-message-kind`, collapsed spoiler, reveal | — |
| `local-composer` | mention autocomplete from `ProfileState.users`, Bold toolbar, slash input, then Rust-owned `send=sent` plus composer clear | `gui_local_mention=ok`, `gui_local_markdown=ok`, `gui_local_slash=ok` |
| `local-scheduled-send` | `Send later`, `datetime-local` via the shared setter, create/edit/cancel | `gui_local_scheduled_create=ok`, `gui_local_scheduled_reschedule=ok`, `gui_local_scheduled_cancel=ok` |
| `local-timeline-navigation` | first-unread pill, bottom pill, jump-to-date focused context | `gui_local_timeline_unread_jump=ok`, `gui_local_timeline_bottom_jump=ok`, `gui_local_timeline_date_jump=ok` |
| `local-rich-formatting` | sanitized Matrix HTML rendering (`strong`, blockquote, list, link, code block, copy control), then toggles `display.code_block_wrap` and waits for the code block CSS to switch from `pre-wrap` to `pre` | — |
| `local-alias` | sets a local alias through `set_local_user_alias`, waits for Rust-projected timeline/member labels, clears it, waits for both surfaces to revert | `gui_local_alias_set=ok`, `gui_local_alias_clear=ok` |
| `local-cjk` | long Japanese/CJK room name and message; verifies `line-break: strict`, `word-break: normal`, `hyphens: none`, room ellipsis, message wrapping, no horizontal document overflow | `gui_local_cjk=ok` |
| `local-settings` | real Settings UI: composer shortcut and theme, E2EE trust section presence, waits for `aria-pressed="true"` / `data-theme="dark"` | — |
| `local-e2ee-key-management` | room-key export, import, secure-backup setup with recovery-key artifact delivery | `gui_room_key_export=ok`, `gui_room_key_import=ok`, `gui_secure_backup_setup=ok` |

Lane scope notes:

- `local-invites-dm` is a deterministic WebDriver smoke. It waits on
  `data-room-kind="dm"`, so keep `RoomButton`'s data attributes in sync with the
  Rust-owned sidebar snapshot if the room list markup changes. Keep using the
  core `invites_dm` scenario for invite-projection correctness.
- `local-activity` intentionally leaves Recent/Unread row ordering,
  focused-context row jumps, and mark-read correctness to the Rust core and
  browser-headless gates.
- `local-media` must not use the visible Attach button to open a native file
  dialog. WebDriver writes an ignored synthetic fixture file in the scenario
  artifact directory, sets that path on
  `input[type=file][aria-label="Attach file input"]`, falls back to
  `DataTransfer.files` if WebKit does not populate `input.files`, confirms no
  `.message-media` row appears before Send, fills the staged upload caption
  field, then waits for `timeline_room=true` and a Rust-owned media row plus
  caption. It also opens `Open media gallery`, opens the item in `Media viewer`,
  and closes the viewer before recording evidence. Do not monkeypatch
  `window.__TAURI_INTERNALS__` from WebDriver; WebKit driver execution contexts
  do not provide a reliable app-world command recorder. The lane uses synthetic
  filenames and content only.
- `local-image-compression` uses a binary-safe `DataTransfer` fallback for the
  synthetic PNG. User Settings can unmount the timeline surface in the real
  WebView, so the lane must reselect the QA Seed Room before attaching media.
- `local-room-tags` must wait until the row is observed in the expected section.
  Do not mutate React state, monkeypatch Tauri IPC, or treat menu click
  completion as evidence.
- `local-e2ee-key-management` may legitimately finish in recovery state: after
  secure-backup setup the SDK recovery observer can move the session to
  `needsRecovery`, so the right panel is forced to Recovery by Rust-owned
  session state. The lane accepts either the Settings secure-backup status or QA
  title `panel=recovery session=needsRecovery` as setup evidence.

## Browser-headless (Playwright) lane

```bash
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop exec -- playwright test <spec> -g "<name>" --workers=1
```

`playwright.config.ts` pins `workers: 1`. `fullyParallel: false` alone still
spreads FILES across workers, and the recorded flakes were all traced to those
workers contending for the single shared Vite harness server. Do not raise the
worker count to speed up a run: the whole suite finishes in about three minutes
serialized, and the parallel-contention flakes come straight back.

The full-app harness (`apps/desktop/src/test/appHarnessMain.tsx`) must import
`../styles.css`, matching production `main.tsx`. Otherwise visibility/layout
assertions can pass against unstyled DOM and miss real production CSS issues.

The Space Members role contract is a Browser-headless gate, not a live-server
scenario. Run the focused harness test with:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/room-space-invites.spec.ts \
  -g "Space member roles" --workers=1
```

It waits on invoke-count and snapshot/DOM barriers (never sleeps) and records
the fixed, private-data-free evidence token `space_member_role=ok`. The test
covers authorized success, stale failure/retry, admin confirmation with inert
Cancel, and child-sync independence. It must not be replaced by a real-account
or live-homeserver scenario.

Harness and spec rules that have each caused a real failure are collected in
[troubleshooting.md](troubleshooting.md#browser-headless-harness).

## Real-account lanes

Attended, credential-bearing, and destructive if misused. These consume device
slots on a real homeserver.

- `npm --prefix apps/desktop run qa:real-homeserver` — headless real-account
  scenarios, writes `qa.log` synchronously before leak checks and exit handling.
- `npm --prefix apps/desktop run qa:mac-gui` — macOS GUI smoke driven through
  `System Events`.

The macOS smoke opens User Settings, selects Display, and activates the
semantic `Compact`, `Default`, and `Comfortable` buttons. It then resizes
`window 1` through System Events' native `set size` command and restores the
original size best-effort. After each transition it polls the private-data-free
Rust receipt title token:
`viewport=aligned viewport_generation=N viewport_parent=true
viewport_webview=true viewport_js=true viewport_root=true`. The generation and
alignment values come from the Rust receipt after native repair; the harness
never computes expected geometry or uses fixed window coordinates. The
`viewport_decision` token records whether that receipt repaired the frame.

Safety rules:

- Pass credentials through `KOUSHI_QA_LOGIN_PIPE`, which contains only a FIFO
  path in the environment and keeps the payload out of argv, logs, screenshots,
  and committed files. Never drive real-account login by fixed window-relative
  coordinates.
- Real-login GUI smoke must set `KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1`.
  `KOUSHI_SKIP_SAVED_SESSIONS=1` only prevents saved-session reads; a successful
  login can still prompt macOS Keychain during session persistence or encrypted
  SDK store key creation.
- First-run GUI smoke should set `KOUSHI_SKIP_SAVED_SESSIONS=1`, or opening User
  Settings can read the macOS Keychain and show a confirmation prompt that
  blocks unattended automation.
- Do not pass the parent shell environment wholesale into GUI smoke child
  processes. Filter out secret-like variables such as API keys, tokens, and
  passwords before spawning `npm run tauri dev`.
- The smoke CLI must attempt logout cleanup after any post-login QA failure
  unless `--keep-session` was explicitly requested. Otherwise a failed
  sync/timeline QA can leave a live smoke device on the homeserver.
- Avoid repeated destructive real-account login cycles while debugging GUI
  automation. Prefer preserving the same running Tauri session while iterating
  on panel/menu checks.
- `--qa-profile=<name>` is the opt-in path for persistent restore/sync QA. It
  preserves the SDK SQLite store, cache, search index, saved session, and
  incremental sync state under ignored
  `.local-secrets/qa-profiles/<name>/data`. Profile names must be synthetic and
  non-secret. It must set `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` so unattended
  runs do not prompt macOS Keychain; that env-controlled file credential store
  stays behind a debug/test-only compile-time gate and release builds must
  ignore it in favour of the OS credential store. A Keychain prompt during a
  profile run is an automation failure — verify the env var is in `--child-env`.
- `--allow-private-screenshots` is only for explicitly approved test accounts
  whose post-login room and message data may be written to ignored artifacts.
- `--allow-empty-timeline` is for sparse test accounts when the goal is
  validating login, room-list sync, and GUI panel automation. Keep the strict
  `timeline_items > 0` release signal for normal real-account smoke.

Prompt order differs between the two entry points:

- `password-login-smoke`: homeserver, username, device name, password.
- `qa:mac-gui -- --real-login-from-stdin`: homeserver, username, password,
  device name, then an optional recovery code. Send all five newline-terminated
  lines; leave the fifth empty to accept `needsRecovery` as a post-login sync QA
  state, and provide it only when verifying recovery completion to `ready`.

## Credential-health tiers

- Tier 1, fast and local — see
  [state-ownership.md](state-ownership.md#credential-health) for the focused
  checks, plus the `credential_health` scenario above.
- Tier 2, real macOS Keychain, opt-in. The previous manual GitHub Actions lane
  is disabled: the preserved recipe lives at
  `.github/workflows.disabled/macos-keychain-tier2.yml`, and GitHub also has the
  workflow disabled manually. Do not run `gh workflow run
  macos-keychain-tier2.yml` until that file is deliberately moved back under
  `.github/workflows/` and re-enabled. Use a manual macOS session instead. Keep
  any future workflow key-crate-only: it copies `crates/koushi-key` to
  `$RUNNER_TEMP` and runs `cargo test --manifest-path` there, so it must not
  require the private vendored Matrix SDK submodule. For a manual macOS session
  without an initialized vendor submodule, use the same temp-copy pattern before
  setting `KOUSHI_MACOS_KEYCHAIN_QA=1`. The test treats `security
  set-key-partition-list` as best-effort on hosted runners; the pass/fail proof
  is the real backend set/get/delete plus missing-credential mapping after
  delete. It temporarily makes the throwaway keychain the user default keychain
  and restores the prior default in a guard, because the macOS `keyring` backend
  writes generic passwords through the default keychain.
- Tier 3, attended only: consent dialogs, Touch ID, locked login-keychain UX,
  and signed-build ACL behavior. Locked-keychain reads on hosted runners can
  block on native authentication UI.

## Startup latency observability

Read-only timing lane for issue #123 Phase A. Full operator details are in
[docs/qa/startup-latency-observability.md](../qa/startup-latency-observability.md);
the implementation plan is
[docs/superpowers/plans/2026-06-23-startup-latency-observability-phase-a.md](../superpowers/plans/2026-06-23-startup-latency-observability-phase-a.md).

```bash
npm --prefix apps/desktop run qa:real-homeserver -- --scenario=startup_latency
```

- **Run 1 performs a real login** and consumes one device slot on the
  homeserver. This requires explicit maintainer GO before the first invocation.
  Subsequent runs restore from the SQLite store without a new login.
- The runner performs two passes against a persistent profile dir
  (`.local-secrets/real-account-qa/profile/startup_latency/`, git-ignored): run 1
  logs in and populates the event cache; run 2 does a cold restore and is the
  measured evidence run.
- Set `KOUSHI_STARTUP_LAT_TEARDOWN=1` to log out and remove the QA device at the
  end of a run. Without it the session is kept so run 2+ can restore.
- Both runs emit `startup_lat phase=… ms=N` macro-phase tokens and, when
  `KOUSHI_STARTUP_TRACE=1` (set by the runner), `koushi.startup phase=…` and
  `phase=origin origin=cache|network|sync` sub-phase tokens.
