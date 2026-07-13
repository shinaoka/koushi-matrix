# Verification Preparation Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a stalled verification admission visible in both Koushi Diagnostics and stderr without exposing private Matrix data.

**Architecture:** Add a narrow dual-sink helper in `koushi-core` that accepts only structured `DiagnosticEvent` values. Instrument the owned promotion task and its correlated completion/cancellation handlers; retain the existing state-machine behavior and closed SDK failure classification.

**Tech Stack:** Rust, Tokio task lifecycle, `koushi-diagnostics`, Cargo tests

## Global Constraints

- Do not log user IDs, room IDs, homeserver URLs, tokens, recovery material, SDK error strings, or message content.
- Do not change verification admission behavior or add a timeout in this change.
- Use test-first development and preserve stale-completion and cancellation semantics.

---

### Task 1: Dual-sink structured admission diagnostics

**Files:**
- Modify: `crates/koushi-core/src/account.rs`
- Test: `crates/koushi-core/src/account.rs`

**Interfaces:**
- Consumes: `koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel}`
- Produces: `record_verification_admission_event(event: DiagnosticEvent)`

- [ ] **Step 1: Write the failing test**

Add a child-process test mode that invokes `record_verification_admission_event` with source `core.verification_admission`, stage `full_state_sync_finished`, numeric correlation fields, `success=false`, and an elapsed duration. Assert the event appears in `koushi_diagnostics::snapshot()` and the formatted event appears on captured stderr.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --lib verification_admission_diagnostic_records_and_writes_stderr -- --exact --nocapture`

Expected: FAIL because `record_verification_admission_event` and the child test mode do not exist.

- [ ] **Step 3: Write minimal implementation**

Implement `record_verification_admission_event` so it formats the structured event once, emits `[koushi] <source> <formatted-event>` with `eprintln!`, and records the original event in `koushi_diagnostics`.

- [ ] **Step 4: Run test to verify it passes**

Run the command from Step 2.

Expected: PASS; stderr and the in-memory snapshot describe the same structured event.

- [ ] **Step 5: Commit**

```bash
git add crates/koushi-core/src/account.rs
git commit -m "Add verification admission diagnostic sink"
```

### Task 2: Instrument promotion preparation lifecycle

**Files:**
- Modify: `crates/koushi-core/src/account.rs`
- Test: `crates/koushi-core/src/account.rs`

**Interfaces:**
- Consumes: `record_verification_admission_event(event: DiagnosticEvent)`
- Produces: structured stages for start, sync result, completion, stale discard, cancellation, and projection dispatch

- [ ] **Step 1: Write failing lifecycle tests**

Extend the existing controlled promotion tests to clear/snapshot the diagnostic buffer around unique transition IDs and assert the ordered stage subsequences for success, failure, stale completion, and cancellation. Assert serialized records do not contain synthetic private user, room, homeserver, token, recovery-key, or SDK-error strings.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p koushi-core --lib promotion_ -- --nocapture`

Expected: FAIL because the lifecycle stages are not recorded.

- [ ] **Step 3: Add lifecycle events**

Record `preparation_started` before spawning, `full_state_sync_started` immediately before the SDK await, and `full_state_sync_finished` immediately after it with `success` and `elapsed_ms`. Record `completion_received` on actor delivery, `completion_ignored` before every correlation-rejection return, `preparation_cancelled` before abort/join, and `ready_projection_dispatched` before the verified projection action.

- [ ] **Step 4: Run focused and full verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-core --lib
cargo test -p koushi-state --test session_state
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
git diff --check
```

Expected: all commands pass.

- [ ] **Step 5: Commit**

```bash
git add crates/koushi-core/src/account.rs
git commit -m "Trace verification room preparation"
```
