# Phase 10 UI Headless Product Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the post-runtime desktop product surface while keeping GUI launches out of ordinary agent work.

**Architecture:** React remains a pure renderer over `AppStateSnapshot` and fake or real `CoreEvent` streams. Headless browser tests exercise DOM layout, scrolling, command shapes, and event reactions through mocked Tauri IPC; the real Tauri app is reserved for the final native-integration smoke. Core/runtime behavior continues to be proven through `CoreCommand`/`CoreEvent`, local homeserver QA, and real-homeserver QA before any GUI confidence claim.

**Tech Stack:** Rust 2024, Matrix Rust SDK, Tauri v2, React, TypeScript, Vitest, Playwright headless Chromium, mocked Tauri IPC, optional `@wdio/tauri-service` browser-mode spike.

---

## Preconditions

- Phase 9 cleanup is complete: the SDK adapter is `matrix-desktop-sdk`, runtime IPC drift is checked by `coreEvents.generated.json`, room lifecycle cleanup commands exist, and real-homeserver QA reports `leave_room=ok forget_room=ok`.
- `docs/architecture/overview.md` and `docs/policies/engineering-rules.md` are the canon. If implementation discovers a contradiction, stop and amend those documents before code continues.
- Native GUI work is not the default path. Unattended agents do not launch the real Tauri app on macOS.

## GUI Launch Policy

| Work type | Native GUI allowed? | Required harness |
| --- | --- | --- |
| React rendering, responsive layout, right panel, settings, search panel, shortcut UI | No | Vitest and Playwright headless browser against Vite harness |
| DOM scroll behavior, focus behavior that a browser can model, fake `CoreEvent` streams | No | Playwright headless browser + `TauriIpcMock` |
| Tauri command invocation shape and frontend event subscription shape | No | Mocked `@tauri-apps/api` transport and IPC contract tests |
| Rust backend behavior, Matrix behavior, sync, recovery, search, room operations | No | core tests, local homeserver QA, real-homeserver QA |
| Real IPC, native window lifecycle, menus, WebKitGTK integration (Linux) | Yes, unattended under a virtual display | Linux Xvfb + `tauri-driver` + WebdriverIO (Phase 13 Linux lane; `tauri-driver` does not support macOS) |
| macOS-specific: WKWebView, OS menu accelerators, keychain/system prompts | Yes, only attended (Phase 13) | `qa:mac-gui` coordinated with the user |

`@wdio/tauri-service` browser mode is not canon yet. It may replace or augment the Playwright browser harness only after a spike proves the installed package can run Vite/browser mode without a Tauri binary, native driver, or real window. Until then, `npm --prefix apps/desktop run test:ui-headless` is the canonical GUI-free DOM gate.

## Phase 10: Headless UI Contract And Harness Hardening

Purpose: make the browser harness broad enough that Phase 11 product UI work can be verified without launching the app.

**Files:**

- Modify: `apps/desktop/src/test/tauriIpcMock.ts`
- Modify: `apps/desktop/src/test/harnessMain.tsx`
- Create: `apps/desktop/e2e/app-shell-headless.spec.ts`
- Modify: `apps/desktop/e2e/timeline-scrollback.spec.ts`
- Modify: `apps/desktop/playwright.config.ts`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src-tauri/src/lib.rs`

- [ ] Extend `TauriIpcMock` with reusable helpers for pushing snapshots, core events, and command responses. Keep recorded invocation sanitization strict for password, recovery, token, and key-like fields.
- [ ] Add a Playwright guard that fails if the harness page attempts to use a native Tauri app path or non-mock IPC path.
- [ ] Expand the headless harness beyond `TimelineView` so it can mount the real app shell with synthetic snapshots.
- [ ] Add headless tests for command invocation shapes used by room selection, right-panel open/close, search entry, settings entry, and logout entry. Assert command names and non-secret argument shapes only.
- [ ] Keep `test:ipc-contract` green after every Rust `CoreEvent` or snapshot wire-format change; regenerate `coreEvents.generated.json` intentionally, never by hand-editing around a failing test.
- [ ] Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run test:ipc-contract
git diff --check
```

Exit gate: Phase 10 exits only when the real app shell can be tested in a headless browser through mock IPC and no native Tauri process is needed.

## Phase 11: Product UI Layout And Interaction Surface

Purpose: build the visible desktop surface using the existing runtime contract, not new Matrix behavior.

**Files:**

- Modify: `apps/desktop/src/App.tsx`
- Create: `apps/desktop/src/components/DesktopShell.tsx`
- Create: `apps/desktop/src/components/RightPanel.tsx`
- Create: `apps/desktop/src/components/SettingsPanel.tsx`
- Create: `apps/desktop/src/components/SearchPanel.tsx`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Create: `apps/desktop/src/components/DesktopShell.test.tsx`
- Create: `apps/desktop/e2e/product-surface-headless.spec.ts`
- Create: `docs/qa/phase-11-headless-ui-audit.md`

- [ ] Implement the Element-like three-pane layout as the primary first screen after login: left navigation/room list, center timeline/composer, contextual right panel.
- [ ] Keep room, Space, thread, search, and settings panels render-only unless the corresponding core command already exists.
- [ ] Add component tests for right-panel mode switching, settings entry points, shortcut registry rendering, room selection state, and narrow-width behavior.
- [ ] Add Playwright headless tests for DOM behavior that Vitest cannot prove: scroll anchoring, focus return after panel close, narrow-width drawer behavior, and command suppression when a panel action is disabled.
- [ ] Keep all UI fixture data synthetic. Do not copy real room names, Matrix IDs, message bodies, attachment names, or screenshots into fixtures.
- [ ] Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run qa:secret-scan
git diff --check
```

Exit gate: Phase 11 exits when the main desktop product surface is useful in the headless harness and all Matrix-facing behavior still comes from existing core commands/events.

## Phase 12: Runtime Transport Integration Hardening

Purpose: prove that the frontend contract and the Tauri adapter stay aligned without relying on manual GUI observation.

**Files:**

- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `crates/matrix-desktop-core/src/**/*.rs`
- Modify: `crates/matrix-desktop-state/src/**/*.rs`

- [ ] Add or update Rust tests for every Tauri command that forwards to `CoreRuntime`; assert the command routes to the intended `CoreCommand` and returns redacted/coarse failures.
- [ ] Add or update frontend tests that consume the same serialized event shapes as `coreEvents.generated.json`.
- [ ] Keep fixture/demo backend behavior out of production Tauri command paths.
- [ ] Run local homeserver QA when Matrix behavior changes:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both
```

- [ ] Run real homeserver QA before any GUI-level confidence claim or release-preflight claim:

```bash
npm --prefix apps/desktop run qa:real-homeserver
```

The real homeserver command reads approved credentials from `.local-secrets/real-account-qa/credentials.json`; it must not print secrets, raw account data, or real message content. Expected release signal includes `recovery=completed`, `search=ok`, `store_restore=ok`, `restore_body=ok`, `leave_room=ok`, `forget_room=ok`, and logout cleanup.

Exit gate: Phase 12 exits when the Tauri adapter, TypeScript contract, local homeserver QA, and real homeserver QA agree on the same runtime behavior without opening the GUI.

## Phase 13: Native GUI Smoke And OS Integration

Purpose: verify only the behavior that cannot be proven headless.

**Files:**

- Modify: `scripts/desktop-mac-gui-smoke.mjs`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `apps/desktop/src-tauri/src/menu.rs`
- Modify: `docs/qa/*.md`
- Modify: `AGENTS.md`

- [ ] Linux virtual-display lane (priority — maximizes agent-driven GUI testing):
  spike a Linux environment (container or CI runner) with Xvfb +
  `tauri-driver` + WebdriverIO driving the REAL Tauri app invisibly;
  port the native-integration checks (window lifecycle, real IPC bridge,
  menu wiring) to it so they run unattended. If the lane proves out,
  moving primary GUI development/testing to Linux is an accepted option
  (canon, QA Model layer 5) — record the outcome as a canon amendment.
- [ ] Before launching native GUI smoke, confirm Phase 10, Phase 11, Phase 12, secret scan, and real-homeserver QA are green for the relevant change set.
- [ ] Keep macOS native GUI smoke attended. Do not launch a real Tauri window from unattended agent verification.
- [ ] Verify native-only behavior: window creation, real IPC bridge, OS menu accelerators, close/quit semantics, first-run saved-session suppression, and keychain prompt suppression.
- [ ] Keep credential entry through FIFO or approved debug/test credential-store paths. Never type credentials by coordinates.
- [ ] Do not store post-login screenshots for real accounts.
- [ ] Run only after coordination with the user:

```bash
npm --prefix apps/desktop run qa:mac-gui
```

Exit gate: Phase 13 exits with a private-data-free QA audit that lists which native-only behaviors were proven and which remain manual.

## Phase 14: Distribution, Trust UX, And Release Hardening

Purpose: finish the work that blocks signed desktop distribution and E2EE trust completeness.

**Files:**

- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `apps/desktop/src-tauri/src/*.rs`
- Modify: `crates/matrix-desktop-core/src/**/*.rs`
- Modify: `crates/matrix-desktop-sdk/src/**/*.rs`
- Modify: `docs/upstream/matrix-rust-sdk-feedback.md`
- Modify: `docs/qa/*.md`

- [ ] Run live OS credential-store ignored tests on Windows before Windows distribution claims.
- [ ] Prepare signed macOS and Windows release builds, including notarization/signing credentials and installer verification.
- [ ] PRECONDITION: device verification / cross-signing is a major design
  task on a canon-declared open area. Before any implementation, the
  strongest available model (per Model Assignment) authors a dated spec
  amending `docs/architecture/overview.md`; implementation starts only
  after that spec is approved.
- [ ] Implement device verification and cross-signing under `AccountActor` with explicit `CoreCommand`/`CoreEvent` surfaces per the approved spec, before claiming E2EE trust UX completeness.
- [ ] Review vendored SDK patches at phase exit. Remove any patch that became unnecessary through public SDK APIs; keep indispensable patches recorded in `docs/upstream/matrix-rust-sdk-feedback.md`.
- [ ] Run release preflight:

```bash
npm --prefix apps/desktop run release:preflight
npm --prefix apps/desktop run qa:secret-scan
git diff --check
```

Exit gate: Phase 14 exits only with signed-build evidence, platform credential-store evidence, and no unreviewed vendored SDK deltas.

## Subagent Execution Pattern

For Phase 10 and later, use a reviewer/implementer split. Model Assignment
(2026-06-12 implementation plan) still binds: canon amendments and redesign
decisions escalate to the strongest available model of the agent family
(never a mini/lightweight tier); implementers stop on gaps instead of
improvising. Phase exits require the reviewer to re-execute the gates.

1. A mini/subagent implements one task or one narrow phase slice.
2. The main agent reviews the diff against `docs/architecture/overview.md`, `docs/policies/engineering-rules.md`, this plan, and the relevant tests.
3. The main agent runs or re-runs the verification commands before accepting the result.
4. The main agent updates canon documents first if a design contradiction is discovered.
5. Commit only after the diff, docs, and gates line up.

## Phase 10+ Gate Summary

```bash
cargo test -p matrix-desktop-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run test:ipc-contract
npm --prefix apps/desktop run qa:secret-scan
git diff --check
```

Add `npm --prefix apps/desktop run qa:headless-local -- --server=both` when Matrix behavior changes. Add `npm --prefix apps/desktop run qa:real-homeserver` before GUI-level confidence, release-preflight claims, or any change that affects login, recovery, sync, encrypted restore, search, room cleanup, or logout.
