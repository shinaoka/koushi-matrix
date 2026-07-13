# SAS Delivery Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one real-device run identify why an unverified Koushi session's own-user SAS request never appears in Element X.

**Architecture:** The SDK adapter records recipient resolution and the result of the API that constructs and sends the to-device request. `AccountActor` records request/SAS state observations, restricted-sync delivery, observer termination, timeout, and settlement using the same app-owned flow ID. Both layers use `koushi_diagnostics::record_and_stderr`; verification behavior and timing remain unchanged.

**Tech Stack:** Rust 2024, Matrix Rust SDK verification APIs, Tokio actor tasks, `koushi-diagnostics`, existing Rust unit and actor tests.

## Global Constraints

- Do not add retries, extend the 120-second timeout, or create a second verification/promotion path.
- Do not log user IDs, device IDs, display names, homeserver URLs, Matrix transaction IDs, access tokens, recovery material, SDK error strings, or message content.
- Diagnostic fields are limited to numeric flow correlation, counts, booleans, elapsed time, and closed enum tokens.
- Diagnostic emission must not change request ordering, awaits, cancellation, settlement, or promotion.
- Preserve the user's untracked `HANDOFF.md`, `apps/desktop/src/graphify-out/`, `docs/design/sidebar-dm-rooms-sort-mock.svg`, and `log`.

---

### Task 1: SDK recipient and send boundary

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`

**Interfaces:**
- Produces: `request_own_user_sas_verification(session: &MatrixClientSession, flow_id: u64) -> Result<MatrixOwnUserVerificationHandle, E2eeTrustError>`.
- Produces: privacy-safe `core.sas_verification` events for `request_started`, `recipients_resolved`, and `request_send_finished`.
- Preserves: `MatrixOwnUserVerificationHandle::eligible_device_count()` and opaque raw Matrix handles.

- [ ] **Step 1: Write failing SDK tests for the privacy-safe recipient predicate and event vocabulary**

Add tests beside `own_user_proof_eligibility_requires_distinct_owner_signed_unblocked_device`:

```rust
#[test]
fn own_user_request_recipient_requires_a_distinct_owner_signed_device() {
    assert!(super::is_own_user_verification_recipient("CURRENT", "OTHER", true));
    assert!(!super::is_own_user_verification_recipient("CURRENT", "CURRENT", true));
    assert!(!super::is_own_user_verification_recipient("CURRENT", "OTHER", false));
}

#[test]
fn sas_delivery_event_contains_only_closed_private_safe_fields() {
    let event = super::sas_delivery_event("recipients_resolved", 41)
        .field(koushi_diagnostics::DiagnosticField::count("other_device_count", 3))
        .field(koushi_diagnostics::DiagnosticField::count("recipient_count", 1));
    assert_eq!(event.source, "core.sas_verification");
    assert_eq!(koushi_diagnostics::format_event(&event),
        "stage=recipients_resolved flow_id=41 other_device_count=3 recipient_count=1");
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p koushi-sdk own_user_request_recipient_requires_a_distinct_owner_signed_device -- --exact
cargo test -p koushi-sdk sas_delivery_event_contains_only_closed_private_safe_fields -- --exact
```

Expected: compilation fails because `is_own_user_verification_recipient` and `sas_delivery_event` do not exist.

- [ ] **Step 3: Add the private-safe event and recipient helpers**

Add:

```rust
fn sas_delivery_event(stage: &'static str, flow_id: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.sas_verification", stage)
        .field(DiagnosticField::count("flow_id", flow_id))
}

fn record_sas_delivery_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

fn is_own_user_verification_recipient(
    current_device_id: &str,
    candidate_device_id: &str,
    cross_signed_by_owner: bool,
) -> bool {
    candidate_device_id != current_device_id && cross_signed_by_owner
}
```

Reuse `is_own_user_verification_recipient` inside `is_eligible_own_user_proof_device`, adding the existing `!blacklisted` gate only for UI capability eligibility.

- [ ] **Step 4: Instrument request construction and send completion**

Change `request_own_user_sas_verification` to accept `flow_id`. Record `request_started` before the keys query. After `get_user_devices`, compute `other_device_count`, `recipient_count` with the helper, and the existing unblocked `eligible_device_count`, then record `recipients_resolved`. Record `request_send_finished outcome=success initial_state=<closed token>` only after `identity.request_verification_with_methods(...).await` succeeds. On every early error, record `request_send_finished outcome=failed failure_stage=<identity_query|identity_missing|device_query|no_eligible_device|send>` before returning the existing closed `E2eeTrustError`.

Use this closed mapper:

```rust
fn verification_request_state_token(state: &MatrixVerificationRequestState) -> &'static str {
    match state {
        MatrixVerificationRequestState::Created => "created",
        MatrixVerificationRequestState::Requested => "requested",
        MatrixVerificationRequestState::Ready => "ready",
        MatrixVerificationRequestState::SasStarted(_) => "sas_started",
        MatrixVerificationRequestState::Done => "done",
        MatrixVerificationRequestState::Cancelled { .. } => "cancelled",
        MatrixVerificationRequestState::UnsupportedMethod => "unsupported_method",
    }
}
```

- [ ] **Step 5: Update the core caller and run SDK/core compilation tests**

Pass `flow_id` from `AccountActor::handle_start_own_user_sas`:

```rust
koushi_sdk::request_own_user_sas_verification(&session, flow_id).await
```

Run:

```bash
cargo test -p koushi-sdk own_user_request_recipient_requires_a_distinct_owner_signed_device -- --exact
cargo test -p koushi-sdk sas_delivery_event_contains_only_closed_private_safe_fields -- --exact
cargo test -q -p koushi-core --lib account::tests::restricted_sync_rechecks_only_current_unstarted_own_user_flow
```

Expected: all pass.

- [ ] **Step 6: Commit SDK boundary**

```bash
git add crates/koushi-sdk/src/lib.rs crates/koushi-core/src/account.rs
git commit -m "Trace SAS request delivery boundary"
```

---

### Task 2: Account-owned request and SAS lifecycle trace

**Files:**
- Modify: `crates/koushi-core/src/account.rs`

**Interfaces:**
- Consumes: `MatrixVerificationRequestState`, `MatrixVerificationCancelKind`, `MatrixSasState`, and the app-owned `flow_id`.
- Produces: `core.sas_verification` events for request/SAS start, state changes, observer termination, timeout, and settlement.

- [ ] **Step 1: Write failing token and dual-sink tests**

Add a table test asserting every request state, cancellation kind, SAS state, failure kind, and terminal variant maps to a closed snake-case token. Add a subprocess test mirroring `verification_admission_diagnostic_records_and_writes_stderr` with this required stderr fragment:

```text
[koushi] core.sas_verification stage=request_state_changed flow_id=41 state=cancelled cancel_kind=timeout cancelled_by_us=false
```

The child test must print `koushi_diagnostics::snapshot()` and the parent must assert that the same event is present in the bounded buffer.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p koushi-core --lib sas_verification_diagnostic_records_and_writes_stderr -- --exact
```

Expected: compilation fails because the SAS diagnostic helpers do not exist.

- [ ] **Step 3: Add closed-token helpers and the dual sink**

Add:

```rust
fn sas_verification_event(stage: &'static str, flow_id: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.sas_verification", stage)
        .field(DiagnosticField::count("flow_id", flow_id))
}

fn record_sas_verification_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}
```

Add exhaustive mappers `verification_request_state_token`, `verification_cancel_kind_token`, `sas_state_token`, `trust_failure_token`, and `verification_terminal_token`. These return only `&'static str`.

- [ ] **Step 4: Trace request and SAS start attempts**

In `handle_start_own_user_sas`, record `sas_start_attempted source=initial` immediately before `start_own_user_sas_verification`, followed by `sas_start_finished outcome=started|pending|failed`. In `recheck_own_user_sas_after_sync`, use `source=restricted_sync` and the same outcomes. Do not change when `store_sas_verification` is called.

- [ ] **Step 5: Trace observed request and SAS states**

At the start of `handle_verification_request_progress`, after stale-flow rejection, record `request_state_changed` with the state token. For `Cancelled`, also include `cancel_kind` and `cancelled_by_us`. At the start of `handle_sas_verification_progress`, after stale-flow rejection, record `sas_state_changed` with the SAS token. Record the current state once from `store_sas_verification` before projection so a handle that does not emit its initial value is still observable.

- [ ] **Step 6: Trace observer end, timeout, and settlement**

Handle `VerificationRequestObserverEnded` and `SasVerificationObserverEnded` separately so each records `observer_ended observer=request|sas`. Record `timeout_fired` only after `handle_sas_verification_timeout` confirms that the flow is active. At the start of `settle_verification`, after finding the active target, record `settled terminal=success|cancelled|failed`; add `reason` or `failure_kind` only from closed enums.

- [ ] **Step 7: Run lifecycle and dual-sink tests**

Run:

```bash
cargo test -p koushi-core --lib sas_verification_diagnostic_records_and_writes_stderr -- --exact
cargo test -p koushi-core --lib actor_sas_settlement_emits_exactly_one_terminal_and_clears_runtime -- --exact
cargo test -q -p koushi-core --lib
```

Expected: the focused tests and all core library tests pass.

- [ ] **Step 8: Commit actor lifecycle trace**

```bash
git add crates/koushi-core/src/account.rs
git commit -m "Trace SAS verification lifecycle"
```

---

### Task 3: Restricted-sync delivery trace

**Files:**
- Modify: `crates/koushi-core/src/account.rs`

**Interfaces:**
- Produces: `AccountMessage::RestrictedSyncFailed { generation }` for a coalesced failure streak.
- Produces: `restricted_sync_succeeded` or `restricted_sync_failed` only when a current own-user verification flow exists.

- [ ] **Step 1: Write failing correlation tests**

Extract and test:

```rust
fn active_own_user_sas_flow_for_restricted_sync(
    generation: u64,
    current_generation: u64,
    has_session: bool,
    session_promoted: bool,
    own_flow_id: Option<u64>,
) -> Option<u64>
```

The test must prove current provisional flow returns its ID, while stale generation, missing session, promoted session, and missing flow return `None`.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p koushi-core --lib restricted_sync_diagnostics_require_current_own_user_flow -- --exact
```

Expected: compilation fails because the correlation helper does not exist.

- [ ] **Step 3: Add failure-streak delivery without changing retry behavior**

Add `RestrictedSyncFailed { generation }`. In the provisional restricted-sync task, keep `let mut failure_reported = false`. On the first error in a consecutive failure streak, send `RestrictedSyncFailed` and set the flag. On a success, reset the flag before sending the existing `RestrictedSyncSucceeded`. Preserve the current 250ms sleep and all sync calls.

- [ ] **Step 4: Record only correlated sync outcomes**

In both message handlers, call `active_own_user_sas_flow_for_restricted_sync`. If it returns a flow ID, record `restricted_sync_succeeded` or `restricted_sync_failed`. Preserve the existing trust refresh and `recheck_own_user_sas_after_sync` decisions.

- [ ] **Step 5: Run focused and core tests**

Run:

```bash
cargo test -p koushi-core --lib restricted_sync_diagnostics_require_current_own_user_flow -- --exact
cargo test -p koushi-core --lib restricted_sync_rechecks_only_current_unstarted_own_user_flow -- --exact
cargo test -q -p koushi-core --lib
```

Expected: all pass.

- [ ] **Step 6: Commit restricted-sync trace**

```bash
git add crates/koushi-core/src/account.rs
git commit -m "Trace SAS restricted sync delivery"
```

---

### Task 4: Full regression and field-test handoff

**Files:**
- Verify only: all changed Rust crates and desktop release gates.

**Interfaces:**
- Produces: a clean branch ready for the user's Element X real-device rerun.

- [ ] **Step 1: Run Rust verification**

```bash
cargo test -p koushi-sdk
cargo test -q -p koushi-core --lib
cargo test -p koushi-diagnostics
cargo test -p koushi-state --test session_state
cargo test -q --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
```

Expected: all pass, with only pre-existing ignored tests.

- [ ] **Step 2: Run desktop verification and repository hygiene checks**

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop test -- --run
git diff --check
git status --short
```

Expected: TypeScript and Vitest pass; `git diff --check` is silent; status contains only the four preserved user-owned untracked paths.

- [ ] **Step 3: Inspect the resulting commit range**

```bash
git log --oneline origin/main..HEAD
git diff --stat origin/main...HEAD
```

Expected: issue #244 admission fixes plus the SAS observability design, plan, and implementation commits; no unrelated tracked files.

- [ ] **Step 4: Real-device run instructions**

Ask the user to restart the newly built Koushi with the same clean-data environment, press “Verify with another device,” and return all lines beginning with `[koushi] core.sas_verification`. Do not claim delivery is fixed until those logs show the failure boundary or Element X receives the request.
