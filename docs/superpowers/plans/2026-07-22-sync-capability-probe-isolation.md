# Sync Capability Probe Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep the authenticated MSC4186 invite-list capability check while preventing its failures from invalidating or refreshing the authoritative desktop session, then stop immediately before attended Mac/matrix.org testing.

**Architecture:** `koushi-sdk` will build a disposable no-store, no-refresh Matrix client from only the authoritative access token and session metadata, run the existing typed bounded request there, and return the same closed support enum. `koushi-core` will pass `MatrixClientSession` to that boundary and retain `Unknown -> LegacySync`, with a deterministic test proving the first legacy response reaches `Running` without locking or stopping the session.

**Tech Stack:** Rust 2024, Tokio, matrix-rust-sdk public APIs, existing synthetic TCP/wiremock test helpers, Cargo unit tests, repository documentation gates.

## Global Constraints

- The authoritative store-backed `matrix_sdk::Client` remains the sole owner of product authentication, token refresh, session-change observation, encryption state, room state, and sync.
- The probe client receives an in-memory access-token copy and user/device metadata only; it receives no refresh token and uses no persistent store.
- Probe HTTP, parse, timeout, `M_UNKNOWN_TOKEN`, build, and restore failures map to `Unknown`; omission of the requested list maps to `KnownIncomplete`.
- `KnownIncomplete` and `Unknown` select `LegacySync`; `Supported` alone selects `SyncService` after `/versions` advertises MSC4186.
- Do not weaken genuine authoritative `SessionChange::UnknownToken -> SessionLocked` behavior.
- Do not add retries, polling, server-family fingerprinting, a second sync owner, raw errors, homeserver identities, or secret-bearing diagnostics.
- Do not change reducers, Core wire types, Tauri DTOs, React, persisted-session formats, or the vendored Matrix SDK.
- Follow RED-GREEN-REFACTOR and capture the expected RED output before production edits.
- Stop after deterministic, crate, formatting, lint/documentation, and headless gates; do not launch the macOS GUI or connect an agent-run process to a real account.

---

### Task 1: Isolate the SDK capability probe

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs:2822-2830,4749-4794,7917-8245`

**Interfaces:**
- Consumes: `MatrixClientSession`, `matrix_sdk::Client::homeserver`, `session_meta`, and `access_token`.
- Produces: `pub async fn probe_sliding_sync_invite_list_support(session: &MatrixClientSession) -> MatrixSlidingSyncInviteListSupport`.
- Keeps internal: `async fn send_sliding_sync_invite_list_probe(client: &matrix_sdk::Client) -> MatrixSlidingSyncInviteListSupport` and a disposable-client constructor returning `Option<matrix_sdk::Client>` or an equivalent closed private result.

- [ ] **Step 1: Add the failing authoritative-session isolation test**

Extend the synthetic invite-probe response fixture with an `M_UNKNOWN_TOKEN` response. Add a test named `sliding_sync_invite_probe_unknown_token_isolated_from_authoritative_session` that:

```rust
let authoritative = authenticated_probe_client(homeserver).await;
let before = authoritative.session_tokens().expect("authoritative tokens");
let mut changes = authoritative.subscribe_to_session_changes();
let session = MatrixClientSession::from_client_for_testing(
    authoritative.clone(),
    SessionInfo {
        homeserver: authoritative.homeserver().to_string(),
        user_id: "@probe:example.invalid".to_owned(),
        device_id: "PROBEDEVICE".to_owned(),
    },
);

assert_eq!(
    probe_sliding_sync_invite_list_support(&session).await,
    MatrixSlidingSyncInviteListSupport::Unknown,
);
assert!(matches!(changes.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
assert_eq!(authoritative.session_tokens().as_ref(), Some(&before));
```

The fixture must record all endpoint paths and finish after a bounded observation window so the test can additionally assert that no `/_matrix/client/v3/refresh` request occurred. Use only synthetic credentials bearing the repository secret-scan allowance comment.

- [ ] **Step 2: Run the isolation test and record RED**

Run:

```bash
cargo test -p koushi-sdk --lib sliding_sync_invite_probe_unknown_token_isolated_from_authoritative_session -- --nocapture
```

Expected: FAIL before production edits because the current public function accepts the authoritative `matrix_sdk::Client` and has no isolated-session boundary. If the test is temporarily expressed against the current signature to obtain behavioral RED, expected failure is an observed `SessionChange::UnknownToken`; change only the call site to the approved `MatrixClientSession` API before GREEN.

- [ ] **Step 3: Build the disposable no-refresh client**

Implement a private constructor with this behavior:

```rust
async fn build_sliding_sync_invite_probe_client(
    session: &MatrixClientSession,
) -> Option<matrix_sdk::Client> {
    use matrix_sdk::authentication::matrix::MatrixSession;
    use matrix_sdk_base::{SessionMeta, store::RoomLoadSettings};

    let authoritative = session.client();
    let meta = authoritative.session_meta()?.clone();
    let access_token = authoritative.access_token()?;
    let probe = matrix_sdk::Client::builder()
        .homeserver_url(authoritative.homeserver())
        .build()
        .await
        .ok()?;

    probe
        .matrix_auth()
        .restore_session(
            MatrixSession {
                meta: SessionMeta {
                    user_id: meta.user_id,
                    device_id: meta.device_id,
                },
                tokens: matrix_sdk::SessionTokens {
                    access_token,
                    refresh_token: None,
                },
            },
            RoomLoadSettings::default(),
        )
        .await
        .ok()?;
    Some(probe)
}
```

Do not call `desktop_client_builder_defaults`, `.handle_refresh_tokens()`, or any store-config helper. If the public SDK types allow `meta` to be used directly, avoid reconstructing it; preserve the same semantics.

- [ ] **Step 4: Split construction from the typed request and fail closed**

Change the public function to accept `&MatrixClientSession`, construct the disposable client, and return `Unknown` if construction/restoration fails. Move the existing typed request into the private send helper without changing request body, connection ID, list key, request retry policy, timeout, or response classification:

```rust
pub async fn probe_sliding_sync_invite_list_support(
    session: &MatrixClientSession,
) -> MatrixSlidingSyncInviteListSupport {
    let Some(probe) = build_sliding_sync_invite_probe_client(session).await else {
        return MatrixSlidingSyncInviteListSupport::Unknown;
    };
    send_sliding_sync_invite_list_probe(&probe).await
}
```

- [ ] **Step 5: Update existing probe tests to use session adapters**

Wrap each synthetic authenticated client with `MatrixClientSession::from_client_for_testing`. Replace the old stalled-refresh test with the approved invariant: the probe receives no refresh token, never calls refresh, stays within `SYNC_INVITE_PROBE_TIMEOUT` plus a small scheduling margin, and leaves authoritative tokens/session changes untouched. Preserve tests for exact authenticated request shape, supported empty list, omitted list, HTTP failure, malformed JSON, and timeout.

- [ ] **Step 6: Run focused SDK GREEN and format**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-sdk --lib sliding_sync_invite_probe -- --nocapture
```

Expected: formatting succeeds; all invite-probe tests pass, including isolation and zero refresh calls.

- [ ] **Step 7: Commit Task 1**

```bash
git add crates/koushi-sdk/src/lib.rs
git commit -m "fix(sync): isolate authenticated capability probe"
```

---

### Task 2: Route Core through the isolated boundary and prove continuation

**Files:**
- Modify: `crates/koushi-core/src/sync.rs:862-943,1822-1870,1939-3545`

**Interfaces:**
- Consumes: Task 1's `probe_sliding_sync_invite_list_support(&MatrixClientSession)`.
- Produces: private `async fn probe_backend(session: &MatrixClientSession) -> BackendProbeResult`.
- Preserves: `BackendProbeReason`, `backend_from_invite_list_support`, `FirstResponseCommitted`, and all existing `SyncEvent`/`AppAction` shapes.

- [ ] **Step 1: Add a failing Core continuation test**

Add `unknown_isolated_invite_probe_falls_back_and_commits_first_legacy_response`. Its synthetic server must return, in order:

```text
GET /_matrix/client/versions -> MSC4186 advertised
POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync -> indeterminate probe result
GET /_matrix/client/v3/sync -> valid empty sync response with next_batch
```

Drive a real `SyncActor` start with a `MatrixClientSession`. Assert the emitted backend is `LegacySync`, receive `SyncActorControl::FirstResponseCommitted`, process it through the actor, and assert `SyncLifecycle::Running` plus `SyncEvent::Running`. Assert there is no `Stopped` or auth-failure event. The SDK test in Task 1 owns the separate proof that the authoritative session-change stream stays empty. Reuse existing actor/server helpers rather than adding sleeps or a second sync implementation.

- [ ] **Step 2: Run the Core test and record RED**

Run:

```bash
cargo test -p koushi-core --lib unknown_isolated_invite_probe_falls_back_and_commits_first_legacy_response -- --nocapture
```

Expected: FAIL before Core production edits because `probe_backend` still passes the authoritative client to the SDK probe or because the Task 1 API no longer matches.

- [ ] **Step 3: Change the backend-selection boundary**

Change the private probe signature and calls as follows:

```rust
async fn probe_backend(session: &MatrixClientSession) -> BackendProbeResult {
    #[cfg(any(debug_assertions, test))]
    if forced_legacy_backend() {
        return BackendProbeResult {
            backend: SyncBackendKind::LegacySync,
            reason: BackendProbeReason::ForcedLegacy,
        };
    }

    let client = session.client();
    let versions = client.available_sliding_sync_versions().await;
    if versions.is_empty() {
        return BackendProbeResult {
            backend: SyncBackendKind::LegacySync,
            reason: BackendProbeReason::SlidingSyncUnavailable,
        };
    }

    let support = koushi_sdk::probe_sliding_sync_invite_list_support(session).await;
    let reason = match support {
        MatrixSlidingSyncInviteListSupport::Supported => BackendProbeReason::InviteListSupported,
        MatrixSlidingSyncInviteListSupport::KnownIncomplete => {
            BackendProbeReason::InviteListKnownIncomplete
        }
        MatrixSlidingSyncInviteListSupport::Unknown => BackendProbeReason::InviteListUnknown,
    };
    BackendProbeResult {
        backend: backend_from_invite_list_support(support),
        reason,
    }
}
```

In `handle_start`, call `probe_backend(&self.session).await`; obtain a clone of the authoritative client only for starting the selected backend after the probe result is known. Do not pass that client into the SDK capability function.

- [ ] **Step 4: Update existing backend-probe unit tests**

Wrap synthetic clients in `MatrixClientSession` and preserve the assertions that `/versions` precedes the invite-list request, advertised supported behavior selects SyncService, and omitted/unknown behavior selects LegacySync with the correct closed reason token.

- [ ] **Step 5: Run focused Core GREEN and related lifecycle tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-core --lib unknown_isolated_invite_probe_falls_back_and_commits_first_legacy_response -- --nocapture
cargo test -p koushi-core --lib backend_probe -- --nocapture
cargo test -p koushi-core --lib legacy_lifecycle_first_response -- --nocapture
cargo test -p koushi-core --lib stale_legacy_first_response_cannot_promote_a_replacement_run -- --nocapture
```

Expected: all focused tests pass without fixed-delay assertions.

- [ ] **Step 6: Commit Task 2**

```bash
git add crates/koushi-core/src/sync.rs
git commit -m "fix(sync): preserve session across probe fallback"
```

---

### Task 3: Harden the rule and complete pre-Mac verification

**Files:**
- Modify: `docs/policies/engineering-rules.md` at the authenticated capability-probe rule.
- Verify: `crates/koushi-sdk/src/lib.rs`, `crates/koushi-core/src/sync.rs`, documentation and repository gates.

**Interfaces:**
- Consumes: Tasks 1-2's isolated probe and Core continuation behavior.
- Produces: durable policy text and deterministic evidence ready for attended Mac/matrix.org verification.

- [ ] **Step 1: Amend the capability-probe rule**

Extend the existing rule that begins “A protocol version advertisement is not proof…” with this binding text:

```text
A non-authoritative authenticated probe must isolate session-change delivery,
access-token-expiry state, token refresh, rotating refresh credentials, and
persistent stores from the authoritative client. Supply no refresh token to a
disposable probe client. Probe failure is a backend-selection fact and must
not itself cause a product authentication-state transition.
```

Keep the existing requirements for one end-to-end deadline, no server-family fingerprinting, no second polling/sync owner, and success/omission/error/timeout coverage.

- [ ] **Step 2: Run diff and secret-safety checks**

Run:

```bash
git diff --check
node scripts/check-sdk-submodule.mjs
node scripts/check-secrets.mjs --all
```

Expected: no whitespace errors, SDK gitlink/path guard passes, and secret scan passes.

- [ ] **Step 3: Run affected crate suites**

Run:

```bash
cargo test -p koushi-sdk --lib
cargo test -p koushi-core --lib
```

Expected: both complete with zero failures.

- [ ] **Step 4: Run compile, formatting, and frontend contract gates**

Run:

```bash
cargo fmt --all -- --check
cargo check --workspace
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop test -- --run
```

Expected: every command exits zero. Frontend gates must remain unchanged by the Rust-only product fix.

- [ ] **Step 5: Run the required local headless sync gate**

Run the repository's current deterministic local homeserver command documented for sync-affecting changes:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both
```

Expected: Conduit and Tuwunel headless scenarios exit zero. If the repository exposes a narrower documented sync/restore preflight required before the full lane, run that fail-fast command first; it supplements rather than replaces the final `--server=both` gate.

- [ ] **Step 6: Commit policy and verification record**

Commit the policy change. Do not add raw logs containing identifiers or credentials. The implementer report records commands and private-data-free pass/fail counts outside the tracked source tree.

```bash
git add docs/policies/engineering-rules.md
git commit -m "docs: isolate authenticated capability probes"
```

- [ ] **Step 7: Stop before attended Mac testing**

Report the exact branch/commit, deterministic gate results, and the remaining attended check:

```text
Launch Koushi on macOS with the existing saved matrix.org account; verify the
diagnostic ordering reaches probe_done -> backend legacy -> first response
committed -> Sync Running, with no intervening session_invalidated/SessionLocked.
```

Do not launch the GUI, read the OS keychain, or perform the real-account check from the agent session.
