# Session Trust Discovery Timeout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent stored-session restoration from waiting forever while discovering verification methods, and make the pending and failed states observable and retryable.

**Architecture:** `AccountActor` owns a bounded verification-method discovery task and reports a typed success/failure result through its mailbox. The reducer projects failures into the existing retryable provisional state, while the React gate renders phase-specific copy and routes Retry back to a fresh actor-owned discovery.

**Tech Stack:** Rust, Tokio, koushi-state reducer, koushi-core actor, React, TypeScript, Vitest, Testing Library.

## Global Constraints

- Do not delete or migrate user data or Keychain entries.
- Do not bypass the verification gate or promote an unverified session.
- Preserve generation and serial correlation for every asynchronous completion.
- Diagnostics may contain only numeric correlation values, elapsed time, booleans, and closed tokens.
- Use tests first and observe each regression test fail before production changes.

---

### Task 1: Reducer failure projection

**Files:**
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/mod.rs`
- Modify: `crates/koushi-state/src/reducer/session.rs`
- Test: `crates/koushi-state/tests/session_state.rs`

**Interfaces:**
- Produces: `AppAction::VerificationMethodDiscoveryFailed { generation: u64, kind: VerificationGateFailureKind }`
- Produces: a reducer transition from `ProvisionalPhase::DiscoveringMethods` to `ProvisionalPhase::RecheckingTrust { failure: Some(kind) }`

- [ ] **Step 1: Write the failing reducer test**

Add a test that dispatches `VerificationMethodDiscoveryFailed { generation: 3, kind: Timeout }` from `DiscoveringMethods`, asserts the retryable provisional state, and proves the same action is ignored from `CheckingTrust`, `AwaitingVerification`, and `Ready`.

- [ ] **Step 2: Run the reducer test and verify RED**

Run: `cargo test -p koushi-state --test session_state verification_method_discovery_failure_is_retryable_and_phase_scoped`

Expected: compilation fails because `VerificationMethodDiscoveryFailed` does not exist.

- [ ] **Step 3: Add the minimal action and reducer handler**

Add the action variant, route it in `reducer/mod.rs`, and implement a handler that accepts only `ProvisionalPhase::DiscoveringMethods`, preserves `SessionInfo`, and emits only `UiEvent::SessionChanged`.

- [ ] **Step 4: Run the reducer test and verify GREEN**

Run the command from Step 2 and expect one passing test.

### Task 2: Bounded actor-owned discovery and diagnostics

**Files:**
- Modify: `crates/koushi-core/src/account.rs`

**Interfaces:**
- Produces: `VerificationMethodDiscoveryResult::{Discovered(VerificationGateState), Failed(VerificationGateFailureKind)}`
- Produces: `wait_for_verification_method_discovery(Duration, Future)`
- Updates: `AccountMessage::VerificationMethodsDiscovered` to carry the typed result
- Produces diagnostics under `core.verification_method_discovery`

- [ ] **Step 1: Write failing timeout and correlation tests**

Add tests proving a pending future returns `Failed(Timeout)` under a millisecond deadline, a known gate returns `Discovered`, an unknown gate returns `Failed(Sdk)`, and stale generation/serial remains rejected by `method_discovery_is_current`.

- [ ] **Step 2: Run the focused core tests and verify RED**

Run: `cargo test -p koushi-core verification_method_discovery --lib`

Expected: compilation fails because the result type and bounded helper do not exist.

- [ ] **Step 3: Implement the bounded task and typed mailbox result**

Add `VERIFICATION_METHOD_DISCOVERY_TIMEOUT`, the typed result, and the generic timeout helper. Update `discover_verification_methods` to cancel the prior owned task, increment serial, record `started`, await the bounded helper, record `finished`, and send the typed result. Update mailbox handling to record receipt, enforce the existing correlation guard, dispatch success or `VerificationMethodDiscoveryFailed`, and record ignored/failure stages.

- [ ] **Step 4: Add cancellation diagnostics**

Centralize owned discovery cancellation so replacement and provisional teardown emit `cancelled` with generation and serial before aborting the task.

- [ ] **Step 5: Make Retry restart discovery**

Update `RetryCurrentDeviceTrustDiscovery` so an unpromoted session with authoritative `Unverified` trust starts a fresh bounded discovery; other trust states retain the authoritative trust recheck behavior.

- [ ] **Step 6: Run the focused core tests and verify GREEN**

Run the command from Step 2 and expect all matching tests to pass.

### Task 3: Phase-specific UI and Retry

**Files:**
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/App.tsx`
- Test: `apps/desktop/src/SessionVerificationGate.test.tsx`
- Test: `apps/desktop/src/i18n/messages.test.ts`

**Interfaces:**
- Produces translation key: `gate.discovering`
- Extends `SessionVerificationGate.operations` with `retryCurrentDeviceTrustDiscovery(): Promise<DesktopSnapshot>`

- [ ] **Step 1: Write failing rendering and Retry tests**

Add table-driven tests for `checkingTrust`, `discoveringMethods`, and failed `recheckingTrust`. Assert distinct English copy, Retry absent during initial checking, Retry present during discovery/failure, and duplicate clicks construct only one retry promise.

- [ ] **Step 2: Run the focused Vitest file and verify RED**

Run: `npm --prefix apps/desktop test -- --run src/SessionVerificationGate.test.tsx`

Expected: assertions fail because all provisional phases use `gate.checking` and discovery has no Retry.

- [ ] **Step 3: Implement phase-specific rendering and injected Retry operation**

Add English and Japanese `gate.discovering` messages. Derive explicit phase booleans in `SessionVerificationGate`, render the matching copy, expose Retry for discovery/recheck, and route it through the existing single-flight `run("recovery", ...)` guard.

- [ ] **Step 4: Run focused UI and i18n tests and verify GREEN**

Run:

`npm --prefix apps/desktop test -- --run src/SessionVerificationGate.test.tsx src/i18n/messages.test.ts`

Expected: all selected tests pass.

### Task 4: Verification and publication

**Files:**
- Modify only files listed above plus this plan/spec.

- [ ] **Step 1: Run Rust verification**

Run:

- `cargo test -p koushi-state --test session_state`
- `cargo test -p koushi-core --lib`
- `cargo fmt --all -- --check`
- `cargo clippy -p koushi-state -p koushi-core --all-targets -- -D warnings`

- [ ] **Step 2: Run desktop verification**

Run:

- `npm --prefix apps/desktop test -- --run src/SessionVerificationGate.test.tsx src/i18n/messages.test.ts src/App.test.tsx`
- `npm --prefix apps/desktop run typecheck`
- `npm --prefix apps/desktop run lint`

- [ ] **Step 3: Review the diff and run secret checks**

Run `git diff --check`, inspect the complete diff, and run `npm --prefix apps/desktop run qa:secret-scan`.

- [ ] **Step 4: Commit implementation**

Stage only the scoped files and commit with `Fix stalled session trust discovery`.

- [ ] **Step 5: Request code review and address findings**

Apply the requesting-code-review workflow, rerun affected tests after every correction, and keep unrelated untracked files untouched.

- [ ] **Step 6: Push and create the PR**

Push `codex/session-trust-discovery-timeout`, create a ready PR describing the reproduced release-build failure and TDD coverage, then monitor all GitHub checks.

- [ ] **Step 7: Resolve CI and review feedback, then merge**

Use the GitHub CI/review workflows for any failures or comments. Merge only after required checks pass, review threads are resolved, and the PR reports mergeable state.
