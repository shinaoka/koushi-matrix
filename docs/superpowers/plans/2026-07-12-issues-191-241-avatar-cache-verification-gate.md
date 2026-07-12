# Issues #191 And #241 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist avatars in the encrypted SDK media cache and prevent any usable session until the SDK authoritatively verifies the current device.

**Architecture:** Avatar bytes remain SDK-owned at rest and are materialized into process-local renderable URLs on demand. Authentication creates an AccountActor-owned provisional SDK session with restricted crypto synchronization; a Rust state machine and authoritative SDK verification stream control promotion, rejection, later locking, and all normal command admission.

**Tech Stack:** Rust, matrix-rust-sdk, Tokio actor runtime, serde DTOs, Tauri v2, React/TypeScript, Vitest, Playwright, disposable local Matrix test servers.

---

## File Structure

- `docs/architecture/overview.md`: canonical provisional-session ownership and encrypted media-store contract.
- `docs/architecture/state-machine.md`: canonical session, verification, rejection, and trust-loss transitions.
- `docs/policies/engineering-rules.md`: privacy and QA requirements for provisional credentials and verification evidence.
- `docs/superpowers/specs/2026-06-22-issue-118-completion-design.md`: mark the obsolete media-encryption assumption as superseded.
- `crates/koushi-sdk/src/lib.rs`: encrypted media-store invariant, current-device verification probe/stream, proof-method discovery, outgoing current-session SAS wrapper.
- `crates/koushi-state/src/state/session.rs`: serializable provisional/gate/rejecting states and private-data-free method capabilities.
- `crates/koushi-state/src/action.rs`: guarded session/trust/bootstrap actions.
- `crates/koushi-state/src/reducer/{mod.rs,session.rs,e2ee.rs,sync.rs}`: transitions, strict Ready guard, projection cleanup, request correlation.
- `crates/koushi-state/tests/session_state.rs`: exhaustive reducer transition and command-admission matrix.
- `crates/koushi-core/src/{account.rs,command.rs,event.rs,runtime.rs}`: quarantined SDK session, restricted crypto sync, promotion/rejection/lock, strict command routing.
- `crates/koushi-core/tests/{runtime_session.rs,command_redaction.rs}`: runtime behavior, privacy, and peer-send policy.
- `apps/desktop/src-tauri/src/{dto.rs,commands/e2ee.rs,commands/mod.rs,lib.rs}`: transport-only DTO/commands and wire contract.
- `apps/desktop/src/{App.tsx,backend/client.ts,backend/browserFakeApi.ts,domain/types.ts,i18n/messages.ts}`: full-screen gate over Rust state and typed outgoing verification calls.
- `apps/desktop/src/**/*.test.ts{,x}` and `apps/desktop/e2e/`: browser-headless gate and no-main-shell evidence.

### Task 1: Canon-First Contracts

**Files:**
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`
- Modify: `docs/superpowers/specs/2026-06-22-issue-118-completion-design.md`
- Test: repository documentation checks discovered in `package.json` and `scripts/`

- [ ] Add the keyed SQLite media-store invariant: `cache_path` separation does not remove the `MatrixClientStoreKey`; SDK retention is the only persistent avatar-retention policy; no Koushi plaintext cache is permitted.
- [ ] Replace the canonical permissive `Ready | NeedsRecovery | Recovering` guard with `Ready` only and document `Provisional`, `AwaitingVerification`, `Verifying`, `Rejecting`, and `Ready -> Locked` transitions.
- [ ] Document restricted crypto sync as AccountActor-internal and explicitly forbid normal projection actors, attention, saved-active-session publication, and normal commands before promotion.
- [ ] Mark the conflicting #118 paragraph superseded by issue #241 and the pinned SDK keyed-media-store behavior.
- [ ] Run documentation/format checks, inspect `git diff --check`, and commit as `docs: require verified sessions and keyed media cache`.

### Task 2: #241 Encrypted Avatar Cache Regression

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/renderable_thumbnail.rs`
- Test: `crates/koushi-core/src/account.rs` tests or a focused `crates/koushi-core/tests/avatar_media_cache.rs`

- [ ] Write a behavioral test named `avatar_download_survives_restart_and_offline_via_keyed_sdk_media_store`. Use a synthetic MXC and local HTTP media response, open a keyed `MatrixClientStoreConfig`, fetch once online, drop the client, clear `RenderableThumbnailCache`, reopen the same store with the same key while the server is unavailable, and assert a non-`file://` `koushi-thumbnail://` ready result with no second request.
- [ ] Write `uncached_avatar_offline_preserves_network_failure` and assert the existing `AvatarThumbnailFailureKind::Network` result.
- [ ] Write structural/adapter coverage proving the cache-path `SqliteMediaStore` receives the same required key and no `avatar_thumbnails` plaintext path is created.
- [ ] Run the focused tests before production edits and record the expected RED: restart/offline hit fails because `get_media_content(..., false)` neither reads nor writes.
- [ ] Change only the avatar MXC path to SDK cache-enabled retrieval. Keep account actor coalescing, the process-local renderable cache, opaque URL generation, session isolation, and SDK retention unchanged.
- [ ] Run the focused tests GREEN, then `cargo test -p koushi-sdk --lib` and `cargo test -p koushi-core --lib renderable_thumbnail`.
- [ ] Commit as `fix: persist avatars in encrypted sdk media cache`.

### Task 3: Session Gate State And Reducer

**Files:**
- Modify: `crates/koushi-state/src/state/session.rs`
- Modify: `crates/koushi-state/src/state/mod.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/{mod.rs,session.rs,e2ee.rs,sync.rs}`
- Modify: reducers whose guards or projection-context matches enumerate session variants
- Test: `crates/koushi-state/tests/session_state.rs`

- [ ] Add failing table-driven tests for `SignedOut -> Authenticating/Restoring -> Provisional(CheckingTrust)`, `Provisional -> AwaitingVerification`, `AwaitingVerification -> Verifying`, authoritative `Verified -> Ready`, no-proof `-> Rejecting -> SignedOut`, and `Ready -> Locked` on trust loss.
- [ ] Add a failing exhaustive test proving every room/timeline/thread/search/send/directory/attention start action is a no-op outside exact `Ready`, while verification/recovery retry/sign-out actions remain accepted in their specific gate states.
- [ ] Add serialization and redacted-Debug tests proving capability DTOs expose booleans/enums only and never raw Matrix IDs, secrets, access tokens, or SDK errors.
- [ ] Run `cargo test -p koushi-state --test session_state` and confirm the new tests fail for the permissive machine.
- [ ] Introduce app-owned types equivalent to `ProvisionalPhase`, `VerificationMethodCapability`, `VerificationGateState`, and private-data-free reject/failure kinds. Do not embed recovery secrets or SDK handles.
- [ ] Make `is_session_ready` exact-Ready; add a narrowly named gate projection helper rather than widening readiness. Update exhaustive matches and clear all normal view/projection state on provisional entry, rejection, and trust loss.
- [ ] Ensure recovery/SAS terminal actions request an authoritative trust recheck instead of setting `Ready` themselves.
- [ ] Run the focused state test and full `cargo test -p koushi-state`; commit as `feat: model fail-closed session verification gate`.

### Task 4: SDK Trust And Method Ports

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Test: `crates/koushi-sdk/src/lib.rs` tests

- [ ] Add failing tests for mapping SDK `VerificationState::{Unknown,Verified,Unverified}` into an app-owned `CurrentDeviceTrustState`, including a stream whose initial value is observed before updates.
- [ ] Add failing tests for method discovery distinguishing from SDK facts: existing cross-signing identity with eligible verified other devices; recovery available; genuine missing identity; existing identity with no proof; unknown/error. Require a bounded key query and never infer a new identity from recovery state alone.
- [ ] Add a failing API-shape test for an outgoing own-user SAS request that keeps raw target IDs inside the adapter/core handle and returns only an opaque app flow ID.
- [ ] Run focused SDK tests and confirm RED due to missing wrappers.
- [ ] Wrap `client.encryption().verification_state()` and expose a current value plus update stream. Remove the `list_devices` current-device `verified = true` shortcut; use authoritative cross-signing verification.
- [ ] Implement proof-method discovery from public SDK identity/device/recovery APIs. Never infer a genuine new identity from a transient network error.
- [ ] Implement outgoing current-session SAS selection/request behind an adapter-owned handle without returning raw IDs.
- [ ] Run `cargo test -p koushi-sdk --lib`; commit as `feat: expose authoritative current-device trust`.

### Task 5: Quarantined Account Runtime

**Files:**
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Test: `crates/koushi-core/tests/runtime_session.rs`
- Test: `crates/koushi-core/src/account.rs` tests

- [ ] Invert the existing permissive login/recovery tests and add RED tests proving password, OIDC, and restore completion install only a provisional actor session, do not persist active credentials, do not start normal sync/room/timeline/search/attention actors, and reject all normal commands.
- [ ] Add RED tests for subscribe-before-current-value trust observation, verified restore promotion, Unknown staying gated, stale-generation trust updates ignored, and Ready trust loss stopping children and clearing projections.
- [ ] Add RED teardown tests proving no-proof rejection and sign-out cancel restricted sync/SAS handles, best-effort logout, erase provisional credentials and keyed stores, and acknowledge completion before `SignedOut`. Add a crash/restart test proving no active/provisional credential, saved-session index, or last-session pointer was persisted and restart returns SignedOut.
- [ ] Run focused runtime/account tests and verify RED against immediate `LoginSucceeded`/`StartSync`.
- [ ] Split authenticated-session installation from Ready promotion. AccountActor owns provisional SDK handles and a generation-tagged trust observer. Restricted sync may process crypto/account/to-device data but must not publish normal room data.
- [ ] Emit state actions/events for gate discovery and authoritative trust only. On Verified, promote atomically, persist active session, then start normal actors and sync. On later non-Verified, lock first, stop normal actors, and clear views.
- [ ] Make runtime `is_ready_session_for_commands`, draft/navigation/scheduled persistence keys, current timeline keys, notification and native-attention paths exact-Ready. Allow only the explicit gate command family while provisional.
- [ ] Run `cargo test -p koushi-core --test runtime_session`, focused account tests, then `cargo test -p koushi-core --lib`; commit as `feat: quarantine sessions until device verification`.

### Task 6: Recovery, SAS, Rejection, And New-Account Bootstrap

**Files:**
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/{command.rs,event.rs,runtime.rs}`
- Modify: `crates/koushi-state/src/{action.rs,state/session.rs,reducer/e2ee.rs}`
- Test: `crates/koushi-core/tests/runtime_session.rs`
- Test: `crates/koushi-state/tests/session_state.rs`

- [ ] Add RED state/core tests for recovery success, invalid secret, cancel, retry, and restart; each successful SDK operation must remain gated until a subsequent authoritative Verified observation.
- [ ] Add RED two-device SAS tests for request, accept/start, seven emoji, match, mismatch, cancel, timeout, remote reject and retry; SAS Done must trigger trust recheck rather than direct promotion.
- [ ] Add RED no-proof tests for an existing identity and cleanup. Assert there is no identity-reset/skip/verify-later command accepted by the gate.
- [ ] Add RED new-account tests for cross-signing bootstrap, secure recovery-key delivery, explicit saved confirmation, and final trust recheck. Do not treat an SDK/network Unknown state as a new identity.
- [ ] Implement handle lifetimes and request correlation in AccountActor, keeping all SDK/raw flow and device identifiers out of state/events.
- [ ] Implement bootstrap confirmation as Rust-owned state. Reuse the existing secret/destination wrappers and zeroizing behavior; do not put recovery key text in React/reducer state.
- [ ] Run the focused suites plus `cargo test -p koushi-core --lib` and `cargo test -p koushi-state`; commit as `feat: complete mandatory device verification flows`.

### Task 7: Peer Trust Remains Non-Blocking

**Files:**
- Modify only if required: SDK/core send policy files identified by current send path
- Test: focused core/SDK encrypted-send tests
- Test: `apps/desktop/src` modal/render tests

- [ ] Add a RED-or-characterization test proving a Ready current device sends encrypted messages to eligible unverified peer devices without a verification prompt.
- [ ] Add characterization tests proving blocked devices, sender/key mismatch, and cryptographic integrity failures remain rejected.
- [ ] Amend the older #31/integration-edge-cases verified-user send-ack expectation where it contradicts #191; scope any remaining acknowledgement to integrity/blocked failures rather than merely unverified peer devices.
- [ ] Search normal send UI for verify/send-anyway prompts and add a browser assertion that none renders during ordinary composition/send.
- [ ] If the existing SDK policy already satisfies these tests, make no production change and commit tests only. If not, change the owning SDK/core policy, not React.
- [ ] Run focused encrypted-send and browser tests; commit as `test: keep peer trust non-blocking` or `fix: separate peer and current-device trust`.

### Task 8: Tauri Wire Contract

**Files:**
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/commands/e2ee.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: checked-in core event/IPC artifacts generated by repository scripts
- Test: Tauri command/DTO tests

- [ ] Add RED DTO snapshots for every provisional/gate/verifying/rejecting state and private-data-free capability/failure projection.
- [ ] Add RED command tests for outgoing current-session verification, retry trust discovery, recovery, SAS actions, bootstrap save confirmation, and sign out; assert all secrets use redacted wrapper Debug.
- [ ] Run focused Tauri tests and verify RED.
- [ ] Mirror Rust state mechanically and route typed commands only. Add no Matrix SDK logic or trust decisions in Tauri.
- [ ] Regenerate the checked-in wire artifact with the repository generator and inspect semantic changes.
- [ ] Run `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib` and the IPC contract command; commit as `feat: expose verification gate transport`.

### Task 9: Full-Screen Desktop Gate

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Test: `apps/desktop/src/App.test.tsx`
- Test: related backend/model tests
- Test: `apps/desktop/e2e/session-verification-gate.spec.ts`

- [ ] Add RED component/browser tests proving provisional states render a full-screen gate and do not mount the sidebar, room list, timeline, composer, search, notification/attention consumers, or main shell.
- [ ] Add RED tests for method-specific actions, seven emoji, mismatch/cancel/retry/sign-out, no skip/reset controls, no-proof rejection, bootstrap save confirmation, and later Ready-to-Locked replacement of the shell.
- [ ] Add RED tests proving the recovery secret text is not copied into React state, snapshots, browser fake logs, screenshots, or diagnostics; only input-presence may be ephemeral.
- [ ] Run focused Vitest/Playwright tests and verify RED.
- [ ] Add typed client methods and render the gate solely from Rust DTOs. Keep the textarea uncontrolled/ephemeral and pass the secret directly on submit. Browser fake transitions must mirror the Rust machine.
- [ ] Add localized English/Japanese catalog strings. Remove the recovery side-panel route for non-Ready sessions without removing Ready-session security settings.
- [ ] Run focused tests, frontend typecheck and lint; commit as `feat: block desktop shell on device verification`.

### Task 10: Integrated Privacy And Acceptance Gates

**Files:**
- Modify: local-server QA scenarios/scripts and docs only where required by the repository QA contract
- Test: Rust workspace, Tauri, frontend, Playwright, local server QA

- [ ] Add or update a private-data-free headless local-server scenario for verified restore, provisional rejection, recovery, two-device SAS, and trust loss. Use synthetic IDs/content and assert explicit success tokens.
- [ ] Run all focused acceptance tests from Tasks 2-9 and save their exact pass counts for the PR body.
- [ ] Run repository format, `git diff --check`, Rust workspace/core/Tauri suites, frontend unit tests, typecheck, ESLint, IPC contract, and relevant Playwright/local-server gates.
- [ ] Generate `git diff origin/main...HEAD`, run the repository-prescribed external `codex review -`, and evaluate every finding against the canon. Luna fixes all verified Critical/Important issues RED-first; reviewers re-review until approved.
- [ ] Rebase or merge current `origin/main` into the batch branch without rewriting published history, rerun affected gates, push once, and create one PR with `Closes #241` and `Closes #191` only when every acceptance criterion has direct evidence.
- [ ] Wait for GitHub Actions. Inspect and repair any failing check from its logs, rerun locally, push, and wait again.
- [ ] Merge with `gh pr merge <number> --merge --delete-branch`, never squash. Verify the merge commit is on `origin/main`, the PR is MERGED, and both issues are CLOSED.
