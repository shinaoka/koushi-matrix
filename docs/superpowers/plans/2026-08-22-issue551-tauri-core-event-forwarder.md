# Issue #551 Tauri Core-event Forwarder

## Scope

Move the complete Tauri `CoreEvent` forwarding, lag recovery, webview packaging, serialization and focused wire tests into private `src/core_event_forwarder.rs`. Keep `CoreRuntimeState`, the shared QA counter field, runtime attachment/setup, Destroyed shutdown and command registry in `lib.rs`.

Baseline `d8602cbf`: `lib.rs` 3,519 lines / 149,052 bytes / SHA-256 `72b780092214c7e3147f2d693fd6a406723662893388c00d149aaf3def64cf03`.

## Exact move

Move three constants:

- `CORE_EVENT_NAME`, `STATE_EVENT_NAME`
- `CORE_FORWARDER_TIMELINE_REPLAY_TIMEOUT`

Move three types/impls:

- `ForwarderLagDisposition`
- `CoreEventForwarderTask` and its abort-on-Drop impl
- `ForwardedWebviewEvent`

Move nine functions exactly:

- `forwarder_lag_disposition`, `spawn_core_event_forwarder`, `submit_timeline_replay_after_forwarder_lag`
- `forwarded_webview_events_for_core_event`, `diffs_net_count_change`
- `forwarded_webview_events_for_state_changed`, `forwarded_webview_events_for_lag_resync`
- `emit_forwarded_webview_events`, `serialize_core_event`

The leaf owns exact Tauri emit/task imports, timeout, serde JSON, Core event/command types, `FrontendDesktopSnapshotDelta`, diagnostics, and the shared counter update logic. Remove now-unused parent imports for `FrontendDesktopSnapshotDelta`, `CoreCommandHandle`, `CoreEvent`, `EventStreamLag`, `SearchEvent`, `TimelineCommand`, `TimelineEvent`, and `AppStateSnapshot`; retain only names still used by root state/setup. Replay remains awaited and 2-second bounded; zero-lag closed sentinel still emits state+marker then exits; task Drop still aborts.

Parent imports `CoreEventForwarderTask` and `spawn_core_event_forwarder` with minimal `pub(super)` visibility. Preserve sibling compatibility with one explicit flat `pub(crate) use core_event_forwarder::CORE_EVENT_NAME`; no barrel/glob/default export. `CoreRuntimeState`, `_forwarder_task`, `timeline_items_count: Arc<AtomicUsize>`, setup attachment and ownership remain parent-owned.

## Tests

Move these nine tests exactly:

- seven forwarding/lag tests from `timeline_items_updated_forwarding_emits_core_event_name_and_all_diffs` through `lag_resync_forwarder_requests_core_timeline_replay_after_marker`;
- `core_event_wire_format_matches_checked_in_contract_artifact`;
- `core_event_contract_artifact_key_set_does_not_shrink`.

The leaf keeps the exact `#[cfg(test)]\nmod tests` marker. The lag source test reads both `include_str!("core_event_forwarder.rs")` and `include_str!("lib.rs")`: disposition/replay/marker/no-leak/no-nested-spawn and `struct CoreEventForwarderTask` assertions target the leaf, while `forwarder_task: Some` remains asserted against the parent setup owner. These are the only intentional body/source-reference deltas. The generated artifact relative path remains valid from the same source directory. Remove the forwarding-only parent test imports, including `AtomicUsize` while retaining `AtomicU64`/`Ordering`. Keep window shutdown source assertions in `lib.rs`.

## Invariants

- All Core event kind envelopes, skipped events and JSON shapes exact; checked-in artifact unchanged.
- Registry183, Tauri command names, public DTO/wire, secret/privacy behavior exact.
- One forwarder task owner and one shared counter; no new task, map, callback registry or state owner.
- OIDC, window lifecycle, bootstrap and command submission remain root-owned.

## Deterministic verifier

- constants3, types3+Drop, functions9 exact; tests8 exact and the lag source test exact except its two documented source-reference/ownership-target deltas;
- parent definitions0; one module/import group and one explicit constant re-export;
- `CoreRuntimeState`/setup/run/Destroyed shutdown/registry183 exact;
- generated artifact hash and `serialize_core_event` behavior exact;
- no glob/default export or resource/API delta.

Run focused forwarding/wire baseline/post x3, full Tauri, IPC/generated contracts, full local matrix, design/full-diff review, latest-main integration, CI7/7 and #551 evidence.

## Implementation evidence

- Immutable baseline core-event9/9 x3; post-move9/9 x3; full Tauri150/1 ignored plus keyring5 and IPC/generated contracts green.
- Exactness verifier: production constants/types/functions+Drop exact; tests8 exact; lag test only documented two-source deltas; parent definitions0; state/setup/run/registry183 and generated artifact exact; no glob.
- Metrics (`wc -l` newline count): parent3,519→1,337 lines; leaf2,181 (mostly exhaustive wire contract); combined3,518 (-1 line / +554 bytes). Content-line indexing including each file's final unterminated line reports1,338 +2,182 =3,520 (+1), reconciling the reviewer count.
- Post-implementation full-diff review: `reviewer-flash` `Correct-to-merge`; no blocking findings. Full matrix pending.

## Gates

- `reviewer-flash` fresh design verdict: `Correct-to-implement`; two-source blocker and all import/assertion precision findings incorporated.
- move-only exactness checks.
- `reviewer-flash` full-diff `Correct-to-merge`.
- merge and cleanup.
