# Live Signals Phase B GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render read receipts, the fully-read marker, typing, and presence from Rust-owned `AppState.live_signals`, and prove GUI controls dispatch typed live-signal commands.

**Architecture:** React remains a transport client: `App.tsx` passes `snapshot.state.live_signals` into `TimelineView`, and `TimelineView` renders indicators from that snapshot. Matrix operations cross the Tauri adapter as typed commands (`send_read_receipt`, `set_fully_read`, `set_typing`, `set_presence`); React does not maintain live-signal success/failure state or repair Rust state.

**Tech Stack:** React + TypeScript, Playwright headless harness, Tauri command adapter, Rust `CoreCommand`, existing i18n catalog and CSS.

---

## File Map

- `apps/desktop/e2e/basic-operations.spec.ts` - headless GUI-operation tests for command dispatch and Rust-snapshot rendering.
- `apps/desktop/src/backend/browserFakeApi.ts` and `apps/desktop/src/backend/client.ts` - desktop API command surface.
- `apps/desktop/src/test/appHarnessMain.tsx` and `apps/desktop/src/test/harnessMain.tsx` - mocked IPC command registration and TimelineView harness props.
- `apps/desktop/src/App.tsx` - pass live signals into timeline, dispatch typing command from composer changes, set user presence from settings/user menu scope if needed.
- `apps/desktop/src/components/TimelineView.tsx` - render receipts/read marker/typing/presence and dispatch mark-read commands when an event-backed row is visible on room open.
- `apps/desktop/src/i18n/messages.ts` - visible labels for receipts, read marker, typing, presence.
- `apps/desktop/src/styles.css` - stable, compact indicators.
- `apps/desktop/src-tauri/src/commands.rs` and `apps/desktop/src-tauri/src/lib.rs` - Tauri command handlers/builders/tests.

## Task 1: Browser-Headless Contract Test

- [x] Write a failing Playwright test in `apps/desktop/e2e/basic-operations.spec.ts` that:
  - seeds `snapshot.state.live_signals` with one receipt, a fully-read event id, one typing user, and presence for the seed sender;
  - pushes `StateChanged`;
  - asserts the timeline renders a read marker, receipt label/avatar, typing indicator, and sender presence;
  - clears invocations, types in the main composer, and expects `set_typing` with `{ roomId, isTyping: true }`;
  - expects room-open mark-read dispatch to call `send_read_receipt` and `set_fully_read` for the seed event.
- [x] Run `cd apps/desktop && npx playwright test e2e/basic-operations.spec.ts --grep "live signals"` and verify it fails because rendering/commands are missing.

## Task 2: API And Tauri Command Surface

- [x] Add `sendReadReceipt(roomId, eventId)`, `setFullyRead(roomId, eventId)`, `setTyping(roomId, isTyping)`, and `setPresence(presence)` to `DesktopApi`, `TauriDesktopApi`, browser fake, and IPC mocks.
- [x] Add Tauri command handlers and builder tests mapping to `TimelineCommand::SendReadReceipt`, `TimelineCommand::SetFullyRead`, `TimelineCommand::SetTyping`, and `AccountCommand::SetPresence`.
- [x] Register the commands in `tauri::generate_handler!`.
- [x] Run the focused Rust command tests and verify the new tests pass.

## Task 3: React Rendering And Dispatch

- [x] Extend `TimelineView` props with `liveSignals`.
- [x] Render read-marker divider before the fully-read event row, receipt avatars/label on event rows with receipts, sender presence, and a typing indicator below the item list.
- [x] In `App.tsx`, pass `snapshot.state.live_signals` to room/focused/thread timeline surfaces where room ids exist.
- [x] On composer draft changes, dispatch `setTyping(activeRoomId, true)` for non-empty drafts and `setTyping(activeRoomId, false)` when the draft is cleared or sent. Do not add React success/failure state.
- [x] When a room timeline first renders an event-backed row, dispatch both read receipt and fully-read marker commands for that event id. Suppress duplicate dispatch for the same timeline key/event in a local ref only.

## Task 4: Verification And Commit

- [x] Run the focused Playwright live-signal test and confirm it passes.
- [x] Run `npm --prefix apps/desktop run typecheck`.
- [x] Run `npm --prefix apps/desktop run test`.
- [x] Run `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- [x] Run `npm --prefix apps/desktop run qa:secret-scan` and `git diff --check`.
- [ ] Commit with message `Add live signals GUI wiring`.
