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
  before starting. Retained local fixture directories and generated files must
  remain editable and recursively removable by the invoking host user after
  success, startup failure, scenario failure, or interruption. The Synapse
  runner aligns its runtime UID/GID with the host on POSIX and installs its
  non-secret entrypoint with execute permission for that numeric identity.
  Fixture retention/replay is preserved; neither world-writable fixture data
  nor ownership changes outside the exact fixture directory are permitted.

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

The primary functional gate. The binary and orchestration live in the
non-default `koushi-qa` package and consume `koushi-protocol` DTOs plus narrow
Core test hooks; `koushi-core` production sources contain no QA binary tree.
Command shape:

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
`--core` and must be the only scenario in the run. The bounded maximum stress
characterization is 10 spaces × 10 rooms × 100 messages (100 rooms and 10,001
messages including the blank-formatted-row probe). Run that required scale
explicitly, for example:

```bash
KOUSHI_QA_STRESS_SPACES=10 \
KOUSHI_QA_STRESS_ROOMS_PER_SPACE=10 \
KOUSHI_QA_STRESS_MESSAGES_PER_ROOM=100 \
npm --prefix apps/desktop run qa:headless-local -- --run \
  --server=tuwunel --core --scenario=timeline_stress \
  --timeout-ms=600000 --cargo-profile=release
```

The stress send oracle is intentionally stricter than the runtime send contract.
The production state machine permits a `SendCompleted` event to replace the
matching pending row atomically with its event-ID row when a server omits or
lags the SDK local-echo diff. `timeline_stress` is instead an interoperability
and load diagnostic: every submitted send must produce the exact local echo,
matching `SendCompleted`, and a non-empty event ID. A no-local-echo result is
therefore product-compatible but is still a stress failure; do not weaken the
waiter or report the lane green. Preserve its coordinate and server/SDK trace
for the contract decision.

The `send_queue` scenario injects offline failure through a stdlib TCP proxy
inside the Rust QA binary and must be built with `koushi-qa`'s `--features
qa-bin`; plain `cargo test` does not compile that binary.

| Scenario | Proves | Evidence tokens |
| --- | --- | --- |
| `safety` | preflight guards | — |
| `login_sync` | login and first sync | — |
| `session_status` | session state projection | — |
| `history_key_recovery_window` | Delayed invitation import at 29/30 days plus explicit Core history-key requests: join without individual invitation, multiple historical sessions, offline administrator, same-device restart, administrator-only startup retry with persistent cooldown, authoritative 31-member default and explicit all-member override. Requires `--core`; disposable local Synapse is the focused proof. [Scenario contract](../qa/history-key-recovery-window.md). | `history_window_deferred=ok`, `history_window_29_days=ok`, `history_window_30_days_expired=ok`, `history_recovery_offline_pending=ok`, `history_recovery_startup_admin_retry=ok`, `history_recovery_restart_cooldown=ok`, `history_recovery_join_without_invite=ok`, `history_recovery_without_manual_verification=ok`, `history_recovery_31_members_admin=ok`, `history_recovery_force_all=ok`, `history_recovery_after_window_expiry=ok`, `history_recovery_visibility_guard=ok`, `history_recovery_membership_guard=ok`, `history_recovery_blocked_device_guard=ok`, `history_recovery_crash_replay=ok`, `history_recovery_policy_race_guard=ok`, `history_recovery_identity_race_guard=ok`, `history_window_cleanup=ok`, `history_key_recovery_window=ok` |
| `credential_health` | credential-store health probe under the debug/test file credential-store guard; must refuse to touch the OS keychain | — |
| `account_notifications` | #981 shared push-rule categories and email pusher guards: a second session for the same account customises rules (mixed group rules, custom DM sound, `@room` disabled); two Core loads leave the server ruleset unchanged; a group toggle writes only group rules and re-reads OFF/ON; an unencrypted mention still notifies with group OFF (encrypted-room behaviour is covered by `koushi-sdk` unit tests); the email flow reports an unsupported server and never creates a pusher for an unverified address; the #1427 keyword write, driven through the app's own commands and read back raw from the server: the minted `content` rule carries the app rule-id shape, the synthetic pattern, and exactly `["notify", {"set_tweak": "highlight"}]` with no sound tweak, the app projects it ON, the same word in another case is refused as a duplicate and adds no second rule or id, an out-of-band disable still projects OFF and still refuses a same-pattern add, and removal clears both the server ruleset and the projection; requires `--core`, separate from `all`. The local homeservers do not send email, so address verification and digest delivery are not covered | `account_notifications_load=ok`, `account_notifications_no_write_on_open=ok`, `account_notifications_category_write=ok`, `account_notifications_overlap=ok`, `account_notifications_category_restore=ok`, `account_notifications_email_unsupported=ok`, `account_notifications_email_requires_verified=ok`, `account_notifications_keyword_actions=ok`, `account_notifications_keyword_projection=ok`, `account_notifications_keyword_duplicate=ok`, `account_notifications_keyword_disabled=ok`, `account_notifications_keyword_remove=ok`, `account_notifications=ok` |
| `user_verification` | #1024 Verify user between two freshly bootstrapped users: both Tuwunel and Synapse use a new encrypted DM; QA waits for A's own room-list projection of B's membership and then for the account actor's post-send verification-progress event — the command-acceptance projection already reports `Requested`, so it cannot stand in for the send — and only then requires B to receive and accept the in-room request; both Cores show the same seven SAS emoji and confirm; A's refreshed contact security reports B verified by A, no longer offered, and B's device confirmation unchanged; requires `--core`, separate from `all`. The megolm room key and the request event travel on independent Sliding Sync connections (the `all_rooms` list and the `encryption` connection), so B may first store the request as undecryptable; the SDK event cache then feeds the request to the verification machine when it resolves the UTD, and the incoming-verification observer surfaces it from the resolved-UTD report (#1462). The scenario must not wait for B's room list before A may send: doing so removes that key-after-event interleaving, where the room and the request arrive in one response | `user_verification_dm_joined=ok` (new-DM servers), `user_verification_offered=ok`, `user_verification_a_observed_join=ok`, `user_verification_request_sent=ok`, `user_verification_incoming_request=ok`, `user_verification_accepted=ok`, `user_verification_sas_match=ok`, `user_verification_done=ok`, `user_verification_identity_verified=ok`, `user_verification=ok` |
| `native_attention` | notification/badge projection | `notification_candidate=ok`, `badge_state=ok`, `suppress_focus=ok`, `clear_badge=ok` |
| `e2ee_trust` | cross-signing bootstrap, key-backup upload, wrong-secret restore failure, passphrase restore on a second device, SAS verification, identity reset | token-only |
| `e2ee_login_store` | persistent-store-first fresh/saved/restore/restart/reauth identity continuity; stock forced rotation plus offline and online index-0 delivery in DM and three-user encrypted room | `e2ee_login_store_forced_rotation_index0=ok`, `e2ee_login_store_fresh_offline_index0=ok`, `e2ee_login_store_restore_offline_index0=ok`, `e2ee_login_store_restart_offline_index0=ok`, `e2ee_login_store_reauth_offline_index0=ok`, `e2ee_login_store_online_index0=ok`, `e2ee_login_store_group_index0=ok`, `e2ee_login_store_identity_stable=ok`, `e2ee_login_store=ok` |
| `device_cleanup` | remote-first device removal, audited against the raw homeserver device list from a separate short-lived audit session, which is then deleted | token-only |
| `gate_restore`, `gate_negative`, `gate_no_proof` | verification-gate paths | `gate_new_identity_bootstrap=ok` |
| `invites_dm` | invite receipt/accept/decline and DM start | `invite_recv=ok`, `invite_accept=ok`, `invite_decline=ok`, `dm_start=ok` |
| `room_space` | room and space classification | — |
| `directory` | public directory query, alias join, Rust address preview/create/share-link join and collision, the advisory availability check, the Space-prefixed default address (#1006), and public-Space rooms (#1023): the public default, an unnamed public room without an address (calculated name, Space link), an entered address naming an unnamed room, its conflict, renaming later, and the explicit private choice | `directory_query=ok`, `directory_join=ok`, `room_address_preview_create_share=ok`, `room_address_collision=ok`, `room_address_availability=ok`, `room_address_space_prefix=ok`, `room_public_space_default=ok`, `room_unnamed_public_space=ok`, `room_unnamed_alias_name=ok`, `room_unnamed_alias_conflict=ok`, `room_unnamed_rename=ok`, `room_public_space_private_choice=ok` |
| `room_management` | shared Space topic empty/multiline/remote updates (open settings and the synced `SpaceSummary.topic`, #1661)/clear/reopen and topic-only permission, settings edit, permission guard, moderation, a Space's join-rule change (#935), adding an existing (room version 12) room to a Space (#1007), leaving a Space with a chosen subset of its rooms, and the #1177 access/history editor: create with an explicit membership policy naming a Space the creator provably cannot edit, a server-side count that the explicit `history` initial-state event occurs exactly once, an independent read-back of the exact canonical allow content and all four history values (never the success snapshots), a second client's OPEN settings reconciling to that exact target set and to a history change without reload, a restricted restore, a second parent attachment whose projection and server-side `m.space.child` are both established before the token, and a room created with the `knock` join rule that a second account knocks, using disposable management rooms and Spaces so they cannot disturb other stages | `room_settings=ok`, `permission_guard=ok`, `moderation=ok`, `space_access_create_membership=ok`, `space_access_target_without_permission=ok`, `space_access_history=ok`, `space_access_history_values=ok`, `space_access_second_client=ok`, `space_access_restricted_restore=ok`, `space_access_two_parent=ok`, `space_access=ok`, `space_topic=ok`, `space_add_existing=ok`, `space_leave_children=ok`, `knock_room_created=ok`, `knock_join_request=ok`, plus cleanup tokens |
| `room_people_projection` | room-scoped member projection, invited main/thread candidates before joining, joined state after acceptance, leave removal, alias search, and structured mention content | `room_people_invited_scope=ok`, `room_people_projection=ok` |
| `timeline` | timeline projection and navigation | `timeline_nav=ok` |
| `timeline_reconnect` | unsubscribes, sends 21 offline events past the room-subscription limit, reopens the room | `live_catchup_checkpoint=ok`, `live_catchup_gap_repaired=ok` |
| `timeline_stress` | sustained timeline load (needs `--core`, run alone) | — |
| `activity` | account-wide Recent/Unread activity | `activity_recent=ok`, `activity_unread=ok`, `activity_resolution=ok`, `activity_markread=ok` |
| `composer` | mentions, markdown, slash commands, IME guard | `mention_send=ok`, `markdown_send=ok`, `slash_command=ok`, `ime_guard=ok` |
| `reply` | reply quote, open-original jump, anchored forward pagination to the live end, anchored backward pagination, the anchored live tail, and pin lifecycle | `reply_quote=ok`, `reply_quote_jump=ok`, `anchored_forward_live_end=ok`, `anchored_backward_history=ok`, `anchored_live_tail=ok`, `pin_event=ok`, `pinned_state=ok`, `unpin_event=ok` |
| `media` | upload staging, captions, compression, receive, gallery | `upload_staging=ok`, `media_gallery=ok`, `send_media=ok`, `media_caption=ok`, `image_compress=ok`, `recv_media=ok`, `media_caption_edit=ok` |
| `avatar_demand` | dedicated 1,500-reader source, eight visible plus eight prefetch images, scoped avatar bytes that the cache re-encodes to WebP (#1396), disjoint window move/return, cache reopen, shared in-flight cancellation and logout retirement with a retained scope; requires `--core`, separate from `all` | `avatar_window_requests=ok` |
| `live_signals` | receipts, live reader source/ACK/download/scoped bytes/cache reopen/close, read markers, typing, presence, ignore/unignore history recovery | `read_receipt=ok`, `fully_read=ok`, `typing=ok`, `presence=ok`, `ignored_user_history_recovery=ok`, `live_signals=ok` |
| `thread` | Rust-owned thread reply quote lifecycle, thread-root projection lifecycle, Room display block, and Thread panel relation | `thread_reply_quote_lifecycle=ok`, `thread_projection_lifecycle=stable`, `thread_summary=ok`, `thread_recv=ok` |
| `edit_redact_search` | edit, redact, search | — |
| `redact_edit_convergence` | redaction/edit room-latest and Activity convergence plus live Rust-projected Room/thread-panel summary advance, edit, redaction rollback, and real-runtime restore parity | `redact_edit_convergence=ok`, `thread_summary_convergence=ok` |
| `search_crawler` | crawler-fed search index; `crawl_throttle` changes the speed Standard → Slow, waits for the room to be Completed again (a completed room whose latest event changed may legitimately catch up, #996) and requires that no crawl was queued as `initial` since the change, read from the token-only `core.search crawl_enqueued cause=` diagnostic (#1634) | `crawl_backfill=ok`, `crawl_no_media_bytes=ok`, `crawl_throttle=ok`, `crawl_failure=ok` |
| `search_crawler_catchup` | a completed room is revisited (#996): a message posted while the reader runs, and messages posted while it is stopped, become searchable after restart without opening the room; requires `--core`, separate from `all`, and waits out the 60 s crawler startup delay twice (use `--timeout-ms=600000`) | `crawl_catchup_live=ok`, `crawl_catchup_restart=ok`, `search_crawler_catchup=ok` |
| `room_history_export` | Archive export of an encrypted room and a Space: full history, a period `[start, end)`, the bounded period seek and its fallback (#988), a withheld key exported as `m.bad.encrypted` and counted, Stop leaving no partial room, a Space export that leaves out a DM, a decrypted image with a thumbnail, and a resume that adds only a newly joined room | `history_export_full=ok`, `history_export_period=ok`, `history_export_period_fallback=ok`, `history_export_utd_counted=ok`, `history_export_cancel=ok`, `history_export_space=ok`, `history_export_attachments=ok`, `history_export_resume=ok`, `room_history_export=ok` |
| `scheduled_send` | schedule, reschedule, cancel, fire | `scheduled_capability=local_fallback`, `scheduled_create=ok`, `scheduled_reschedule=ok`, `scheduled_cancel=ok`, `scheduled_fire=ok` |
| `send_queue` | retry/cancel across injected offline failure (`--features qa-bin`) | — |
| `restore_cleanup` | session restore and logout cleanup | — |
| `link_preview` | link preview projection | — |
| `read_state_convergence` | local viewed boundary advances while receipt/read-marker writes are held or failed, then converges through the bounded Rust dispatcher | `read_state_convergence=ok` |
| `hidden_state_acl` | one ordinary message followed by 125 real `m.room.server_acl` updates (#1110/#1117): every update stays a hidden item, a re-subscription replay of the Core-held timeline (more raw items than the 120-row live-edge capacity) still contains the message, and viewing only the message converges to a server-confirmed read boundary and a zero unread count; requires `--core`, separate from `all` | `hidden_state_acl=ok` |
| `thread_late_joiner` | in a `joined`-history room, a member who joins after the thread root was sent sees a permanent not-visible root row (`NotFound`/`Forbidden`, never a retryable failure, exported with `is_hidden == false` so its reply chip stays reachable, #1141) and can still open the thread and read the reply sent after they joined; a fresh device whose sync window no longer holds that reply opens the thread through `OpenThread` and the pane settles in `Open` or a visible `Failed(Forbidden|NotFound)`, never a silent close, and Retry/Close settle again (#1473) | `thread_late_joiner_root_not_visible=ok`, `thread_late_joiner_thread_panel=ok`, `thread_late_joiner_withheld_open_settled=ok`, `thread_late_joiner_withheld_retry_close=ok`, `thread_late_joiner=ok` |
| `mcp_message_actions` | #1628 display-independent message read/action entry point an MCP caller uses, proven from two account runtimes on a disposable local homeserver: A posts synthetic messages in a room both accounts joined, `Read` returns the sender, timestamp, `Text` kind, `content_available` and the body, a second `React` with the same key reports `added=false` with the existing reaction event id (no duplicate event), `Unreact` reports `removed=1` then `removed=0`, `Edit` shows up as the edited body on the next `Read`, and repeating `Edit` or `ThreadReply` with the same retry key replays the first event id instead of sending again (#1681), a second `Delete` reports `already_redacted=true` with no new event and the target then reads redacted without a body, and a `ThreadReply` reads back with its root; B re-reads the same events through the same operations, which proves the effects reached the homeserver rather than a local store, and B never mounts a timeline. Asserts no GUI state, timeline actor, receipt, or unread value. Requires `--core`, separate from `all`, runs on both servers | `mcp_message_actions=ok` |
| `idle_sync_gap_inspection` | #1392 idle-sync gap-inspection benchmark: several resident encrypted rooms with a large persisted history run an idle window; every `core.timeline_gap_inspection` record with `continuity=complete` must report `event_bodies_decoded = 0` (healthy rooms decode nothing; gapped rooms decode only their minimal boundary events, reported separately), a completed `live_edge` inspection must be observed, the zero-room committed-response amplifier is counted, and p50/p95 inspection durations are printed (tunable via `KOUSHI_QA_IDLE_INSPECTION_ROOMS`, `KOUSHI_QA_IDLE_INSPECTION_MESSAGES_PER_ROOM`, `KOUSHI_QA_IDLE_INSPECTION_WINDOW_MS`) | `idle_sync_gap_inspection=ok` |
| `notification_delivery` | #1427/#1460 device-local notification delivery, with each provider replaced by a loopback HTTP stub this scenario owns (never a real provider) and the sink credential written through the app's own command: a destination with no target selected leaves an unread direct message alone; with its categories on and a one second dwell the direct message produces no request before the dwell and exactly one after, authorized by the credential and addressed to the lane's own stub, naming the room by the label the badge shows with its counts, and carrying no room id, event id, account id or message text; a read that lands inside the dwell cancels the send and the same room unread again sends after a fresh dwell; a second delivery inside the destination's `timing.min_interval_secs` is held and released, not dropped, when the window reopens; a watched room sends while every category is off, and a room that joins a watched Space after the list was written sends too; the sink secret reaches neither the settings snapshot nor the diagnostics stream; changing the endpoint a stored credential was entered for takes that sink out of service and its credential out of the file store; with `content: Prefix` and a consent that covers the sink kind, its recipient and its endpoint, the delivered body quotes the full text of the room's newest message as a `> ` line under the room line, and with a prefix but no consent (read back as names only), with names only chosen, and when the newest event is a file message whose caption is the marker (no readable text), the delivery still succeeds and the marker reaches no body and no idempotency key; changing the destination, or the endpoint, that a stored consent was given for drops the consent and returns the destination to names only, so the next body carries no marker and on an endpoint change nothing is sent at all; and that marker reaches no serialized app-state snapshot, no settings file, no diagnostics stream, and no file the lane wrote; a second ServerChan destination on its own stub takes a synthetic `SCTqa<pid>` SendKey through the same command, drops a text consent when a SendKey is saved again, and sends exactly one request the app reads as accepted, a form POST to `{endpoint}/{SendKey}.send` with no bearer, naming the room kind and unread count and carrying no message text without consent, while the SendKey reaches no snapshot, settings file, diagnostics stream or captured body. Requires `--core`, separate from `all` | `notification_delivery_file_credential_store=ok`, `notification_delivery_unfocused=ok`, `notification_delivery_channel_off=ok`, `notification_delivery_dwell=ok`, `notification_delivery_request=ok`, `notification_delivery_body=ok`, `notification_delivery_read_cancels=ok`, `notification_delivery_rearm=ok`, `notification_delivery_min_interval=ok`, `notification_delivery_watched_room=ok`, `notification_delivery_watched_space=ok`, `notification_delivery_secret_hygiene=ok`, `notification_delivery_secret_binding=ok`, `notification_delivery_prefix=ok`, `notification_delivery_prefix_no_consent=ok`, `notification_delivery_prefix_option_off=ok`, `notification_delivery_prefix_no_text=ok`, `notification_delivery_prefix_consent_cleared=ok`, `notification_delivery_prefix_endpoint_cleared=ok`, `notification_delivery_prefix_hygiene=ok`, `notification_delivery_serverchan_configured=ok`, `notification_delivery_serverchan_sendkey_drops_consent=ok`, `notification_delivery_serverchan_request=ok`, `notification_delivery_serverchan_no_text=ok`, `notification_delivery_serverchan_secret_hygiene=ok`, `notification_delivery=ok` |

Key-backup scope: `joined_room_restore=ok` is the #30 MVP proof token for
recovery-secret import plus currently joined-room key hydration. It is not proof
of exhaustive backup-wide restore. `KeyBackupRestoreSummary.scope` must remain
`JoinedRooms` unless docs/policies and upstream SDK feedback record a broader
public API or a reviewed vendored patch decision.

Room-list space classification can lag behind room/space create or join on local
homeservers. Headless core QA should perform a bounded `SyncOnce` after A
creates/invites and after B joins before asserting `rooms` vs `spaces`;
otherwise a valid space can temporarily appear as a plain room and make
aggregate lanes flaky. Address-prefix checks also wait for the created
Space's authoritative name state: list presence alone does not mean that its
name has been hydrated. Keep the prefix assertion separate from this readiness
check, and use one bounded event deadline rather than a fixed sleep.

Headless logout cleanup must observe both the exact correlated `LoggedOut` event
and an authoritative `SessionState::SignedOut` snapshot before issuing a
follow-up restore. Those independent lanes may arrive in either order. Event
waiters use one monotonic absolute deadline across all unrelated events and
phases; recreating a relative timeout inside the receive loop is forbidden.

If `live_signals` reaches `fully_read=ok` and then times out at typing, the
observer needs a bounded debug/test `SyncOnce` on the observer account after
`SetTyping` is acknowledged, to wake the same Rust-owned typing observer. Do not
replace this with React polling or local UI timers.

### Automated scenario lanes

[`scripts/lib/headless-qa-lanes.mjs`](../../scripts/lib/headless-qa-lanes.mjs)
is the single source for which automation runs each scenario:

- `COVERED_BY_ALL`: stages the nightly `--scenario=all` aggregate job runs.
- `PR_CI_SCENARIOS`: short focused scenarios run on every PR by the
  `core-invites` job on both servers.
- `NIGHTLY_SCENARIOS`: focused scenarios outside `all`, run by the nightly
  `focused` job one at a time; a failure does not stop the rest, and each
  failing scenario files or comments on one open `nightly-qa` issue. That
  issue's identity is its signature label, `nightly-qa/<server>/<scenario>`
  (the aggregate job uses `nightly-qa/<server>/aggregate`), alongside the
  umbrella `nightly-qa` label; the title is only a readable summary. A
  recurrence that scrapes a different failure stage therefore comments on the
  same issue instead of filing another one, and a recurrence after closure
  reopens it. Each passing run is recorded on the open issue, which closes
  after three consecutive passes once a commit on `main` references it
  (`scripts/nightly-qa-record-passes.mjs`; REPOSITORY_RULES
  "GitHub Issue Closure"). Only runs of `main` touch issues.
- `EXCLUDED_SCENARIOS`: scenarios in no lane, each with a written reason.

`node scripts/headless-qa-lane.mjs --lane=pr|nightly --server=<server>` runs a
lane locally the same way CI does. PR CI runs
`node scripts/check-headless-qa-lanes.mjs`, which fails when a scenario
registered in `registry.rs` is in none of these lists. When adding a scenario,
place it in a lane or record why it is excluded.

### Multi-stage QA participant ownership

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
Logout confirmation follows the authoritative-snapshot waiter rule
(engineering rules "Async and Runtime" 1), including a final `SignedOut` read
after timeout, lag, or closure under the original deadline. Failure-injection
acceptance drives each ownership phase through the behavioral cleanup boundary
and asserts logout/barrier/drop/shutdown order plus continuation to the
remaining owners. Source-text inventory guards alone are not evidence.

## Agent MCP scenario lane

Proves that a coding agent can drive Koushi through the MCP bridge end to end
(#1526). It is the only lane where the caller is a model, so the model's prose
is never evidence: the scenario passes on server-side state only.

`headless-core-qa` runs as a long-lived bridge host instead of a scenario when
`KOUSHI_QA_MCP_HOST=1`, reusing the same guarded QA login, then publishes
`mcp/endpoint.json` and grants one MCP client (`KOUSHI_QA_MCP_CLIENT`, `pi` by
default) in `mcp/connections.json`, written in the older schema 1 shape so the
lane keeps proving that format still works. The host has no consent surface, so
its broker runs fail-closed: a missing permission answers `consent_required` at
once instead of waiting for a dialog (#1536). The agent reaches that bridge
through the real `koushi-mcp` launcher, so the transport, the grant file, and
the tool surface are exercised, not a test double.

```bash
DEEPSEEK_API_KEY=<key> node scripts/mcp-agent-scenario.mjs --server=tuwunel
```

The runner creates a disposable homeserver, a fresh account, one room, a
searchable message, and one PNG, asks the agent to search, download the
attachment, and reply, and then asserts:

- a new message from the account replies to the seeded image event, and
- the seeded PNG is in Koushi's media cache (same size and SHA-256).

A passing run prints `agent scenario search_attachment_reply=ok`. The bridge
lives in a short `mkdtemp` directory because a Unix socket path is limited to
about 104 bytes; Koushi's data, credentials, and logs stay under
`.local-secrets/agent-scenarios/<stamp>/`. Run directories are deleted unless
`--keep` is passed.

The lane calls a paid model, so `.github/workflows/agent-scenarios.yml` runs it
on `workflow_dispatch` and nightly rather than per pull request. The model is
`deepseek-flash` through pi's `deepseek` provider; `DEEPSEEK_API_KEY` is the
repository secret. `pi mcp list` must report every Koushi tool before the
agent runs, so a transport or grant failure never looks like a model failure.

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
| `local-image-compression` | attaches a synthetic wide PNG, then selects `1/2` and `JPEG` in the upload-staging dialog's Resize and Format controls, and waits for the compressed `.jpg` filename, `image/jpeg`, and selected dimensions | `gui_local_image_compress=ok` |
| `local-room-tags` | real room row context menu, waits for the row to move between Rooms and Favourites | `gui_local_room_tag_set=ok`, `gui_local_room_tag_removed=ok` |
| `local-room-management` | topic edit (waits for `AppState.room_management.settings.topic`), role change through the Rust-owned power-level command, kick, waiting for the room-scoped `settings.members` snapshot to remove the row | `gui_local_room_topic=ok`, `gui_local_room_role=ok`, `gui_local_room_kick=ok` |
| `local-room-history-export` | Room info → Download history: all available history, a past period, and today's period in the displayed time zone; reads each export folder's room `messages.json` for Element's top-level keys, the seeded messages, and the dialog's per-room count | `gui_local_history_export_all=ok`, `gui_local_history_export_period=ok`, `gui_local_room_history_export=ok` |
| `local-activity` | Activity rail entry and tab switching | `gui_local_activity_open=ok`, `gui_local_activity_unread_tab=ok`, `gui_local_activity_recent_tab=ok` |
| `local-explore` | real Explore search and Join over a synthetic public-room fixture | `gui_local_explore_query=ok`, `gui_local_explore_join=ok` |
| `local-message-actions` | hover-gated action menu, source/forward, redaction, `Hide deleted messages` toggle to `TimelineItem.is_hidden` | — |
| `local-receipt-readers` | five real helper users, a real read receipt, compact `3 + 2` reader affordance, Tauri subscribe/receive/ACK and keyboard close | `gui_local_reader_subscribe=ok`, `gui_local_reader_close=ok` |
| `local-pins` | pin affordances | — |
| `local-message-types` | injects `m.emote`, `m.notice`, and formatted spoiler events; checks `data-message-kind`, collapsed spoiler, reveal | — |
| `local-composer` | mention autocomplete from `AppState.mention_candidates`, Bold toolbar, slash input, then Rust-owned `send=sent` plus composer clear | `gui_local_mention=ok`, `gui_local_markdown=ok`, `gui_local_slash=ok` |
| `local-scheduled-send` | `Send later`, `datetime-local` via the shared setter, create/edit/cancel, a mouse-driven time adjustment, then a short-fuse schedule that must actually fire | `gui_local_scheduled_create=ok`, `gui_local_scheduled_reschedule=ok`, `gui_local_scheduled_cancel=ok`, `gui_local_scheduled_adjust=ok`, `gui_local_scheduled_fire=ok` |
| `local-timeline-navigation` | first-unread pill, bottom pill, jump-to-date focused context | `gui_local_timeline_unread_jump=ok`, `gui_local_timeline_bottom_jump=ok`, `gui_local_timeline_date_jump=ok` |
| `local-rich-formatting` | sanitized Matrix HTML rendering (`strong`, blockquote, list, link, code block, copy control), then toggles `display.code_block_wrap` and waits for the code block CSS to switch from `pre-wrap` to `pre` | — |
| `local-alias` | sets a local alias through `set_local_user_alias`, waits for Rust-projected timeline/member labels, clears it, waits for both surfaces to revert | `gui_local_alias_set=ok`, `gui_local_alias_clear=ok` |
| `local-cjk` | long Japanese/CJK room name and message; verifies `line-break: strict`, `word-break: normal`, `hyphens: none`, room ellipsis, message wrapping, no horizontal document overflow | `gui_local_cjk=ok` |
| `local-settings` | real Settings UI: composer shortcut and theme, E2EE trust section presence, waits for `aria-pressed="true"` / `data-theme="dark"` | — |
| `local-e2ee-key-management` | room-key export and import through the adapter-owned file dialogs, then the `local-secure-backup` steps | `gui_room_key_export=ok`, `gui_room_key_import=ok`, `gui_secure_backup_setup=ok`, `gui_secure_backup_passphrase_change=ok` |
| `local-secure-backup` | the login's new-identity gate reveal (Create, then **I saved the recovery key**) leaves Settings reporting the backup `Enabled`; a Settings passphrase change reveals a new key on screen and only **I saved the recovery key** dismisses it; the optional save is skipped and the key is never read | `gui_secure_backup_setup=ok`, `gui_secure_backup_passphrase_change=ok` |

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
  synthetic PNG. Settings can unmount the timeline surface in the real WebView,
  so the lane must reselect the QA Seed Room before attaching media.
- `local-room-tags` must wait until the row is observed in the expected section.
  Do not mutate React state, monkeypatch Tauri IPC, or treat menu click
  completion as evidence.
- `local-room-history-export` cannot drive the native folder dialog. Its run
  sets the debug-build-only `KOUSHI_QA_HISTORY_EXPORT_DIR`, and the Tauri
  adapter then uses that ignored run directory as the chosen folder instead of
  opening the dialog; the scenario empties it before each export so no earlier
  export is resumed. Release builds compile the override out
  (`scripts/desktop-release-gate-check.mjs`). The lane prints only counts and
  booleans; the files hold synthetic local-homeserver data.
- `local-e2ee-key-management` cannot drive the native room-key file dialogs.
  Its run sets the debug-build-only `KOUSHI_QA_ROOM_KEY_FILE`, and the Tauri
  adapter answers both the export and the import dialog with that ignored
  run-directory file. Release builds compile the override out
  (`scripts/desktop-release-gate-check.mjs`).

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

Specs run in headless Chromium. A spec whose defect is engine-native (search
field decorations, form-control chrome) also runs in headless WebKit, the
engine family of macOS WKWebView and Linux WebKitGTK, by adding it to the
`webkit` project's `testMatch` in `playwright.config.ts`. Prove such a defect
by behavior or pixels (for example, clicking the native control's hit target),
not by DOM button counts. Install both engines once with
`npm --prefix apps/desktop exec -- playwright install chromium webkit`.

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

## Documentation screenshot lane

The README screenshot lane renders the real React `<App />` in the existing
browser harness with synthetic state only. It is not native GUI, Tauri IPC, or
homeserver proof, and it must not be used as a substitute for those lanes.

Run the deterministic generator from the repository root in the pinned
Playwright container documented in [Develop Koushi](../development.md#deterministic-readme-screenshot):

```bash
npm --prefix apps/desktop run docs:screenshot
```

The focused config uses port 5184, one worker, UTC, light theme, and a 2x
scale factor. The checked-in PNG is the only output under test; CI regenerates
it and rejects any byte or git-status difference.

## Website screenshot lane

The website lane (`npm --prefix apps/desktop run site:screenshots`,
`playwright.site.config.ts`, port 5185) uses the same harness and determinism
settings as the README lane. It captures seven scenes for each of `en`, `ja`,
`zh-Hans`, `zh-Hant`, and `ko` into the git-ignored `site/screenshots/`. Like
the README lane, it is synthetic state, not native GUI or homeserver proof.
Run it in the pinned container after `site/prepare-screenshot-fonts.sh`. The
spec fails when CJK characters render as the missing-glyph box, or when a
locale's text is drawn by another language's CJK font (the image's bundled
WenQuanYi Zen Hei otherwise renders every CJK locale). The search scenes
render the golden output of the `koushi-core` `search::site_scenes` test. The
full procedure, publishing, and translation workflow are in
[site/README.md](../../site/README.md).

## Real-account lanes

Retired. Automated QA must not log in to matrix.org or another external
homeserver, including through the macOS or Linux GUI smoke runners. Use
`qa:headless-local -- --server=both` and the disposable local GUI lanes.

## macOS notification activation smoke

The headless registry and account-fence tests run in the ordinary desktop Rust
test suite. A real notification click requires a packaged `.app` with its own
bundle identifier and a macOS desktop session; a `cargo test` binary cannot
exercise the `UNUserNotificationCenter` delegate.

For the attended check, use disposable local Matrix accounts, never an external
homeserver. In the packaged app, explicitly enable Desktop notifications and
grant macOS authorization, then leave a different room open and put Koushi in
the background. Send a message to another room from the second local account.
Click its banner and check that Koushi comes forward and opens the notified
room. Repeat with more than four notifications to cover the former stuck-waiter
limit. Dismiss a banner and clear unread attention to check that neither action
navigates. Diagnostics should record `activation_settled=delivered` only after
the banner click. The current room-list notification has no verified event ID,
so the expected navigation target is the room.

## Windows and Linux notification activation smoke

Use disposable local Matrix accounts in an installed Windows build or a Linux
desktop session with a notification server that supports actions. Enable
Desktop notifications, leave a different room open, send a message to another
room, and click the notification body. Check that Koushi opens the notified
room. Dismiss a second notification and check that it does not navigate.
Windows must use the installed app identity; Linux must expose the freedesktop
default action. These platform checks complement the typed-response unit tests.

## Credential-health tiers

- Tier 1, fast and local — see
  [state-ownership.md](state-ownership.md#credential-health) for the focused
  checks, plus the `credential_health` scenario above.
- Tier 2, real macOS Keychain, opt-in. The previous manual GitHub Actions lane
  is disabled: the preserved recipe lives at
  `.github/workflows.disabled/macos-keychain-tier2.yml`, and GitHub also has the
  workflow disabled manually. Do not run `gh workflow run
  macos-keychain-tier2.yml` until that file is deliberately moved back under
  `.github/workflows/` and re-enabled. Use a manual macOS session instead: Tier
  2 evidence runs only through the env-gated temporary-keychain test on a real
  macOS session. A re-enabled lane must not use the debug/test file credential
  store, and its output stays private-data-free. Keep any future workflow
  key-crate-only: it copies `crates/koushi-key` to `$RUNNER_TEMP` and runs
  `cargo test --manifest-path` there, so it must not require the private
  vendored Matrix SDK submodule. For a manual macOS session
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

The former real-server startup-latency lane is retired. Local startup timing
should use disposable homeservers and synthetic accounts; historical details
remain in [the observability note](../qa/startup-latency-observability.md).

## Windows updater packaging validation

The optional `updater_validation` input on `build-windows.yml` builds an NSIS
updater artifact using disposable keys. It requires no release environment,
uses no production keys, uploads no artifacts, and writes no releases or tags.
The script verifies the actual installer with pinned official minisign and
requires `windows_updater_package_signature=ok`,
`windows_updater_tampered_package=reject`, `windows_updater_wrong_key=reject`,
and `windows_updater_validation=ok`, followed by process exit 0. Expected
verifier rejection exits must not become the overall process exit. Private
signer/build output and disposable
keys are deleted in `finally`; failure output contains only the phase.
See the [desktop release runbook](../releases/desktop-release.md) for dispatch.

## Local CLI

The deterministic CLI lane starts a disposable local Tuwunel, signs a fresh
synthetic account into the production headless MCP host, seeds an encrypted
room through Core, and runs the shipped `koushi` binary. It requires no model
or API key. Generated credentials enter the QA host through a private FIFO,
never argv or environment values. Process payloads remain in memory; only fixed success/failure tokens
are printed, and all processes and temporary fixtures are cleaned up on exit.

```bash
KACHE_EXPLAIN_MISS=1 cargo build -p koushi-qa --features qa-bin --bin headless-core-qa --locked
KACHE_EXPLAIN_MISS=1 cargo build -p koushi-mcp --bin koushi --locked
node scripts/cli-local-qa.mjs
```

With CI-profile binaries use `node scripts/cli-local-qa.mjs --ci` and set
`CARGO_TARGET_DIR` when the target root differs. Required tokens:
`cli_accounts=ok`, `cli_encrypted_read=ok`, `cli_encrypted_search=ok`.
The PR Tuwunel job enforces this lane. `MCP_HOST_READY` means the snapshot the
bridge serves already lists the seeded room by name as encrypted: the host runs
behind the QA proxy, withholds Sliding Sync responses while seeding (create and
send settle on their own HTTP responses, but the name arrives only by sync),
then waits for that room-list projection. Host fixtures enable
`KOUSHI_QA_CLI_FIXTURE=1` only in the existing debug/test-gated QA host mode;
this is not a product credential injection or executable option.
