# Headless Basic Operations QA

This contract covers three lanes:

- Local headless QA, verified through `CoreCommand`/`CoreEvent` only.
- matrix.org compatibility QA, using the same core operations with a reduced
  token set and one login per run.
- Linux virtual-display client QA, using the real Tauri client under Xvfb.

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
thread=ok
edit_redact_search=ok
restore_cleanup=ok
```

## matrix.org compatibility lane

Run:

```bash
npm --prefix apps/desktop run qa:headless-basic:real
```

This lane validates the same core flows against matrix.org with a bounded
compatibility subset. It must avoid OS keychain access, use one login per run,
and clean up created rooms, spaces, and sessions even after earlier failures.

Required success tokens for the compatibility subset:

```text
safety=ok
login_sync=ok
timeline=ok
edit_redact_search=ok
restore_cleanup=ok
```

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
```

`local-login` proves the Linux client can boot against a disposable local
homeserver, complete FIFO-driven login, seed exactly one synthetic room on the
server side, and reach a ready synced UI with an active room. `local-send`
proves the actual composer can send one message through WebDriver and the QA
title reports `send=sent` with no errors.

The combined Linux lane is exposed through the shared release aggregator:

```bash
npm --prefix apps/desktop run qa:linux-gui
```

Required target tokens:

```text
gui_local_login=ok
gui_local_send=ok
```
