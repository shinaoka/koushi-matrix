# Category Unread Badges Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render simultaneous Rust-owned unread/highlight badges and distinct total counts on DMs and Rooms tabs.

**Architecture:** `Sidebar` passes existing `SidebarModel` aggregates to a presentation-only `RoomListControls`. Browser tests mutate Rust-shaped snapshots to prove inactive-category and live-delta behavior.

**Tech Stack:** React/TypeScript, CSS, Playwright, Rust sidebar tests.

---

### Task 1: Add browser-headless RED coverage

**Files:**
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] Add one focused scenario that seeds totals and Rust sidebar aggregates,
  asserts accessible names such as `DMs, 3 unread, 58 total`, and proves both
  categories remain visible while Rooms then DMs is selected.
- [ ] In the same scenario apply snapshots for DMs-only, Rooms-only, zero,
  highlights, `100 -> 99+`, mark-read clearing, and active-space scoped counts.
  Assert category persistence after a harness remount.
- [ ] Run:

  ```bash
  npm --prefix apps/desktop exec -- playwright test --config apps/desktop/playwright.config.ts apps/desktop/e2e/basic-operations.spec.ts -g "category unread badges" --workers=1
  ```

  Expected: FAIL because the tabs expose only total counts.

### Task 2: Render projected counts without aggregation

**Files:**
- Modify: `apps/desktop/src/components/Shell.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`

- [ ] Add a presentation helper:

  ```ts
  function compactAttentionCount(count: number): string {
    return count > 99 ? "99+" : String(count);
  }
  ```

- [ ] Pass total/unread/highlight triples from `snapshot.sidebar`; do not scan
  room arrays for attention. Render `room-list-chip-total` and, only when
  nonzero, `room-list-chip-unread` with `is-highlight` when highlight > 0.
- [ ] Add localized accessible-name messages with unread, total, and optional
  highlight placeholders for English and Japanese.
- [ ] Add responsive/forced-colors styles that preserve selected/focus/hover
  contrast and stable narrow widths.
- [ ] Re-run the Playwright RED command and:

  ```bash
  npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
  npm --prefix apps/desktop run typecheck
  cargo test -p koushi-state --test navigation_state sidebar
  ```

  Expected: PASS.
- [ ] Commit with `feat(#265): show category unread badges`.

