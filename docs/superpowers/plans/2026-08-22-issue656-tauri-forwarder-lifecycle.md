# Issue #656 Tauri Core-event Forwarder Lifecycle

## Scope

Fix the existing `apps/desktop/src-tauri/src/lib.rs` core-event forwarder owner before #551 decomposition. Do not extract modules, alter the183-command registry, or change CoreCommand/CoreEvent/DTO/wire names.

## Defects

1. `CoreConnection::recv_event()` represents closed broadcast as `EventStreamLag { skipped: 0 }`; the forwarder treats it as recoverable lag forever and spawns replay tasks in a tight loop.
2. The main forwarder handle is discarded, so Tauri managed state cannot abort it on teardown.
3. window destruction recognizes the stop boundary but submits no `AppCommand::Shutdown`.
4. the forwarder updates a leaked `Box::leak` timeline counter while QA commands read a separate managed-state counter.

## Design

- Add a private pure `forwarder_lag_disposition(EventStreamLag)` decision: zero means `ResyncAndStop`, positive means `ResyncAndReplay`. The loop preserves one final resync/marker before defensive zero-sentinel exit. Today `CoreRuntime` retains the broadcast sender and the forwarder's connection retains a command sender, so managed-state `Drop` abort is the primary production terminator; the zero branch defends future/live runtime closure and prevents any closed-stream storm.
- Make `submit_timeline_replay_after_forwarder_lag` async and await its existing2-second timeout inline. Do not create detached replay tasks. The bounded stall after a positive lag is intentional: latest-snapshot resync preserves correctness and a second lag may trigger another bounded cycle.
- `spawn_core_event_forwarder` returns a small lifecycle-owning `CoreEventForwarderTask` containing Tauri's `JoinHandle<()>`; its `Drop` aborts unfinished work. Construct the shared counter, spawn, then build/manage `CoreRuntimeState` with `forwarder_task: Some(task)`; the contract-test construction uses `None`. This is an owner, not a wrapper abstraction.
- Replace the leaked counter and duplicate counter with one `Arc<AtomicUsize>` stored in `CoreRuntimeState` and cloned into the forwarder. Existing QA reads and event update logic remain unchanged.
- Make `AppCommand::Shutdown` authoritative in `AppActor`: a private command disposition stops draining at the first Shutdown, publishes any preceding coalesced command state, then exits through the existing single epilogue that flushes drafts and shuts down `AccountActor`. Duplicate Shutdown and commands queued after the first are intentionally not handled; later sends fail `RuntimeClosed`. Shutdown remains exempt from Ready-session admission. Do not change any other command semantics or wire shape.
- On the existing main-window destruction predicate, call one `submit_core_shutdown(app_handle)` helper. It allocates a request ID from the managed connection and submits the now-authoritative `CoreCommand::App(AppCommand::Shutdown { request_id })`. macOS close-to-hide remains excluded because it prevents close and returns before destruction.
- OIDC/startup/focus tasks are short command submissions and remain out of scope.

## Verify first

1. RED pure forwarder-disposition tests: positive lag requires resync+replay; zero sentinel requires resync+stop.
2. RED Core runtime tests: signed-out Shutdown must complete the AppActor shutdown handle; a preceding command queued in the same drain must publish its state before completion; duplicate Shutdown must stop at the first disposition and complete the single epilogue once. Pin draft-flush/AccountActor cleanup and tolerate `RuntimeClosed` at the Tauri submission race.
3. RED source/ownership tests: forwarder spawn result must be stored, no `Box::leak`, replay helper contains no nested spawn, destruction path submits Shutdown. Update the existing source split that keys on `Err(_lag)` without weakening replay-after-marker, and assert zero-lag state+marker emission precedes loop exit.
4. Existing lag serialization/order and timeline count tests remain exact. The zero sentinel result type stays unchanged.
5. Focused Core and Tauri tests must repeat at least three times.

## Implementation evidence

- RED: signed-out Shutdown timed out waiting for AppActor; Tauri disposition/source tests failed to compile with four missing production symbols.
- GREEN focused x3: two Core shutdown tests and three Tauri disposition/replay/window tests. Affected suites: Core lib1,023 passed/8 ignored; Tauri150 passed/1 ignored plus keyring5.
- Static verifier:183-command registry and `serialize_core_event` exact; no `Box::leak` or nested replay spawn; one Arc counter; owned task; closed disposition; Destroyed Shutdown; only the three approved files changed.
- Post-implementation full-diff review: `reviewer-flash` `Correct-to-merge`; no blocking findings. The documented concurrent-submit cutoff and best-effort Destroyed task race are accepted teardown semantics.
- Final local matrix: Core lib1,023/8 ignored, Tauri150/1 ignored plus keyring5, Vitest1,429, Playwright248, workspace all-targets, Headless Core QA130, wasm state/search, typecheck/lint/build, SDK/docs, Tauri/domain/IPC boundaries, secret/release/version, rustfmt, `cargo deny`, `cargo machete`, source/diff checks green without reruns.

## Invariants

- Final snapshot/ResyncMarker still emits once on a live closed stream; managed-state teardown may abort immediately because no WebView remains.
- Positive lag still requests `ReplaySubscribed` after marker with the same request identity and timeout.
- No raw SDK error, secret, callback URL, message body, or new diagnostic payload.
- No new registry, callback bag, compatibility shim, detached task collection, or unbounded state.

## Gates

- `reviewer-flash` design verdict: `Correct-to-implement` after two rounds; all causal, ordering, duplicate-shutdown, setup, and test findings incorporated.
- Verify-first RED/GREEN and static ownership/wire checks.
- `reviewer-flash` full-diff `Correct-to-merge`.
- Full local matrix, CI7/7, latest-main merge, #656/#551 evidence, cleanup.
