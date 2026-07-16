# Windows Overlay ACL Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Admit Windows taskbar overlay set/clear through the checked-in Tauri ACL and distinguish ACL denial from native backend failure.

**Architecture:** Keep the capability scoped to the existing `main` window. Prove admission with a Windows mock-runtime IPC call built from the real Tauri context, while TypeScript emits only coarse diagnostic tokens.

**Tech Stack:** Tauri 2 Rust tests, Vitest, GitHub Actions YAML.

---

### Task 1: Lock the ACL and diagnostic behavior with failing tests

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/domain/desktopAttention.test.ts`

- [ ] Add a Rust config contract test that parses `capabilities/default.json`,
  requires `windows == ["main"]`, requires
  `core:window:allow-set-overlay-icon`, and rejects wildcard windows.
- [ ] Under `#[cfg(target_os = "windows")]`, build
  `tauri::test::mock_builder()` with `tauri::generate_context!()`, create the
  `main` webview, and invoke `plugin:window|set_overlay_icon` with set and clear
  request bodies through `tauri::test::get_ipc_response`.
- [ ] Add Vitest cases expecting `attention_overlay_acl_denied` for a synthetic
  Tauri permission rejection and `attention_overlay_failed` for an ordinary
  backend rejection.
- [ ] Run:

  ```bash
  cargo test -p koushi-desktop main_window_overlay_permission_contract
  npm --prefix apps/desktop run test -- --run src/domain/desktopAttention.test.ts
  ```

  Expected: Rust fails because the permission is absent; Vitest fails because
  the ACL token and classifier do not exist.

### Task 2: Add the minimal permission and classifier

**Files:**
- Modify: `apps/desktop/src-tauri/capabilities/default.json`
- Modify: `apps/desktop/src/domain/desktopAttention.ts`

- [ ] Add only `core:window:allow-set-overlay-icon` to the existing main
  capability.
- [ ] Extend `DesktopAttentionDiagnosticToken` with
  `attention_overlay_acl_denied`. Pass the caught value to a private classifier
  that recognizes Tauri command-denial wording and emits only the coarse token;
  never retain or print the raw error.
- [ ] Run the focused Rust and Vitest commands from Task 1. Expected: PASS on
  Linux for the config/adapter tests; the Windows IPC test is cfg-skipped.

### Task 3: Add Windows CI proof

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] Add a `windows-overlay-acl` job on `windows-latest` with recursive
  checkout, Rust 1.96.0, Node 22/npm cache, `npm ci`, and:

  ```bash
  cargo test -p koushi-desktop windows_overlay_ipc_is_authorized -- --exact
  ```

- [ ] Validate YAML and run `git diff --check`.
- [ ] Commit with `fix(#266): authorize Windows unread overlay`.

