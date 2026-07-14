# Composer Media Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make picker, paste, and drag/drop reliable for ordinary files, prepare inspectable image variants before send, and expose the same safe formatted/media behavior in thread replies.

**Architecture:** A typed Rust `ComposerTarget` scopes staged uploads and a core-owned ephemeral byte cache. A portable Rust media-preparation module emits PNG/JPEG/WebP variant descriptors before send. Main and thread composers share one presentation component and dispatch target-correlated commands; thread media and scheduled sends preserve Matrix thread relations.

**Tech Stack:** Rust reducers/core actors, image codecs, Tauri 2, TypeScript/React, Vitest/Playwright, local Conduit/Tuwunel QA.

---

### Task 1: Canonical target-scoped staging state (#252 Phase A)

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `crates/koushi-state/src/state/timeline.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/timeline.rs`
- Modify: `crates/koushi-state/src/reducer/thread.rs`
- Modify: `crates/koushi-state/src/state/mod.rs`
- Modify: `crates/koushi-state/src/lib.rs`
- Test: `crates/koushi-state/tests/upload_staging_state.rs`
- Test: `crates/koushi-state/tests/timeline_thread_state.rs`

- [ ] Add RED tests for main/thread targets, deterministic multi-file order, stale request IDs, preparation after target close, room switch, thread switch, logout cleanup, retry, and missing prepared variant.
- [ ] Run `cargo test -p koushi-state --test upload_staging_state --test timeline_thread_state`; expect thread-target and lifecycle tests to fail.
- [ ] Introduce:

```rust
#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ComposerTarget {
    Main { room_id: String },
    Thread { room_id: String, root_event_id: String },
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StagedUploadPreparation {
    Preparing { request_id: u64 },
    Ready { variants: Vec<PreparedUploadVariant>, selected_variant_id: String },
    Failed { failure_kind: MediaPreparationFailureKind, can_use_original: bool },
}
```

- [ ] Key the non-serialized staging store by `ComposerTarget` and staged ID. Project only the active main or open thread target into the relevant UI slice.
- [ ] Add guarded actions for stage requested, preparation ready/failed, select variant, retry, remove, clear target, and send terminal. Implement stale target/request no-ops.
- [ ] Amend `state-machine.md` with target-scoped transitions and cleanup guards.
- [ ] Run both state tests; expect PASS. Commit `feat(#252): model target-scoped upload preparation`.

### Task 2: Core ephemeral cache and portable image variants (#84 Phase A)

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/koushi-media/Cargo.toml`
- Create: `crates/koushi-media/src/lib.rs`
- Create: `crates/koushi-media/tests/image_variants.rs`
- Modify: `crates/koushi-core/Cargo.toml`
- Create: `crates/koushi-core/src/media_preparation.rs`
- Modify: `crates/koushi-core/src/lib.rs`
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Test: `crates/koushi-core/tests/runtime_timeline.rs`

- [ ] Add RED codec tests using generated synthetic PNG alpha, JPEG, WebP, and animated/unsupported bytes. Assert actual MIME/extension, dimensions, alpha preservation, metadata removal, no default larger candidate, and original-only fallback.
- [ ] Add RED core tests for bounded concurrent preparation, cancellation, stale completion, cache eviction, retry, selection, logout cleanup, and send referencing selected bytes without conversion.
- [ ] Run `cargo test -p koushi-media` and the focused core media tests; expect missing crate/command failures.
- [ ] Implement pure functions in `koushi-media`:

```rust
pub fn prepare_image_variants(
    source: &[u8],
    filename: &str,
    mime: &str,
    policy: &ImagePreparationPolicy,
) -> Result<Vec<PreparedImageVariant>, ImagePreparationError>;
```

- [ ] Support original/resized PNG, original/recompressed JPEG, original/recompressed WebP, preserve alpha, strip metadata on encoded outputs, derive thumbnail, and verify encoded output MIME from bytes.
- [ ] Implement an account-scoped `MediaPreparationRegistry` in core holding source/prepared bytes behind redacted IDs. Use bounded tasks and reliable correlated reducer delivery. Clear target/account entries on cancellation/logout.
- [ ] Add `StageUploadBytes`, `RetryUploadPreparation`, `SelectUploadVariant`, `RemoveStagedUpload`, and `SendPreparedUpload` commands with custom redacted Debug.
- [ ] Run codec/core tests; expect PASS. Run `cargo check --target wasm32-unknown-unknown -p koushi-state -p koushi-media`. Commit `feat(#84): prepare image variants before send`.

### Task 3: Tauri and frontend ingestion adapter (#252 Phase B)

**Files:**
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `apps/desktop/src-tauri/src/commands/timeline.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Create: `apps/desktop/src/domain/attachmentIngestion.ts`
- Create: `apps/desktop/src/domain/attachmentIngestion.test.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/composer.tsx`
- Modify: `apps/desktop/src/styles.css`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Test: `apps/desktop/src/components/composer.test.tsx`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] Add RED unit/browser tests dispatching real `DataTransfer.files` to textarea, toolbar/body, and footer with a PDF/archive, plus picker and paste. Assert all call one adapter with captured target and original FileList order.
- [ ] Add a Tauri config test asserting `app.windows[0].dragDropEnabled === false`; expect failure under current default config.
- [ ] Set `dragDropEnabled: false` and implement `ingestAttachmentFiles(target, files, client)` which reads bytes immediately and dispatches one typed stage command. Do not retain `stagedUploadFilesRef` as authoritative.
- [ ] Move dragenter/dragover/dragleave/drop to the full composer section; ignore non-file items and render a localized presentation-only overlay.
- [ ] Update browser fake semantics and every snapshot/contract mirror.
- [ ] Run focused Vitest, Playwright picker/paste/drop cases, Tauri tests, and typecheck; expect PASS. Commit `feat(#252): unify composer file ingestion`.

### Task 4: Prepared variant staging UI (#84 Phase B)

**Files:**
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/components/dialogs.tsx`
- Modify: `apps/desktop/src/components/panes.tsx`
- Modify: `apps/desktop/src/domain/mediaUrl.ts`
- Modify: `apps/desktop/src/styles.css`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Test: `apps/desktop/src/components/panes.test.tsx`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] Add RED tests for preparing/ready/failed states, actual preview, PNG/JPEG/WebP choices, exact size/dimensions/MIME/savings, Retry/Use original, selected-state confirmation only after Rust snapshot, and Send disabled while preparing.
- [ ] Replace Original/Ask/Compressed buttons with descriptor-driven variant buttons. Remove send-time `ImageCompressionDialog` and all canvas encoder helpers from `App.tsx`.
- [ ] Fetch preview bytes through an app-owned ephemeral handle, create/revoke object URLs on descriptor/target changes, and never store URLs in AppState.
- [ ] Make Send dispatch selected prepared IDs only; assert no decode/encode API is called on Send.
- [ ] Run focused Vitest, Playwright image-compression cases, typecheck, and IPC contract tests; expect PASS. Commit `feat(#84): render pre-send image variants`.

### Task 5: Thread formatted composer parity (#250 text/UI)

**Files:**
- Modify: `apps/desktop/src/components/composer.tsx`
- Modify: `apps/desktop/src/components/rightPanel.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/panes.tsx`
- Modify: `apps/desktop/src/styles.css`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Test: `apps/desktop/src/components/composer.test.tsx`
- Test: `apps/desktop/src/components/panes.test.tsx`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] Add RED tests for bold/italic/link/list/code, mention/emoji, shortcuts/IME, picker/paste/drop, target-isolated draft/attachments, removed Start heading, thread-root summary suppression, and room-timeline latest-reply timestamp.
- [ ] Extract a shared composer surface with explicit `surface: "main" | "thread"`, capabilities, target, draft, mention intent, staging projection, and callbacks. Delete the minimal duplicate ThreadComposer behavior.
- [ ] Pass thread target into the common ingestion adapter and render its target-scoped staged items.
- [ ] Add TimelineView presentation context so only thread context suppresses Start heading and the root reply-summary. Render Rust `thread_summary.latest_reply_timestamp_ms` in the main timeline with the existing adaptive timestamp formatter.
- [ ] Run focused component/E2E tests and typecheck; expect PASS. Commit `feat(#250): share full composer with threads`.

### Task 6: Thread media and scheduled-send relations (#250 Matrix contract)

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `crates/koushi-state/src/state/timeline.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/thread.rs`
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/store.rs`
- Modify: `apps/desktop/src-tauri/src/commands/timeline.rs`
- Test: `crates/koushi-core/tests/runtime_timeline.rs`
- Test: `crates/koushi-state/tests/timeline_thread_state.rs`
- Test: `apps/desktop/src-tauri/src/commands/mod.rs`

- [ ] Add RED tests proving prepared file/image events use `Relation::Thread`, captions remain in the media event, retry/cancel cannot escape target, and server/local scheduled thread sends retain the root relation.
- [ ] Introduce `ScheduledSendTarget::{Room, Thread}` and persist it in the encrypted scheduled-send store with legacy room-only backfill.
- [ ] Extend prepared upload and scheduled-send commands/events with `ComposerTarget`; build Matrix content relation in Rust before SDK send.
- [ ] If delayed events cannot carry thread content, project unsupported capability and hide the thread schedule control; never fall back to an unthreaded event.
- [ ] Amend state-machine diagrams and update redacted Debug implementations.
- [ ] Run state/core/Tauri focused tests and local homeserver thread/media scenario; expect relation assertions and private-data-free tokens to pass. Commit `feat(#250): preserve thread relations for media and scheduling`.

### Task 7: DTO/delta/fake/wire lockstep

**Files:**
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts` or current IPC mock file discovered by contract tests
- Test: Tauri serialization and frontend IPC contract suites

- [ ] Add RED serialization tests for targets, preparation states, variant descriptors, thread media relation, scheduled target, and latest-reply timestamp.
- [ ] Update every hand-maintained mirror and regenerate the checked-in contract artifact.
- [ ] Run Tauri serialization tests, `npm --prefix apps/desktop run test:ipc-contract`, and typecheck; expect PASS. Commit `test: lock composer media wire contracts`.

### Task 8: Platform and full completion gates

**Files:**
- Modify: `docs/qa/headless-basic-operations.md`
- Modify: `scripts/desktop-linux-gui-qa.mjs`
- Modify: platform CI workflows only where an existing safe packaged lane exists

- [ ] Add private-data-free tokens for generic file drop, prepared image variants, thread formatted reply, thread media relation, and cleanup.
- [ ] Run focused Rust, Tauri, Vitest, Playwright, IPC, typecheck, wasm, secret scan, and `git diff --check` gates.
- [ ] Run local Conduit and Tuwunel media/thread core scenarios with both core backends.
- [ ] Run packaged Linux OS-drop GUI evidence. Run existing macOS/Windows packaged gates or record required CI jobs; do not claim cross-platform acceptance without them.
- [ ] Generate the complete branch diff including new files and run `codex review -` against repository canon, this design, and both plans. Verify and resolve findings.
- [ ] Rebase/merge current `origin/main`, rerun affected gates, push once, open one PR with all four `Closes` lines, wait for CI, fix failures, and merge with `gh pr merge --merge --delete-branch`.
