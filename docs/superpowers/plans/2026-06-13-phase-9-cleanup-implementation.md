# Phase 9 Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the post-headless cleanup items deferred from Phase 9 before starting Phase 10 GUI design work.

**Architecture:** Keep behavior owned by `matrix-desktop-core` and low-level SDK calls in the renamed `matrix-desktop-sdk` adapter. Preserve the frontend import surface while making IPC drift fail locally through a generated contract artifact.

**Tech Stack:** Rust 2024, Cargo workspace, Tauri v2, React/TypeScript, serde JSON contract fixtures.

Status: completed.

---

## Task 1: SDK Adapter Rename

**Files:**
- Move: `crates/matrix-desktop-auth/` -> `crates/matrix-desktop-sdk/`
- Modify: `Cargo.toml`
- Modify: `crates/matrix-desktop-core/Cargo.toml`
- Modify: `crates/matrix-desktop-backend/Cargo.toml`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: Rust imports from `matrix_desktop_auth` to `matrix_desktop_sdk`
- Modify: docs and scripts that refer to package `matrix-desktop-auth`

- [x] Run a search for `matrix-desktop-auth` and `matrix_desktop_auth`.
- [x] Move the crate directory with `git mv`.
- [x] Update package names, dependency names, imports, scripts, and docs to `matrix-desktop-sdk` / `matrix_desktop_sdk`.
- [x] Run `cargo check -p matrix-desktop-core` and `cargo test -p matrix-desktop-sdk`.

## Task 2: Room Lifecycle Commands

**Files:**
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/room.rs`
- Modify: `crates/matrix-desktop-core/src/bin/real-homeserver-qa.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs` if frontend commands are needed

- [x] Write failing core tests proving `LeaveRoom`/`ForgetRoom` without a session emit `SessionRequired`.
- [x] Add adapter functions that parse room IDs, call SDK leave/forget operations, and classify errors.
- [x] Add `RoomCommand::LeaveRoom` and `RoomCommand::ForgetRoom`.
- [x] Add `RoomEvent::RoomLeft` and `RoomEvent::RoomForgotten`.
- [x] Route the commands through `RoomActor`, emit classified failures, and refresh the room list after success.
- [x] Update real-homeserver QA to leave/forget the synthetic QA room and fail if cleanup is unavailable.

## Task 3: Runtime IPC Contract Check

**Files:**
- Create: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/package.json`

- [x] Add a Rust test that serializes representative `CoreEvent` payloads and compares them with the checked-in JSON artifact.
- [x] Add an npm script that runs the Rust contract test.
- [x] Keep TypeScript imports stable; `coreEvents.ts` remains the frontend import surface.
- [x] Document that the generated JSON is the source-of-truth drift guard until full TS generation replaces handwritten types.

## Task 4: Optional App Commands Decision

**Files:**
- Modify: `docs/superpowers/specs/2026-06-13-post-headless-core-followups.md`
- Modify: `docs/architecture/overview.md`
- Modify: `apps/desktop/src-tauri/src/commands.rs`

- [x] Record `discover_login_methods` as a compatibility shim because `LoginPassword` performs discovery inside core.
- [x] Record `open_thread`/`close_thread` as frontend navigation shims unless a later product design needs cross-window/native thread commands.
- [x] Remove these items from the active follow-up list.

## Verification

- [x] `cargo check -p matrix-desktop-core`
- [x] `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [x] `cargo test -p matrix-desktop-sdk`
- [x] `cargo test -p matrix-desktop-core --lib`
- [x] `cargo test -p matrix-desktop-core --features qa-bin,test-hooks --bin real-homeserver-qa -- --nocapture`
- [x] `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [x] `npm --prefix apps/desktop run typecheck`
- [x] `npm --prefix apps/desktop run test`
- [x] `npm --prefix apps/desktop run test:ipc-contract`
- [x] `npm --prefix apps/desktop run qa:secret-scan`
- [x] `npm --prefix apps/desktop run qa:real-homeserver`
- [x] `git diff --check`
- [x] targeted `rustfmt --check` on changed Rust files
