# #161 Jump to date navigates the main timeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** `Jump to date` scrolls the main room timeline to the nearest event at arbitrary depth (never opens the right panel), reusing the existing event-focused timeline in the main pane, with a live-edge return.

**Architecture:** Add a Rust-owned guarded main-pane mode `Live ↔ Anchored{event_id}` (`NavigationState.main_timeline_anchor: Option<MainTimelineAnchor>`). Jump-to-date resolves the nearest event via `timestamp_to_event`; if it is in the loaded live window, set `room_scroll_anchors` and stay `Live`; otherwise enter `Anchored` (main pane subscribes `TimelineKind::Focused`). The live-edge control returns to `Live`.

**Tech Stack:** Rust (`koushi-state` reducer/state machine, `koushi-core` account/timeline), Tauri commands, React `TimelineView.tsx`/`App.tsx`, Playwright, Linux GUI lane.

## Global Constraints

- Reducer transitions are guarded state machines; keep the Mermaid diagram in `docs/architecture/state-machine.md` (and the spec) in sync. (verbatim: REPOSITORY_RULES / memory state-machine-design-rigor)
- Timeline navigation semantics are Rust-owned; React reports `observe_timeline_viewport` facts and renders — it must not compute placement/mode. (verbatim: AGENTS.md Timeline Navigation)
- DTO mirror tax (same change): `dto.rs`, `types.ts`, `browserFakeApi.ts`, `tauriIpcMock.ts`, `appHarnessMain.tsx`, DTO serialization-contract test; new command/event → `serialize_core_event`, `coreEvents.ts`, `coreEvents.generated.json`, wire-contract test.
- verify-first RED before fix; private-data-free (`timeline_nav=ok`).

---

### Task 1: State + guarded transition for `main_timeline_anchor` (RED→GREEN, Rust)

**Files:**
- Modify: `crates/koushi-state/src/state/navigation.rs` (add `main_timeline_anchor: Option<MainTimelineAnchor { event_id: String }>`; `#[serde(default, skip_serializing_if=...)]`)
- Modify: `crates/koushi-state/src/reducer/navigation.rs` (guarded transitions: SetAnchored, ReturnToLive, room-change reset)
- Modify: `crates/koushi-state/src/action.rs` (actions: `EnterAnchoredTimeline{room_id,event_id}`, `ReturnMainTimelineToLive{room_id}`)
- Test: `crates/koushi-state/tests/` navigation state-machine test

**Interfaces:**
- Produces: `NavigationState.main_timeline_anchor`; reducer guarantees anchor cleared on `SelectRoom`/room change and on `ReturnMainTimelineToLive`; only set for session-ready known room.

- [ ] Step 1: RED test — dispatching `EnterAnchoredTimeline` sets the anchor; `ReturnMainTimelineToLive` clears it; selecting a different room clears it; ignored when session not ready.
- [ ] Step 2: `cargo test -p koushi-state --test <nav_test>` → FAIL.
- [ ] Step 3: Implement state field + guarded reducer arms + keep Mermaid in spec/state-machine.md in sync.
- [ ] Step 4: `cargo test -p koushi-state` → PASS.
- [ ] Step 5: Commit — `feat(state): main-pane timeline anchor state machine (#161)`

---

### Task 2: Core resolves nearest event and drives the mode (RED→GREEN, Rust)

**Files:**
- Modify: `crates/koushi-core/src/account.rs` (`handle_open_timeline_at_timestamp`): after `timestamp_to_event`, decide loaded-vs-out-of-window against the live timeline items; loaded → dispatch scroll-anchor update + stay Live; out-of-window → dispatch `EnterAnchoredTimeline` + subscribe `TimelineKind::Focused`. Remove `AppAction::OpenFocusedContext` from this path.
- Modify: `crates/koushi-core/src/runtime.rs` (rewrite the wrong-contract test)
- Modify: `crates/koushi-core/src/command.rs`/`event.rs` if a `ReturnMainTimelineToLive` command/event is added.

**Interfaces:**
- Consumes: Task 1 actions.
- Produces: on timestamp jump, either a `room_scroll_anchors` update (loaded) or `main_timeline_anchor=Some` + Focused subscription (out-of-window); never focused-context.

- [ ] Step 1: RED — rewrite `timestamp_jump_uses_local_activity_projection_before_homeserver_fallback` to assert the main-timeline path (no `OpenFocusedContext`); add coverage that an out-of-window resolved event enters `Anchored`.
- [ ] Step 2: run the core test → FAIL (still dispatches OpenFocusedContext).
- [ ] Step 3: Implement the loaded-vs-out-of-window decision + subscription routing.
- [ ] Step 4: `cargo test -p koushi-core` → PASS.
- [ ] Step 5: Commit — `fix(core): jump-to-date drives main-pane mode, not focused context (#161)`

---

### Task 3: Live-edge "return to live" command + Tauri wiring (RED→GREEN)

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands/navigation.rs` (`open_timeline_at_timestamp`: stop `wait_for_focused_context`; wait for the anchor/mode snapshot. Add `return_main_timeline_to_live` command.)
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs` (builders), `lib.rs` (register command)
- Test: src-tauri command tests + DTO serialization-contract test extension.

- [ ] Step 1: RED — DTO serialization-contract test includes `main_timeline_anchor`; command test asserts `open_timeline_at_timestamp` no longer requires focused-context.
- [ ] Step 2: run → FAIL.
- [ ] Step 3: Implement command changes + extend `dto.rs` `FrontendAppState`.
- [ ] Step 4: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` → PASS.
- [ ] Step 5: Commit — `feat(tauri): main-timeline anchor snapshot + return-to-live command (#161)`

---

### Task 4: DTO/TS mirrors + coreEvents wiring (GREEN via contract tests)

**Files:** `apps/desktop/src/domain/types.ts`, `browserFakeApi.ts`, `tauriIpcMock.ts`, `test/appHarnessMain.tsx`, `domain/coreEvents.ts`, `domain/coreEvents.generated.json`.

- [ ] Step 1: Mirror `main_timeline_anchor` + any new command/event across all snapshot fixtures/mocks.
- [ ] Step 2: `npm --prefix apps/desktop run typecheck` and `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact` → PASS.
- [ ] Step 3: Commit — `chore(dto): mirror main_timeline_anchor across snapshots/mocks (#161)`

---

### Task 5: TimelineView renders anchored mode + live-edge return (RED→GREEN, React)

**Files:**
- Modify: `apps/desktop/src/App.tsx` (`openAtTimestamp`: remove `setRightPanelMode("focusedContext")`)
- Modify: `apps/desktop/src/components/TimelineView.tsx` (main pane subscribes/renders the `Focused` timeline when `main_timeline_anchor` is set; down/live-edge button dispatches `return_main_timeline_to_live` when anchored; center the target event)
- Test: `apps/desktop/e2e/timeline-scrollback.spec.ts` (rewrite date-jump spec) + `App.test.tsx` (rewrite)

- [ ] Step 1: RED — rewrite `App.test.tsx` date-jump test (assert NO focused-context) and add a Playwright spec: jump to an out-of-window date navigates the main timeline and does not open the right panel; live-edge returns to live.
- [ ] Step 2: run specs → FAIL.
- [ ] Step 3: Implement main-pane anchored rendering + live-edge return. Follow the #158 scroll-anchoring rules (user-driven scroll only leaves live edge; dispatch explicit scroll events in tests).
- [ ] Step 4: run specs (isolated / `--workers=1` per known flake notes) → PASS; `npm --prefix apps/desktop run typecheck`.
- [ ] Step 5: Commit — `feat(gui): main-pane anchored timeline + live-edge return for jump-to-date (#161)`

---

### Task 6: Linux GUI lane contract update (proof)

**Files:** `scripts/desktop-linux-gui-qa.mjs` `--scenario=local-timeline-navigation` (date-jump asserts main-pane navigation, rejects focused-context).

- [ ] Step 1: Update the lane's date-jump assertions to the new contract; keep `focused=` title tokens meaningful.
- [ ] Step 2: Run the lane per AGENTS.md (build once, then `--skip-build`); expect `gui_local_timeline_date_jump=ok`.
- [ ] Step 3: Commit — `test(qa): local-timeline-navigation asserts main-pane date jump (#161)`

## Self-Review

- Spec coverage: state machine (T1), core decision + arbitrary depth (T2), commands/Tauri (T3), DTO mirrors (T4), GUI render + live-edge (T5), lane proof (T6). ✓
- Wrong-contract tests rewritten in T2 (runtime.rs) and T5 (App.test.tsx, e2e). ✓
- Type consistency: `main_timeline_anchor` / `EnterAnchoredTimeline` / `ReturnMainTimelineToLive` used identically across tasks.
- Non-goal preserved: focused-context remains for other callers (verify in T2).
