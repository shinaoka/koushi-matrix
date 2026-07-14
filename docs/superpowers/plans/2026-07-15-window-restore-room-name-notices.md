# Window Restore And Room-Name Notices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep restored desktop windows usable and render typed `m.room.name` events as localized timeline notices.

**Architecture:** Tauri gathers monitor work areas but delegates all restore decisions to a deterministic pure geometry helper. Core converts the SDK's typed room-name change into a closed localization projection; React only interpolates the Rust-projected values.

**Tech Stack:** Rust, Tauri 2, matrix-rust-sdk UI timeline types, TypeScript, React, Vitest.

---

### Task 1: Prove and implement safe physical window geometry

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs`

- [ ] Add failing unit tests for valid in-bounds geometry, oversized Retina-like geometry, wholly off-screen geometry, disconnected secondary-monitor geometry, negative monitor origins, and work areas smaller than the configured minimum.
- [ ] Run `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib restored_window_geometry` and confirm the new resolver is missing.
- [ ] Add a private `WindowWorkArea` value type and `restored_window_geometry` pure function that selects the maximum-intersection work area, falls back to primary/first, clamps size and position, and returns `None` for unusably small work areas.
- [ ] Update `apply_persisted_window_state` to collect `available_monitors()` and `primary_monitor()`, apply the resolved physical size and position, and maximize only afterward.
- [ ] Re-run the focused window test and `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib window_state`.
- [ ] Commit the window restore fix.

### Task 2: Prove and implement the Rust-owned room-name projection

**Files:**
- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/timeline.rs`

- [ ] Add failing Core unit tests for original content without previous name, changed content, empty content, identical content, redacted content, and the existing unknown-event fallback.
- [ ] Run `cargo test -p koushi-core --lib room_name_notice` and confirm typed room-name handling is absent.
- [ ] Add a serializable closed `TimelineNoticeI18n` projection carrying a key plus optional old/new plain-text values, with default-compatible timeline serialization.
- [ ] Pattern-match typed `AnyOtherStateEventContentChange::RoomName` before `is_redacted()` and the generic fallback, and project English fallback bodies as non-user notice content.
- [ ] Propagate the structured notice through canonical, focused, and loaded thread-root `TimelineItem` construction paths without changing identity or action affordances.
- [ ] Re-run `cargo test -p koushi-core --lib room_name_notice` and the existing `state_event_notice` tests.
- [ ] Commit the Core projection.

### Task 3: Localize and render structured notices

**Files:**
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/i18n/messages.test.ts`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json` if the checked-in wire artifact changes.

- [ ] Add failing Vitest assertions for English/Japanese set, changed, removed, and generic messages, including CJK/emoji/RTL/HTML-like values.
- [ ] Add a failing TimelineView test whose Rust-shaped item contains a room-name notice and assert the localized text is present while `Unsupported event: m.room.name` is absent.
- [ ] Run the two focused Vitest files and confirm the new keys/projection are unsupported.
- [ ] Add the TypeScript notice DTO, catalog keys and translations, and exhaustive presentation-only rendering branch.
- [ ] Update checked-in wire fixtures only where required by the serialized Rust contract.
- [ ] Run `npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts src/components/TimelineView.test.tsx` and `npm --prefix apps/desktop run typecheck`.
- [ ] Commit the frontend localization wiring.

### Task 4: Final verification and PR

**Files:**
- Review all files changed since `origin/main`.

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib window_state`.
- [ ] Run `cargo test -p koushi-core --lib room_name_notice` and `cargo test -p koushi-core --lib state_event_notice`.
- [ ] Run the focused Vitest command, desktop typecheck, and `node scripts/check-tauri-adapter-boundary.mjs`.
- [ ] Generate `git diff origin/main...HEAD` and run the repository's private-data-free `codex review -` recipe against repository rules and this plan.
- [ ] Resolve verified findings, repeat affected gates, push `codex/issues-246-256`, and open one PR with `Closes #246` and `Closes #256`.
