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
invite_recv=ok
invite_accept=ok
invite_decline=ok
dm_start=ok
room_space=ok
timeline=ok
reply=ok
thread_hidden=ok
thread_summary=ok
thread_recv=ok
thread_paginate=end_reached
send_media=ok
recv_media=ok
edit_redact_search=ok
e2ee_trust=ok
restore_cleanup=ok
```

`thread_summary=ok` is a strict Phase 11 signal: local core QA fails if the
server/SDK path does not surface a root `thread_summary` for the threaded
reply.

`send_media=ok` and `recv_media=ok` are the Phase A media/file state-machine
signals. The core lane sends a synthetic file through
`TimelineCommand::UploadAndSendMedia`, observes Rust-owned upload progress and
local-echo media metadata, receives the event on the second account timeline,
and downloads it through a Rust-only effect that emits only byte-count
completion. The lane must not print filenames, MXC URIs, room IDs, event IDs,
media bytes, encrypted media keys/hashes, or raw SDK errors.

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
key-backup enable, wrong-secret restore failure, successful restore on a second
same-user device, SAS verification, and identity reset through
`CoreCommand`/`CoreEvent` only. The runner must not print account keys,
verification target user/device ids, backup versions, room ids, event ids,
recovery secrets, or raw SDK errors for this stage. It is a separate Rust-owned
trust proof and runs after the ordinary room/timeline/search operations in the
aggregate local lane. The local runner registers separate synthetic users for
each core backend leg so the trust proof is not affected by devices created by
the SDK smoke lane.

For room/space checks, the core lane performs bounded `SyncOnce` refreshes
before asserting `rooms` vs `spaces`. Local homeservers can briefly report a
newly created or joined space as a plain room until the create state is folded
into the room-list projection.

Focused local proof:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=e2ee_trust --core --core-backend=probed --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=invites_dm --core --core-backend=probed --timeout-ms=240000
npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=media --core --core-backend=probed --timeout-ms=240000
```

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
gui_local_settings=ok
gui_local_trust_settings=ok
```
