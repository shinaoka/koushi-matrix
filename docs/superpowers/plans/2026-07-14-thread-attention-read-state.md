# Thread Attention Read-State Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace vector-diff-driven thread attention with a Rust-owned read-state tracker that is stable across hydration, backfill, replay, reset, and reconnect.

**Architecture:** `TimelineActor` owns a per-thread semantic tracker seeded from the SDK's latest own threaded receipt. The tracker reconciles canonical timeline items by stable event ID and explicit lifecycle mode, while the existing reducer and React surfaces continue to consume the unchanged `ThreadAttentionState` DTO.

**Tech Stack:** Rust, Tokio, matrix-rust-sdk UI timeline, React/TypeScript, Vitest/Playwright, Cargo.

---

### Task 1: Lock the regression with RED Rust tests

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`

- [ ] Replace the existing diff-shape attention unit test with tracker-facing tests for root/history hydration, backfill/reset, own local and remote echoes, one genuine live remote reply, reconnect deduplication, and two-thread isolation.
- [ ] Add a test that keeps `ThreadSummaryDto.reply_count` separate from new attention.
- [ ] Run `cargo test -p koushi-core --lib thread_attention`; confirm the new tests fail because the semantic tracker API/behavior is absent.

### Task 2: Implement the Rust semantic tracker

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`

- [ ] Introduce explicit hydration/live/backfill/replay observation modes and stable-event-ID sets.
- [ ] Seed the tracker from initial items and `Timeline::latest_user_read_receipt_timeline_event_id`.
- [ ] Apply diffs to `navigation_items` before reconciling attention; mark pagination/restore as backfill and reset/recovery/replayed initial items as replay.
- [ ] Subscribe to own read-receipt changes and reconcile them inside the actor.
- [ ] On successful threaded read receipt, acknowledge the event and reliably project zero or remaining counters.
- [ ] Run `cargo test -p koushi-core --lib thread_attention`; confirm GREEN.

### Task 3: Prove the unchanged GUI projection and acknowledgement clear

**Files:**
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] Extend the thread-attention browser test so the root affordance and header show the same Rust-shaped count.
- [ ] Drive the defined read acknowledgement through the mocked command, keep both counts until a new Rust-shaped snapshot arrives, then assert both clear together.
- [ ] Run `npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "thread attention" --workers=1`; confirm RED before any harness/product adjustment and GREEN after the minimal test/harness change.

### Task 4: Update normative canon

**Files:**
- Modify: `REPOSITORY_RULES.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `AGENTS.md`

- [ ] Replace the obsolete remote-`PushBack` rule with the receipt/lifecycle tracker contract.
- [ ] Document stable-ID deduplication, actual-thread-reply filtering, and acknowledgement behavior.
- [ ] Keep total reply count explicitly separate from new/unread attention.

### Task 5: Run focused and repository verification

**Files:**
- Test only

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p koushi-core --lib thread_attention`.
- [ ] Run `cargo test -p koushi-state --test timeline_thread_state`.
- [ ] Run `npm --prefix apps/desktop run typecheck`.
- [ ] Run the focused Playwright command from Task 3.
- [ ] Run the repository secret scan and the CI-relevant gates selected from `.github/workflows/ci.yml` for the changed paths.

### Task 6: Independent review, PR, CI, and non-squash merge

**Files:**
- Review and GitHub operations only

- [ ] Commit the verified change and generate the complete base-to-HEAD diff, including new files.
- [ ] Run the repository-mandated private-data-free `codex review -` against repository rules, architecture, state machine, engineering policy, AGENTS, and this plan.
- [ ] Resolve every verified in-scope finding and rerun affected gates.
- [ ] Push the dedicated branch, open a PR linked to #258, and monitor required checks.
- [ ] Fix in-scope CI failures; when green, merge with a non-squash merge.
- [ ] Verify issue closure, post the final issue summary, remove the dedicated worktree/branch, and report the centralized `AISS_RESULT` record.
