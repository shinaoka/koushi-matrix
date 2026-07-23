# Composer Draft Revision Fence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent stale main/thread composer draft persistence from restoring content after an accepted send while preserving newer input and rejected/unknown submissions.

**Architecture:** Rust owns a monotonically increasing revision for each `ComposerTarget`. Draft writes carry their captured revision; accepted plain/reply, scheduled, and prepared-upload submissions advance beyond the submitted draft, clearing it when current or preserving any already-newer input at the advanced revision. The encrypted draft store persists revision tombstones with backward-compatible migration, while one shared TypeScript target/revision coordinator creates revisions and rejects stale command responses.

**Tech Stack:** Rust reducers/core/Tauri, serde encrypted persistence, React/TypeScript, Vitest/Playwright, Cargo integration tests.

---

### Task 1: Reproduce stale draft restoration in reducers

**Files:**
- Modify: `crates/koushi-state/tests/timeline_thread_state.rs`
- Modify: `crates/koushi-state/tests/scheduled_send_state.rs`

- [ ] Add deterministic main plain/reply, immediate-next-input, thread isolation, thread-next-input, and scheduled-send sequences in which a pre-acceptance draft action arrives after the accepted clear.
- [ ] Run `cargo test -p koushi-state --test timeline_thread_state composer_draft_revision -- --nocapture` and the focused scheduled test; require assertion failures showing old content restored.

### Task 2: Add the Rust-owned revision and persistence contract

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/architecture/overview.md`
- Modify: `crates/koushi-state/src/state/timeline.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/{mod,timeline,thread}.rs`
- Modify: `crates/koushi-core/src/{command,runtime,store}.rs`
- Modify affected Rust tests.

- [ ] Add `draft_revision` to active composer projection and persisted target revision maps.
- [ ] Migrate legacy encrypted stores with absent revision maps to revision zero; retain bounded empty revision tombstones.
- [ ] Accept only draft writes newer than the target revision.
- [ ] Make accepted plain/reply/thread/scheduled clears set an empty revision greater than both the authoritative and submitted revisions.
- [ ] Run focused state/core persistence tests and require GREEN.

### Task 3: Carry causal revisions through Tauri and the shared frontend helper

**Files:**
- Create: `apps/desktop/src/domain/composerDraftRevision.ts`
- Create: `apps/desktop/src/domain/composerDraftRevision.test.ts`
- Modify: `apps/desktop/src/backend/{client,browserFakeApi}.ts`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src-tauri/src/{dto.rs,commands/mod.rs,commands/timeline.rs}`
- Modify frontend/IPC/browser fixtures and tests.

- [ ] Add deferred-future tests for both completion orders, immediate next input, target isolation, room/thread switching, rejection/unknown retention, and stale response suppression.
- [ ] Use one target-keyed coordinator for main and thread revision creation, acceptance fences, and response applicability.
- [ ] Pass revisions through draft, plain/reply/thread, scheduled, and prepared-upload commands.
- [ ] Remove the racing post-upload empty draft saves; after all prepared uploads are accepted, dispatch one Rust-owned causal clear.
- [ ] Update all hand-maintained DTO/TypeScript/browser/IPC mirrors and serialization tests.
- [ ] Run focused Vitest/Playwright/Tauri/typecheck checks and require GREEN.

### Task 4: Persistence, restart, IME, and final verification

**Files:**
- Modify focused persistence/runtime/browser tests as required.

- [ ] Prove encrypted round-trip, legacy migration, restart hydration, both completion orders, and accepted-empty persistence.
- [ ] Run IME inventory/controller tests and composer submission correlation regressions.
- [ ] Run state/core/Tauri/frontend focused suites, lint, typecheck, build, secret scan, SDK guard, `git diff --check`, and repository release gates applicable to the diff.
- [ ] Generate the complete diff and run the independent `codex review -` canon/security/contract review; resolve only verified findings.
- [ ] Commit, push, open one PR with `Closes #293`, monitor CI, fix change-caused failures, merge with repository policy, and verify #293 closed plus the merge commit on `origin/main`.
