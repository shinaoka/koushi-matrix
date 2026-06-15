# Headless Basic Operations QA

This contract covers three lanes:

- Local headless QA, verified through `CoreCommand`/`CoreEvent` only.
- matrix.org compatibility QA, using the same core operations with a reduced
  token set and one login per run.
- Linux virtual-display client QA, using the real Tauri client under Xvfb.

Current GUI-operation policy: room creation, space creation, replies,
media/file sends and downloads, and other destructive Matrix operations are
iterated only against disposable local Conduit/Tuwunel homeservers. matrix.org
is a final compatibility gate, not a GUI development or retry loop.

## Local homeserver binary search path

Local homeserver runners start `conduit` and `tuwunel` by command name, using
the sanitized child process `PATH`. They do not probe a hidden absolute-path
list. The effective search order is the child process `PATH` order after any
agent-added prepends. Put disposable QA binaries in front of `PATH` before
local runs:

```text
/tmp/matrix-desktop-local-qa-bin        host fast lane, preferred
/tmp/matrix-desktop-local-qa-bin-test   host fallback/test binaries
/usr/local/bin                          Docker lane inside the committed image
%TEMP%\matrix-desktop-local-qa-bin      Windows/manual equivalent
existing PATH entries                   searched after the QA bin directories
```

The paths are operational search locations only; they are not product state,
not secrets, and not committed artifacts.

## Local headless lane

Run:

```bash
npm --prefix apps/desktop run qa:headless-basic:local
```

This lane runs against disposable local homeservers and must prove the full
basic-operations scenario set.

Required success tokens:

```text
safety=ok
login_sync=ok
credential_health=ok
fail_closed=ok
notification_candidate=ok
badge_state=ok
suppress_focus=ok
clear_badge=ok
invite_recv=ok
invite_accept=ok
invite_decline=ok
dm_start=ok
room_space=ok
directory_query=ok
directory_join=ok
room_settings=ok
moderation=ok
permission_guard=ok
timeline=ok
activity_recent=ok
activity_unread=ok
activity_markread=ok
reply=ok
reply_quote=ok
pin_event=ok
pinned_state=ok
unpin_event=ok
mention_send=ok
markdown_send=ok
slash_command=ok
ime_guard=ok
thread_hidden=ok
thread_summary=ok
thread_recv=ok
thread_paginate=end_reached
send_media=ok
recv_media=ok
read_receipt=ok
fully_read=ok
typing=ok
presence=ok
live_signals=ok
edit_redact_search=ok
send_fail=ok
resend=ok
cancel_send=ok
fifo=ok
unsent_restart=ok
e2ee_trust=ok
restore_cleanup=ok
```

`thread_summary=ok` is a strict Phase 11 signal: local core QA fails if the
server/SDK path does not surface a root `thread_summary` for the threaded
reply.

`reply_quote=ok`, `pin_event=ok`, `pinned_state=ok`, and `unpin_event=ok` are
the Phase A message-interaction proof. The core lane projects reply quote DTOs
and routes pin/unpin through Rust-owned room state before any GUI affordance is
considered.

`directory_query=ok` and `directory_join=ok` are the Phase A public-directory
proof. The core lane creates a disposable public alias room through a Rust core
command, queries the homeserver public directory through `RoomCommand`, and
joins by alias/server through Rust-owned directory state. The lane must not
print room IDs, aliases, server names, query text, pagination tokens, or raw SDK
errors as success output.

`room_settings=ok`, `moderation=ok`, and `permission_guard=ok` are the Phase A
room-management proof. The core lane creates a disposable management room,
loads Rust-owned settings/permission facts, updates a setting through
`RoomCommand`, rejects an unauthorized moderation command before SDK mutation,
and performs an authorized moderation action. The lane must not print room IDs,
user IDs, room names/topics, reasons, avatar URLs, or raw SDK errors as success
output.

`mention_send=ok`, `markdown_send=ok`, `slash_command=ok`, and `ime_guard=ok`
are the Phase A composer-semantics proof. The core lane sends typed
`MentionIntent` data through `TimelineCommand::SendText`, builds markdown/html
and `/me` emote content in Rust before SDK send, and verifies composing Enter
resolves to `CommitImeCandidate` rather than send or autocomplete acceptance.
The composer stage prints only these tokens and must not print mentioned Matrix
IDs, message bodies, raw SDK errors, or composer transaction/event IDs.

`send_media=ok` and `recv_media=ok` are the Phase A media/file state-machine
signals. The core lane sends a synthetic file through
`TimelineCommand::UploadAndSendMedia`, observes Rust-owned upload progress and
local-echo media metadata, receives the event on the second account timeline,
and downloads it through a Rust-only effect that emits only byte-count
completion. The lane must not print filenames, MXC URIs, room IDs, event IDs,
media bytes, encrypted media keys/hashes, or raw SDK errors.

`read_receipt=ok`, `fully_read=ok`, `typing=ok`, `presence=ok`, and
`live_signals=ok` are the Phase A live-signal state-machine proof. The core
lane sends a read receipt, sets the fully-read marker, sends a typing notice,
and records a Rust-owned presence value through `CoreCommand`/`CoreEvent` and
`AppState.live_signals`. The current presence proof is a Rust-owned command and
snapshot contract; full network presence propagation stays in the Rust sync
backend policy when the SDK API path is finalized. This stage must not print
Matrix room IDs, event IDs, user IDs, message bodies, or raw SDK errors. On
local SyncService homeserver legs, the typing check may use one bounded
debug/test `SyncOnce` on the observer account after `SetTyping` is acknowledged
because local sliding-sync typing delivery does not reliably wake the SDK room
typing observer. That nudge is part of the headless QA harness only; product
sync policy remains Rust-owned.

`activity_recent=ok`, `activity_unread=ok`, and `activity_markread=ok` are the
Phase A account-wide Activity proof. The core lane opens Activity through
`CoreCommand`, verifies Rust-owned Recent and Unread streams from timeline
observations plus room unread facts, and clears unread rows only through the
Rust mark-read substate. This stage must not print Matrix room IDs, event IDs,
sender IDs, message previews, pagination tokens, or raw SDK errors.

`send_fail=ok`, `resend=ok`, `cancel_send=ok`, `fifo=ok`, and
`unsent_restart=ok` are the Phase A outbound send-queue proof. The core lane
inserts a local TCP proxy between the Rust runtime and the disposable
homeserver, drops proxy traffic to create recoverable SDK send failures, then
proves Rust-owned `TimelineItem.send_state`, guarded retry/cancel commands,
FIFO completion, and unsent local-echo survival across runtime restart. This
stage must not print Matrix room IDs, event IDs, SDK transaction IDs, message
bodies, raw SDK errors, or proxy connection details.

The Phase B browser-headless proof lives in
`apps/desktop/e2e/basic-operations.spec.ts`. It seeds Rust-shaped
`TimelineItem.send_state` values through the app harness CoreEvent stream,
clicks the inline resend/delete/cancel controls and room-level resend bar, and
asserts only typed IPC dispatch plus later CoreEvent-driven DOM changes. React
must not relabel a `TimelineItemId::Transaction` row as failed/sending without
`send_state`, nor repair the row after a command response.

`invite_recv=ok`, `invite_accept=ok`, `invite_decline=ok`, and `dm_start=ok`
are the Phase A invite/DM state-machine proof. The core lane proves incoming
room/space invite projection into Rust-owned `AppState.invites`, accept/decline
commands, and direct-message creation/invite projection through
`CoreCommand`/`CoreEvent` only. This stage must not print Matrix room IDs, user
IDs, invite display names, or raw SDK errors. The SyncService room-list observer
must consume non-left entries so invited-room diffs wake the Rust projection
loop; joined-only observation can leave the invite snapshot stale.

`e2ee_trust=ok` is the Phase A E2EE trust signal. The core lane proves
cross-signing bootstrap, encrypted seed-room backup upload, passphrase-backed
key-backup enable, wrong-secret restore failure, successful joined-room restore
on a second same-user device, SAS verification, and identity reset through
`CoreCommand`/`CoreEvent` only. The runner must not print account keys,
verification target user/device ids, backup versions, room ids, event ids,
recovery secrets, or raw SDK errors for this stage. It is a separate Rust-owned
trust proof and runs after the ordinary room/timeline/search operations in the
aggregate local lane. The local runner registers separate synthetic users for
each core backend leg so the trust proof is not affected by devices created by
the SDK smoke lane. Until a broader SDK/public API path exists, "successful
restore" means recovery secret import plus currently joined-room key hydration;
the token must not be described as exhaustive backup-wide restore.

For room/space checks, the core lane performs bounded `SyncOnce` refreshes
before asserting `rooms` vs `spaces`. Local homeservers can briefly report a
newly created or joined space as a plain room until the create state is folded
into the room-list projection.

Focused local proof:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=e2ee_trust --core --core-backend=probed --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=invites_dm --core --core-backend=probed --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=directory --core --core-backend=both --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=room_management --core --core-backend=both --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=media --core --core-backend=probed --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=live_signals --core --core-backend=probed --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=activity --core --core-backend=both --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=composer --core --core-backend=both --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=credential_health --core --core-backend=both --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=native_attention --core --core-backend=both --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=send_queue --core --core-backend=both --timeout-ms=240000
```

Core Batch A adds focused Phase A token contracts before the aggregate lane is
expanded. These tokens are private-data-free and become required only in the
slice that implements the corresponding state machine:

```text
credential_health=ok
fail_closed=ok
notification_candidate=ok
badge_state=ok
suppress_focus=ok
clear_badge=ok
activity_recent=ok
activity_unread=ok
activity_markread=ok
ja_catalog=ok
cjk_normalize=ok
cjk_collation=ok
joined_room_restore=ok
```

`credential_health=ok` / `fail_closed=ok` prove StoreActor-fed, Rust-owned
`LocalEncryptionState` transitions and kind-only credential-store failure
projection. The headless lane runs under the debug/test file credential-store
guard and must refuse to touch the OS keychain.
For Settings/Security Phase B, the browser-headless harness seeds Rust-shaped
`AppState.local_encryption` snapshots and Linux/macOS/Windows platform profiles.
It proves credential-store label/status rendering, recovery/reset affordance
visibility, and `probe_local_encryption_health` / `reset_local_data` dispatch.
React must not read OS/keyring errors, infer fail-open behavior, repair health
locally after a click, or clean stores through a React-local logout path. The
typed Rust reset command is the only GUI path for clearing current-account local
persistence and returning to a local signed-out snapshot.
`notification_candidate=ok`, `badge_state=ok`, `suppress_focus=ok`, and
`clear_badge=ok` prove Rust-owned native attention candidates and platform
capability mapping without message bodies or identifiers.
`activity_recent=ok`, `activity_unread=ok`, and `activity_markread=ok` prove the
Rust-owned Activity projection and mark-read substate without leaking event
identity or previews. `ja_catalog=ok`, `cjk_normalize=ok`, `cjk_collation=ok`,
and `ime_guard=ok` prove Japanese/CJK catalog, normalization, ordering, and IME
send-vs-commit contracts.
`joined_room_restore=ok` proves the explicit #30 MVP restore scope from
Rust-observed joined-room hydration progress. It must not be described as a
backup-wide restore token.

## Headless browser IPC-contract lane

Run the full headless browser tier:

```bash
npm --prefix apps/desktop run test:ui-headless
```

Focused E2EE trust GUI proof:

```bash
cd apps/desktop && npx playwright test e2e/basic-operations.spec.ts -g "E2EE trust controls"
```

Focused media/file GUI proof:

```bash
npm --prefix apps/desktop run test:ui-headless -- e2e/basic-operations.spec.ts --grep "attach control"
```

This lane mounts the full React app over mocked Tauri IPC. For E2EE trust Phase
B, it seeds a Rust-shaped `e2ee_trust` snapshot, drives User Settings controls,
and asserts that the UI invokes the typed Tauri commands (`accept_verification`,
`enable_key_backup`, `bootstrap_cross_signing`, `reset_identity`,
`submit_identity_reset_password`) with Rust-owned flow ids. The test then checks
the returned snapshot state, not React-local state. The mocked IPC recorder must
redact password fields, and visible assertions must avoid verification target
user/device ids.

For media/file Phase B, the harness uses a plain hidden `<input type="file">`
and Playwright `setInputFiles()`; do not open a native file dialog in headless
tests. The GUI proof asserts `upload_media`/`download_media` command names and
synthetic arg shapes, then injects Rust-shaped `TimelineEvent` payloads to render
media metadata and progress. Local echo rows use the canonical transaction DOM
id prefix from `timelineItemDomId`, e.g. `data-item-id="txn:desktop-media-1"`.
Do not assert on or render MXC URIs, encrypted media keys/hashes, downloaded
bytes, real filenames, room IDs, or event IDs.

For Activity Phase B, the harness seeds Rust-shaped `AppState.activity`
snapshots. It proves the Activity rail entry, Recent/Unread tabs, row order as
provided by Rust, pagination cursor dispatch, focused-context row opens, and
mark-read command shapes. Viewing Unread must not dispatch mark-read, and rows
must remain visible after `mark_activity_read` until a later Rust-shaped
snapshot removes them. React must not sort, filter, synthesize unread
membership, or locally repair Activity streams.

## matrix.org compatibility lane

This lane is reserved for the last compatibility pass after local headless and
Linux virtual-display lanes are green. Do not use it while iterating on GUI
controls or UI state models.

Run:

```bash
npm --prefix apps/desktop run qa:headless-basic:real
```

This lane validates the same core flows against matrix.org with a bounded
compatibility subset. It must avoid OS keychain access, use one login per run,
and clean up created rooms, spaces, and sessions even after earlier failures.

The subset exercises room creation, space creation, space-child linking,
send/edit/redact/search, and — added once reply was proven on the local lanes
(roadmap Phase 15) — **reply** (`SendReply`). It leaves and forgets every
created room and space. Real-homeserver run tokens include `real_reply=ok`,
`leave_room=ok forget_room=ok`, and `real_space_cleanup=ok`.

The default scenario is `space_compat` (full cleanup-proving lane); `compat` is
a reduced debug subset. Required success tokens for the default `space_compat`
lane (the runner enforces these via `scripts/lib/qa-token-contract.mjs`, not just
the process exit code):

```text
login=ok
sync=running
qa_room=created
send_msg1=ok
send_search=ok
send_msg2=ok
real_reply=ok
edit_msg1=ok
redact_msg2=ok
search=ok
store_restore=ok
leave_room=ok
forget_room=ok
real_space_create=ok
real_space_child=ok
real_space_cleanup=ok
logout=ok
post_logout_restore=not_found
```

Real QA output is private-data-free: the runner additionally rejects any Matrix
identifier (`@user:server`, `!room:server`, `$event:server`) in the output.

## Linux virtual-display client lane

This lane exercises the real Linux Tauri client on a virtual display and uses
the local homeserver setup for product-integration verification.
The committed Docker lane image bundles the runnable `conduit` and
`tuwunel` binaries plus `zstd`/`unzstd`, so the local-homeserver scenarios can
run entirely inside the container.

Run the local-client lanes:

```bash
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-login --server=conduit --artifact-dir=artifacts/linux-gui-local-login --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=artifacts/linux-gui-local-send --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-room --server=conduit --artifact-dir=artifacts/linux-gui-local-create-room --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-space --server=conduit --artifact-dir=artifacts/linux-gui-local-create-space --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-invites-dm --server=conduit --artifact-dir=artifacts/linux-gui-local-invites-dm --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-reply --server=conduit --artifact-dir=artifacts/linux-gui-local-reply --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-media --server=conduit --artifact-dir=artifacts/linux-gui-local-media --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-room-tags --server=conduit --artifact-dir=artifacts/linux-gui-local-room-tags --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-room-management --server=conduit --artifact-dir=artifacts/linux-gui-local-room-management --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-activity --server=conduit --artifact-dir=artifacts/linux-gui-local-activity --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-explore --server=conduit --artifact-dir=artifacts/linux-gui-local-explore --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-message-actions --server=conduit --artifact-dir=artifacts/linux-gui-local-message-actions --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-composer --server=conduit --artifact-dir=artifacts/linux-gui-local-composer --timeout-ms=180000
npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-settings --server=conduit --artifact-dir=artifacts/linux-gui-local-settings --timeout-ms=180000
```

`local-login` proves the Linux client can boot against a disposable local
homeserver, complete FIFO-driven login, seed exactly one synthetic room on the
server side, and reach a ready synced UI with an active room. `local-send`
proves the actual composer can send one message through WebDriver and the QA
title reports `send=sent` with no errors.

`local-create-room`, `local-create-space`, and `local-reply` exercise the real
basic-operation controls through WebDriver against a disposable local
homeserver: clicking `Create room`/`Create space`, submitting the dialog, and
replying to a real timeline event. They assert the Rust-owned snapshot reacts
(`rooms`/`spaces` counts grow; a `data-reply="true"` row renders) rather than
relying on React-only state. These destructive operations stay local-only;
matrix.org is reserved for the final compatibility gate.

`local-media` writes a synthetic fixture file under the ignored scenario
artifact directory, sets that path on the Composer's hidden file input through
WebDriver, falls back to a browser `DataTransfer` file list when WebKit does not
populate `input.files`, waits for `timeline_room=true`, waits for a Rust-owned
`TimelineItem.media` row to render in the real Tauri WebView, then clicks the
rendered Download button and checks the QA title stays error-free. It prints
only `gui_local_media=ok`; it must not open native file dialogs, use
real/private filenames, print MXC URIs, expose downloaded bytes, monkeypatch
Tauri internals from WebDriver, or synthesize upload/download lifecycle state in
React.

`local-room-tags` opens the seeded synthetic room row's real context menu in the
Linux Tauri WebView, clicks `Add to Favourites`, waits for the row to move from
the Rooms section to Favourites from the Rust-owned `RoomSummary.tags` snapshot,
then clicks `Remove from Favourites` and waits for the row to return to Rooms.
The lane prints only `gui_local_room_tag_set=ok` and
`gui_local_room_tag_removed=ok`; it must not monkeypatch Tauri IPC, synthesize
React-local room-list membership, or print Matrix room IDs / raw SDK errors.

`local-room-management` seeds a helper member in the disposable local room,
opens the real Room info panel, edits the room topic through the right-panel
form, waits for the Rust-owned `AppState.room_management.settings.topic`
snapshot row to update, changes the helper's role through the real role
select and waits for the Rust-owned `settings.members[*].role` /
`power_level` snapshot to update, then clicks the Kick control and waits for
the room-scoped `settings.members` snapshot to remove the member row. The lane
prints only `gui_local_room_topic=ok`, `gui_local_room_role=ok`, and
`gui_local_room_kick=ok`; it must not monkeypatch Tauri IPC, synthesize
React-local settings/member state, or print Matrix room IDs, user IDs, room
names/topics, avatar URLs, moderation reasons, or raw SDK errors. Room avatar
URL editing is covered by the browser-headless command/snapshot test because
the local homeserver lane should not depend on reusable synthetic MXC media.

`local-activity` opens the real Activity entry in the Linux Tauri WebView and
switches between Unread and Recent tabs through the Tauri command path. Row
ordering, focused-context row jumps, and mark-read behavior are covered by the
browser-headless Activity proof and Rust core Activity scenario. The Linux lane
prints only `gui_local_activity_open=ok`, `gui_local_activity_unread_tab=ok`,
and `gui_local_activity_recent_tab=ok`; it must not monkeypatch Tauri IPC,
synthesize Activity rows in React, or print Matrix IDs, event IDs, message
bodies, pagination tokens, or raw SDK errors.

`local-explore` registers a synthetic helper account on the same disposable
homeserver, has that helper create one public room with a synthetic alias, then
drives the real Explore pane through WebDriver. It searches public rooms, waits
for Rust-owned directory results to render, clicks Join, and waits for the
joined room to appear in the Rust-owned room list. The lane prints only
`gui_local_explore_query=ok` and `gui_local_explore_join=ok`; it must not
monkeypatch Tauri IPC, synthesize directory results or joined rooms in React, or
print Matrix aliases, room IDs, server names, pagination tokens, or raw SDK
errors.

`local-message-actions` sends one synthetic message, opens the real hover-gated
message action menu in the Linux Tauri WebView, clicks View source, waits for
the Rust-owned `MessageSourceLoaded` DTO to render the Message source dialog,
then forwards the event to the Rust-snapshot destination room. The lane prints
only `gui_local_message_source=ok` and `gui_local_message_forward=ok`; it must
not monkeypatch Tauri IPC, generate Matrix permalinks in React, copy message
bodies through React for forwarding, or print Matrix IDs, message bodies,
generated permalinks, or raw SDK errors.

`local-composer` registers a synthetic helper account, gives it a synthetic
display name, joins it to the seeded local room, and sends one helper seed
message so the Rust-owned `ProfileState.users` projection can populate member
mention candidates. The lane drives the real composer controls in the Linux
Tauri WebView: type `@`, select the member autocomplete option, send the
mention, select text and click Bold, then send a slash command. It prints only
`gui_local_mention=ok`, `gui_local_markdown=ok`, and `gui_local_slash=ok`; it
must not monkeypatch Tauri IPC, synthesize `m.mentions` or formatted HTML in
React, print Matrix IDs, or treat DOM-local text insertion as enough evidence
before the Rust-owned send state reaches `send=sent` and the composer clears.

`local-invites-dm` registers a synthetic helper account on the same disposable
homeserver, has that helper create and invite the QA user to a synthetic room,
then drives the real Invites pane through WebDriver. It accepts the invite and
waits for the pending invite list to clear, then submits New DM to the helper
user and waits for a `data-room-kind="dm"` room-list row to appear. The lane
prints only `gui_local_invite_accept=ok` and `gui_local_dm_start=ok`; it must not
print Matrix IDs, room IDs, or raw SDK errors. This virtual-display smoke forces
the legacy sync backend for determinism; the local core `invites_dm` QA remains
the SyncService/legacy correctness gate for invite projection semantics.

`local-settings` opens the real Settings UI, changes the composer send shortcut,
switches the theme to dark, and verifies the E2EE trust settings section renders
inside the real Tauri WebView. It waits for the controls to reflect the
Rust-owned settings snapshot (`aria-pressed="true"`), for `data-theme="dark"` to
be applied from that snapshot, and for the trust section's localized labels to
appear without seeding React-only state.

For fast iteration, build the debug app once and reuse it with `--skip-build`
(optionally `--app-binary=PATH`):

```bash
npm --prefix apps/desktop run tauri build -- --debug --no-bundle
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-room --server=conduit --skip-build --artifact-dir=artifacts/linux-gui-local-create-room-fast --timeout-ms=180000
```

The combined Linux lane is exposed through the shared release aggregator:

```bash
npm --prefix apps/desktop run qa:linux-gui
```

Required target tokens:

```text
gui_local_login=ok
gui_local_send=ok
gui_local_create_room=ok
gui_local_create_space=ok
gui_local_invite_accept=ok
gui_local_dm_start=ok
gui_local_reply=ok
gui_local_media=ok
gui_local_room_tag_set=ok
gui_local_room_tag_removed=ok
gui_local_room_topic=ok
gui_local_room_role=ok
gui_local_room_kick=ok
gui_local_activity_open=ok
gui_local_activity_unread_tab=ok
gui_local_activity_recent_tab=ok
gui_local_message_source=ok
gui_local_message_forward=ok
gui_local_settings=ok
gui_local_trust_settings=ok
```
