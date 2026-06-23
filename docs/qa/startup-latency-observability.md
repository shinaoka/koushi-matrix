# Startup Latency Observability QA

## Purpose

This lane measures the wall-clock cost of the four startup macro-phases
(session restore, sync-to-ready, room-list, and room subscribe/paginate) on a
real Matrix homeserver account. It is the Phase A evidence gate for issue
#123 and uses the instrumentation wired in
[docs/superpowers/plans/2026-06-23-startup-latency-observability-phase-a.md](../superpowers/plans/2026-06-23-startup-latency-observability-phase-a.md).

The lane is **read-only**: it creates no rooms, sends no messages, and leaves
nothing on the homeserver (unless `KOUSHI_STARTUP_LAT_TEARDOWN=1` is set; see
Teardown below).

---

## Persistent-profile two-run model

Standard QA lanes use a fresh per-run data directory so no state bleeds across
runs. The `startup_latency` lane instead uses a **persistent profile dir**:

```
.local-secrets/real-account-qa/profile/startup_latency/
  data/        ← SQLite store, media cache, search index (persisted across runs)
  cred-store/  ← file-dir credential store (persisted across runs)
  logs/        ← per-run log files (run1-<ts>.log, run2-<ts>.log)
```

This directory is inside `.local-secrets/`, which is already git-ignored.

**Run 1 — populate:**  
Attempts `RestoreLastSession`; on a fresh profile dir this returns
`SessionNotFound` so the binary falls back to `LoginPassword`. The SQLite
store, event cache, and credential store are populated for run 2.  
Token emitted: `startup_lat restore=not_found login=fallback`

**Run 2 — measure:**  
Attempts `RestoreLastSession` against the populated profile. This is the
cold-restore path the timing tokens describe. Run 2 is the evidence run.  
Token emitted: `startup_lat restore=session`

**Rate-limit note:** run 1 performs ONE real login and creates ONE device on
the homeserver. Run 2 restores from the local store without a new login. The
device persists until `KOUSHI_STARTUP_LAT_TEARDOWN=1` is used.

---

## Command

```bash
npm --prefix apps/desktop run qa:real-homeserver -- --scenario=startup_latency
```

**Requires maintainer GO before the first run** (run 1 logs in to the
homeserver using `.local-secrets/real-account-qa/credentials.json`).

---

## Environment variables

| Variable | Required | Purpose |
|---|---|---|
| `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` | yes (set by runner) | Prevents OS keychain prompts; points at the persistent `cred-store/` dir |
| `KOUSHI_QA_DATA_DIR` | yes (set by runner) | Points at the persistent `data/` dir |
| `KOUSHI_STARTUP_TRACE` | yes, set to `1` (set by runner) | Enables the env-gated `koushi.startup` sub-phase and origin observer tokens |
| `KOUSHI_STARTUP_LAT_PAGES` | optional | Override number of paginate pages (default: 3) |
| `KOUSHI_STARTUP_LAT_TEARDOWN` | optional, `1` to enable | Log out and remove the QA device at teardown (see below) |

---

## Expected private-data-free tokens

All tokens are emitted to stdout/stderr. No Matrix IDs, user IDs, room IDs,
event IDs, message bodies, device IDs, or raw SDK errors appear.

### Macro-phase tokens (always present in run 2)

```
startup_lat restore=session
startup_lat phase=restore ms=<N>
startup_lat phase=sync_to_ready ms=<N>
startup_lat phase=room_list ms=<N> rooms=<count>
startup_latency=ok
```

### Subscribe/paginate tokens (present unless account has no joined non-DM room)

```
startup_lat phase=subscribe ms=<N>
startup_lat phase=paginate ms=<N> reached_start=true|false
startup_lat phase=paginate ms=<N> reached_start=true|false
...
startup_lat pages=<N> reached_start=true|false
```

If no joined non-DM room exists, the binary emits:
```
startup_lat subscribe=skipped reason=no_non_dm_room
```
and the runner accepts this as a valid sparse-account result.

### Sub-phase and origin tokens (emitted to stderr when `KOUSHI_STARTUP_TRACE=1`)

The `koushi-core` startup trace module emits finer-grained tokens:

```
koushi.startup phase=restore ms=<N>
koushi.startup phase=timeline_build ms=<N>
koushi.startup phase=timeline_subscribe ms=<N> items=<bucket>
koushi.startup phase=crawler_page ms=<N>
koushi.startup phase=paginate ms=<N> gate_ms=<N> reached_start=true|false
koushi.startup phase=origin origin=cache|network|sync
```

`origin=cache` means the SDK served timeline events from the local event
cache (warm restore path). `origin=network` means a `/messages` gap was
filled by a pagination request. `origin=sync` means events arrived via live
sync. A single room load may emit multiple origin tokens.

---

## Teardown

By default the QA device is **kept** on the homeserver so run 2+ can restore
rather than login (cheaper on rate limits). To remove the device:

```bash
KOUSHI_STARTUP_LAT_TEARDOWN=1 npm --prefix apps/desktop run qa:real-homeserver -- --scenario=startup_latency
```

This causes the binary to log out at the end of the run, removing the device.
The next invocation will treat the profile as unpopulated and run a fresh
login.

To fully reset the persistent profile, delete the profile dir:

```bash
rm -rf .local-secrets/real-account-qa/profile/startup_latency/
```

---

## Report template

Fill this table with the measured values from run 2. Do **not** commit real
timing data or any identifiers — this template stays as-is in the repository.

| phase | ms | notes |
|---|---|---|
| `restore` | — | session restore from SQLite store |
| `sync_to_ready` | — | sync start → running → Ready snapshot |
| `room_list` | — | first non-empty room-list snapshot |
| `subscribe` | — | timeline subscribe + initial items |
| `paginate` (page 1) | — | first backward paginate page |
| `paginate` (page 2) | — | second backward paginate page |
| `paginate` (page 3) | — | third backward paginate page |

| origin token | count | interpretation |
|---|---|---|
| `origin=cache` | — | events served from local event cache |
| `origin=network` | — | events fetched via `/messages` pagination |
| `origin=sync` | — | events arrived via live sync |
