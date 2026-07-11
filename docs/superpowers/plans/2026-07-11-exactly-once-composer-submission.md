# Exactly-Once Composer Submission Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correlate each main/reply composer submission with one opaque `SubmissionId`, accept it at most once in Rust, and settle the frontend only from matching accepted/rejected state.

**Architecture:** A private-data-safe typed ID crosses TypeScript, Tauri, `TimelineCommand`, timeline actor, and reducer unchanged. The reducer owns acceptance/deduplication and terminal release; Tauri waits for the correlated result instead of returning an immediate snapshot. A shared frontend submission controller owns synchronous resolve/submit guards and clears drafts only after correlated acceptance.

**Tech Stack:** Rust reducers and actors, Tokio/Tauri IPC, TypeScript/React, Vitest, Cargo tests.

---

### Task 1: Canonical reducer lifecycle

**Files:** `docs/architecture/state-machine.md`, `crates/koushi-state/src/action.rs`, `crates/koushi-state/src/state/timeline.rs`, `crates/koushi-state/src/reducer/timeline.rs`, `crates/koushi-state/src/reducer/thread.rs`, `crates/koushi-state/tests/timeline_thread_state.rs`

- [ ] Add RED reducer tests proving duplicate `SubmissionId` produces one accepted pending transaction and stale/duplicate terminal actions do nothing.
- [ ] Add an opaque typed `SubmissionId` with redacted `Debug`, carry it in submission and terminal actions, and store the active ID beside the pending transaction.
- [ ] Clear drafts only on the first matching accepted transition; release only for matching terminal completion/failure/cancel.
- [ ] Run `cargo test -p koushi-state --test timeline_thread_state` and commit.

### Task 2: Core actor at-most-once enqueue

**Files:** `crates/koushi-core/src/command.rs`, `crates/koushi-core/src/timeline.rs`, `crates/koushi-core/tests/runtime_timeline.rs`

- [ ] Add RED actor tests injecting an SDK enqueue counter and issuing the same submission twice.
- [ ] Carry `SubmissionId` through `TimelineCommand` and actor messages; deduplicate before SDK enqueue while preserving the first transaction correlation.
- [ ] Emit typed accepted/rejected and terminal reducer actions carrying the same ID.
- [ ] Run focused core tests and commit.

### Task 3: Correlated Tauri settlement

**Files:** `apps/desktop/src-tauri/src/commands/mod.rs`, `apps/desktop/src-tauri/src/commands/timeline.rs`, `apps/desktop/src-tauri/src/state.rs`, command tests

- [ ] Add RED tests proving `send_text`/`send_reply` futures remain pending until matching accepted/rejected state and ignore unrelated snapshots.
- [ ] Add a typed submission response containing outcome, opaque ID, optional transaction ID, and snapshot; return typed timeout/disconnect failures without body or Matrix IDs.
- [ ] Register a waiter before command submission, settle it from matching core events, and remove it on every terminal path.
- [ ] Run Tauri focused tests and commit.

### Task 4: Shared frontend submission controller

**Files:** `apps/desktop/src/domain/composerSubmission.ts`, its tests, `apps/desktop/src/components/composer.tsx`, `apps/desktop/src/components/rightPanel.tsx`, `apps/desktop/src/App.tsx`, `apps/desktop/src/backend/client.ts`, browser fake API and contract types

- [ ] Add RED fake-timer tests for rapid Enter/delayed resolver, double click/invoke, rejection release, terminal release, and draft preservation until accepted.
- [ ] Implement one controller with synchronous `resolveInFlight`, `submitInFlight`, opaque ID generation, accepted/rejected correlation, and terminal release.
- [ ] Route main and reply sends through the controller; edit uses the resolve guard only; retry reuses the existing transaction and never creates a submission.
- [ ] Update IPC fakes/contracts/goldens and run frontend focused tests, typecheck, and lint; commit.

### Task 5: End-to-end contract verification

**Files:** IPC contract tests and generated artifacts identified by the contract check

- [ ] Add a RED end-to-end duplicate invocation test proving one resolver, invoke, reducer acceptance, transaction, and SDK enqueue.
- [ ] Regenerate checked-in wire artifacts using the repository generator.
- [ ] Run state/core/Tauri/frontend suites plus IPC contract checks and commit only generated contract changes.

### Task 6: Atomic admission and ambiguous IPC recovery

**Files:** `crates/koushi-core/src/timeline.rs`, `apps/desktop/src-tauri/src/commands/{mod.rs,timeline.rs}`, `apps/desktop/src/domain/composerSubmission.ts`, `apps/desktop/src/App.tsx`, `apps/desktop/src/backend/browserFakeApi.ts`, focused tests

- [x] Replace source-order assertions with deterministic closed/open/dropped permit behavior tests.
- [x] Reserve the actor mailbox with a one-shot permit and open it only after reducer acceptance delivery, ledger recording, and accepted-event emission.
- [x] Tombstone reducer-delivery failures as rejected so replay is deterministic and never reaches the SDK.
- [x] Preserve ID, captured target/payload, draft, and guard for timeout/disconnect/lag; explicit retry reuses them without an automatic loop.
- [x] Mirror accepted replay and terminal snapshot fields in the browser fake for main/reply/thread.
- [ ] Run the full state/core/Tauri/frontend/fake/wire gates and commit.
