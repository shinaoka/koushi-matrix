# Headless Basic Operations QA

This contract covers three lanes:

- Local headless QA, verified through `CoreCommand`/`CoreEvent` only.
- matrix.org compatibility QA, using the same core operations with a reduced
  token set and one login per run.
- Linux virtual-display client QA, using the real Tauri client under Xvfb.

Current GUI-operation policy: room creation, space creation, replies, and other
destructive Matrix operations are iterated only against disposable local
Conduit/Tuwunel homeservers. matrix.org is a final compatibility gate, not a
GUI development or retry loop.

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
room_space=ok
timeline=ok
reply=ok
thread_hidden=ok
thread_summary=ok
thread_recv=ok
thread_paginate=end_reached
edit_redact_search=ok
restore_cleanup=ok
```

`thread_summary=ok` is a strict Phase 11 signal: local core QA fails if the
server/SDK path does not surface a root `thread_summary` for the threaded
reply.

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

`local-settings` opens the real Settings UI, changes the composer send shortcut,
and switches the theme to dark. It waits for the controls to reflect the
Rust-owned settings snapshot (`aria-pressed="true"`) and for `data-theme="dark"`
to be applied from that snapshot, not from localStorage.

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
gui_local_reply=ok
gui_local_settings=ok
```
