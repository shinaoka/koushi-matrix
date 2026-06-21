# State Transport Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce the #111 frontend selector store fed by existing full snapshots, preserving unchanged slice references so unrelated background updates do not re-render hot timeline/composer consumers.

**Architecture:** Phase 1 does not change Rust transport. `App.tsx` still receives `DesktopSnapshot` responses and `StateChanged` refreshes, but every snapshot is applied through a Zustand projection cache that value-compares top-level snapshot/domain/ui/sidebar/timeline/thread slices and keeps prior references for unchanged values. Components can then subscribe to selected slices or memoized derived selectors without treating React as the product-state owner.

**Tech Stack:** React 19, TypeScript, Zustand 5, Vitest, Tauri IPC mock, Rust-owned `DesktopSnapshot` DTOs.

**Spec:** `docs/superpowers/specs/2026-06-21-state-transport-architecture-design.md`

---

## Global Constraints

- Rust remains the source of truth. The store is a WebView projection cache only; React dispatches typed commands and never repairs Matrix/product state locally.
- Phase 1 keeps the existing full-snapshot transport. Delta DTOs, generation gaps, and Channel routing are Phase 2/3 work.
- Canon updates precede code. Do not claim full-snapshot normal-path removal until Phase 2 changes `CoreEvent`/Tauri DTOs/runtime.
- Main agent owns hot files: `apps/desktop/src/App.tsx`, `apps/desktop/src/components/panes.tsx`, `apps/desktop/src/components/TimelineView.tsx`, canon docs, package manifests, commits.
- Tests must use synthetic snapshots only; do not print real account IDs, room IDs, message bodies, raw SDK errors, or secrets.

### Task 1: Canon and Dependency Boundary

**Files:**
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `AGENTS.md`
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop/package-lock.json`

- [x] **Step 1: Amend canon.** Add the state-transport rule: Rust owns product state; the WebView projection store is selector-subscribed cache; Phase 1 full snapshots are applied slice-by-slice with stable references; Phase 2 changes normal transport to deltas plus full reset fallback; Channels are high-frequency only and measurement-gated.
- [x] **Step 2: Add Zustand.** Run `npm --prefix apps/desktop install zustand@^5.0.14`.
- [x] **Step 3: Verify docs/dependency shape.** Run `git diff -- docs/architecture/overview.md docs/architecture/state-machine.md AGENTS.md apps/desktop/package.json apps/desktop/package-lock.json` and confirm there are no product-state ownership contradictions.

### Task 2: Projection Store RED/GREEN

**Files:**
- Create: `apps/desktop/src/domain/appStore.ts`
- Create: `apps/desktop/src/domain/appStore.test.ts`

- [x] **Step 1: Write failing tests.** Cover `applySnapshotToState(previous, next)` with synthetic `DesktopSnapshot`s:
  - identical-by-value incoming snapshot keeps `state`, `state.domain`, `state.ui`, `sidebar`, `timeline`, and `thread` references stable;
  - changing only `state.domain.search_crawler` replaces `state.domain` and `state`, but preserves `state.ui`, `sidebar`, `timeline`, and `thread`;
  - changing only `state.ui.timeline.composer.draft` replaces `state.ui` and `state`, but preserves `state.domain`, `sidebar`, `timeline`, and `thread`;
  - `null` resets the store snapshot to `null`.
- [x] **Step 2: Run RED.** Run `npm --prefix apps/desktop run test -- --run src/domain/appStore.test.ts`; expected failure: module/functions missing.
- [x] **Step 3: Implement store.** Export a Zustand store, `useAppStore`, `getAppStoreSnapshot`, `setAppStoreSnapshot`, `applySnapshotToState`, and selectors for the full snapshot and common slices. Use value equality on slices (JSON-compatible DTOs) and rebuild only the containing `state/domain/ui` objects required by changed child slices.
- [x] **Step 4: Run GREEN.** Re-run the same Vitest command; expected pass.

### Task 3: App Snapshot Boundary Integration

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Test: `apps/desktop/src/domain/appStore.test.ts`

- [x] **Step 1: Extend test.** Add a test for `setAppStoreSnapshot` notifying a domain selector subscriber only when the selected slice changes.
- [x] **Step 2: Run RED if subscriber behavior is absent.** Run the app-store test command.
- [x] **Step 3: Integrate boundary.** In `App.tsx`, subscribe to the app-store snapshot for rendering, and make the existing guarded `setSnapshot` call `setAppStoreSnapshot(next)` instead of replacing a private `useState` snapshot directly. Keep schema mismatch handling in `App.tsx`.
- [x] **Step 4: Run GREEN.** Run `npm --prefix apps/desktop run test -- --run src/domain/appStore.test.ts` and `npm --prefix apps/desktop run typecheck`.

### Task 4: Hot Derived Selectors and Render-Isolation Guard

**Files:**
- Modify: `apps/desktop/src/domain/appStore.ts`
- Test: `apps/desktop/src/domain/appStore.test.ts`
- Modify as needed: `apps/desktop/src/components/panes.tsx`

- [x] **Step 1: Write failing selector tests.** Add memoized selectors for forward destinations and mention candidates. Assert an unrelated `search_crawler` update keeps selector output references stable, while changing `rooms` or `profile.users` changes the appropriate output.
- [x] **Step 2: Run RED.** Run `npm --prefix apps/desktop run test -- --run src/domain/appStore.test.ts`; expected failure: selectors missing or unstable.
- [x] **Step 3: Implement selectors.** Move derived array memoization into `appStore.ts` using input-reference caches. Keep Rust-projected labels/search terms unchanged; do not recompute alias precedence.
- [x] **Step 4: Wire hot consumers.** Use the memoized selector outputs in `TimelinePane` for `TimelineView.forwardDestinations` and `Composer.mentionCandidates`; keep broad prop-driven rendering elsewhere for this phase.
- [x] **Step 5: Run GREEN.** Run the app-store test and `npm --prefix apps/desktop run typecheck`.

### Task 5: Verification and Commit

- [x] **Step 1:** Run `npm --prefix apps/desktop run test -- --run src/domain/appStore.test.ts`.
- [x] **Step 2:** Run `npm --prefix apps/desktop run typecheck`.
- [x] **Step 3:** Run `npm --prefix apps/desktop run qa:secret-scan`.
- [x] **Step 4:** Run `git diff --check`.
- [x] **Step 5:** Dispatch focused subagent review against #111 Phase 1 requirements and this plan; fix any verified findings.
- [x] **Step 6:** Commit exactly the #111 Phase 1 changes as one commit.

## Self-Review

- **Spec coverage:** Phase 1 (`frontend store, no transport change`) is covered by Tasks 1-4. Phase 0 measurement, Phase 2 deltas, Phase 3 Channels, and Phase 4 list diffs are explicitly out of this commit.
- **Placeholder scan:** No placeholders remain; each task has exact files and commands.
- **Type consistency:** `DesktopSnapshot` remains the input DTO; `appStore.ts` owns projection-cache helpers only and does not introduce product-state mutation APIs.
