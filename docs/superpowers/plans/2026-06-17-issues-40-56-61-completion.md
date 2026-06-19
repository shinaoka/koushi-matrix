# Issues #40, #56, #61 Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete issue #40 (ignore/block/report Phase B), issue #56 (attachment/file browser Phase A+B), and issue #61 (threads list Phase A+B) in dependency-safe order, keeping Phase A Rust-owned semantics before Phase B GUI evidence.

**Architecture:**
- #40 Phase B is a GUI/browser-headless layer over the already-merged Phase A Rust state/commands.
- #56 Phase A extends the existing encrypted search-index `AttachmentDocument` and wires `OpenFilesView`/`CloseFilesView` commands; Phase B renders the Rust-owned file list.
- #61 Phase A adds a new `ThreadsListState` and a `ThreadsListActor` wrapper around the SDK `ThreadListService`; Phase B renders the list and opens the thread pane.
- Shared state/command/event files are serialized by the main agent; module-local patches may use subagents.

**Tech Stack:** Rust workspace, Tauri v2, React/TypeScript, Playwright, Vitest, local Conduit/Tuwunel headless QA.

---

## Phase 1 — #40 Phase B: Ignore/Block/Report GUI

### Task 1: Tauri command handlers

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`

Add builders following the `build_set_local_user_alias_command` pattern:

```rust
pub(crate) fn build_ignore_user_command(
    request_id: u64,
    user_id: String,
) -> AccountCommand {
    AccountCommand::IgnoreUser { request_id, user_id }
}

pub(crate) fn build_unignore_user_command(
    request_id: u64,
    user_id: String,
) -> AccountCommand {
    AccountCommand::UnignoreUser { request_id, user_id }
}

pub(crate) fn build_report_user_command(
    request_id: u64,
    user_id: String,
    reason: Option<String>,
) -> AccountCommand {
    AccountCommand::ReportUser { request_id, user_id, reason: reason.unwrap_or_default() }
}

pub(crate) fn build_report_content_command(
    request_id: u64,
    room_id: String,
    event_id: String,
    reason: Option<String>,
) -> RoomCommand {
    RoomCommand::ReportContent { request_id, room_id, event_id, reason: reason.unwrap_or_default() }
}

pub(crate) fn build_report_room_command(
    request_id: u64,
    room_id: String,
    reason: Option<String>,
) -> RoomCommand {
    RoomCommand::ReportRoom { request_id, room_id, reason: reason.unwrap_or_default() }
}
```

Register Tauri commands in `lib.rs` and route to the existing `CoreCommand` boundary.

- [ ] Step 1: Add builders and Tauri commands
- [ ] Step 2: Register in invoke handler
- [ ] Step 3: Run `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`
- [ ] Step 4: Commit

### Task 2: TypeScript API surface

**Files:**
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/domain/types.ts` if fields missing

Add to `DesktopApi`:

```typescript
ignoreUser(userId: string): Promise<DesktopSnapshot>;
unignoreUser(userId: string): Promise<DesktopSnapshot>;
reportUser(userId: string, reason: string): Promise<DesktopSnapshot>;
reportContent(roomId: string, eventId: string, reason: string): Promise<DesktopSnapshot>;
reportRoom(roomId: string, reason: string): Promise<DesktopSnapshot>;
```

In `BrowserFakeApi`:
- For ignore/unignore, mutate `snapshot.state.profile.ignored_user_ids` and `ignored_user_update`.
- For report commands, enforce known-room guard and return a fresh snapshot.

In `TauriDesktopApi`, invoke the new Tauri commands.

- [ ] Step 1: Add interface methods
- [ ] Step 2: Implement browser fake mutations
- [ ] Step 3: Implement Tauri invocations
- [ ] Step 4: Run `npm --prefix apps/desktop run typecheck` and Vitest for backend
- [ ] Step 5: Commit

### Task 3: Context-menu affordances

**Files:**
- Modify: `apps/desktop/src/domain/contextMenus.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx`

Extend `ContextMenuActionId` with:

```typescript
| "ignoreUser"
| "unignoreUser"
| "reportUser"
| "reportContent"
| "reportRoom";
```

In `contextMenuItems` for message menus, add ignore/unignore/report-sender/report-message actions guarded by `sender !== currentUserId`. Add a report-room action to the room menu.

In `App.tsx` `runContextMenuAction`, dispatch `api.ignoreUser`, `api.unignoreUser`, `api.reportUser`, `api.reportContent`, `api.reportRoom`. For report actions, show a small reason dialog (use existing dialog pattern) before dispatch.

In `RoomInfoPanel.tsx`, add ignore/report buttons on member rows (subject to not being self) and a report-room entry.

- [ ] Step 1: Extend menu model
- [ ] Step 2: Wire handlers in App.tsx
- [ ] Step 3: Add RoomInfoPanel affordances
- [ ] Step 4: Run typecheck and component tests
- [ ] Step 5: Commit

### Task 4: i18n labels

**Files:**
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/i18n/messages.test.ts`

Add English and Japanese keys:

```typescript
context: {
  ignoreUser: { en: "Ignore user", ja: "ユーザーを無視" },
  unignoreUser: { en: "Unignore user", ja: "無視を解除" },
  reportUser: { en: "Report user", ja: "ユーザーを通報" },
  reportContent: { en: "Report message", ja: "メッセージを通報" },
  reportRoom: { en: "Report room", ja: "ルームを通報" },
},
dialog: {
  reportReasonTitle: { en: "Report reason", ja: "通報理由" },
  reportReasonLabel: { en: "Reason (optional)", ja: "理由（任意）" },
},
action: {
  report: { en: "Report", ja: "通報" },
}
```

- [ ] Step 1: Add labels
- [ ] Step 2: Update test coverage
- [ ] Step 3: Run `npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts`
- [ ] Step 4: Commit

### Task 5: Browser-headless tests

**Files:**
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

Add scenarios:
- Right-click a message → ignore sender → assert `ignore_user` invocation and `ignored_user_ids` update.
- Unignore via context menu → assert `unignore_user` invocation.
- Report user / content / room → assert corresponding invocation with reason.

Use harness invocation patterns from existing redact/local-alias tests.

- [ ] Step 1: Add ignore/report scenarios
- [ ] Step 2: Run Playwright focused lane
- [ ] Step 3: Commit

### Task 6: State-machine docs

**Files:**
- Modify: `docs/architecture/state-machine.md`

Add sections:
- Ignored-user update state machine (`Idle → Saving → Idle` with optimistic mutation and revert).
- Report commands are fire-and-forget one-shots; success via `ReportCompleted`.
- Privacy rule: user IDs, event IDs, reasons must not appear in logs/QA tokens.

- [ ] Step 1: Add docs
- [ ] Step 2: Run placeholder scan and `git diff --check`
- [ ] Step 3: Commit

---

## Phase 2 — #56 Phase A: Attachment Index + File Browser Core

### Task 7: Enrich attachment document model

**Files:**
- Modify: `crates/koushi-search/src/document.rs`
- Modify: `crates/koushi-state/src/state.rs`
- Modify: `crates/koushi-core/src/search.rs`

Add to `AttachmentDocument`:

```rust
pub thread_root: Option<String>,
pub encrypted: bool,
pub encryption_version: Option<String>,
pub width: Option<u32>,
pub height: Option<u32>,
pub is_edited: bool,
```

Add corresponding fields to `AttachmentResult` in `state.rs` and projection in `search.rs`.

- [ ] Step 1: Extend document/result types
- [ ] Step 2: Run `cargo test -p koushi-search --test attachment_query`
- [ ] Step 3: Commit

### Task 8: Feed metadata from timeline indexer

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`

In the timeline diff handler that builds `SearchIndexMessage::Upsert`, include:

```rust
thread_root: item.thread_root.clone(),
encrypted: media_source.encrypted,
encryption_version: media_source.encryption_version.clone(),
width: media_source.width,
height: media_source.height,
is_edited: item.is_edited,
```

- [ ] Step 1: Map fields into SearchIndexMessage
- [ ] Step 2: Add/update timeline tests
- [ ] Step 3: Run `cargo test -p koushi-core --lib timeline`
- [ ] Step 4: Commit

### Task 9: Wire files-view command boundary

**Files:**
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/src/search.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`

Add to `AppCommand`:

```rust
OpenFilesView { request_id: u64, scope: FilesViewScope, filter: AttachmentFilter, sort: AttachmentSort },
CloseFilesView { request_id: u64 },
RefreshFilesView { request_id: u64 },
```

In `runtime.rs`, dispatch these to the existing reducer actions (`FilesViewOpened`, `FilesViewClosed`, `FilesViewQueryRequested`). Route `AppEffect::SearchAttachments` to `SearchActor` (already intended).

Add Tauri command builders following existing patterns.

- [ ] Step 1: Add command variants and runtime routing
- [ ] Step 2: Add Tauri commands
- [ ] Step 3: Run state and core tests
- [ ] Step 4: Commit

### Task 10: Mirror contracts and tests

**Files:**
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `crates/koushi-state/tests/attachment_files_view_state.rs`

Add `threadRoot`, `encrypted`, `encryptionVersion`, `width`, `height`, `isEdited` to TypeScript `AttachmentResult`. Update browser fake, harness, and IPC mock snapshots. Update Tauri DTO tests if shape changes.

Add state tests for open/close/refresh transitions and new metadata round-trip.

- [ ] Step 1: Update TypeScript contracts
- [ ] Step 2: Update Rust state tests
- [ ] Step 3: Run typecheck + Tauri serialization contract test
- [ ] Step 4: Commit

---

## Phase 3 — #56 Phase B: File Browser GUI

### Task 11: Wire RoomInfoPanel / SpaceInfoPanel files entries

**Files:**
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx`
- Modify: `apps/desktop/src/components/SpaceInfoPanel.tsx`
- Modify: `apps/desktop/src/App.tsx`

Add `onClick` to the existing `room.files` entry in `RoomInfoPanel`. Add a Files entry to `SpaceInfoPanel`. In `App.tsx`, dispatch `api.openFilesView({ scope: "room" | "space" | "account", ... })` and render a new `FilesView` panel from `snapshot.state.files_view`.

Create `apps/desktop/src/components/FilesView.tsx` with rows showing kind icon, filename, sender, date, size; sort/filter controls dispatch typed commands.

- [ ] Step 1: Create FilesView component
- [ ] Step 2: Wire panel entries
- [ ] Step 3: Run typecheck + component tests
- [ ] Step 4: Commit

### Task 12: i18n and browser-headless tests for file browser

**Files:**
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/i18n/messages.test.ts`
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

Add labels: `room.files`, `files.title`, `files.kind.image`, `files.kind.video`, `files.kind.audio`, `files.kind.file`, `files.sort.newest`, `files.sort.filename`, `files.filter.all`.

Add browser-headless scenario: open room Files → list reflects Rust projection; open space Files → aggregated across child rooms; filter/sort drive the Rust query.

- [ ] Step 1: Add i18n
- [ ] Step 2: Add e2e scenario
- [ ] Step 3: Run focused Playwright lane
- [ ] Step 4: Commit

---

## Phase 4 — #61 Phase A: Threads List Core

### Task 13: Threads-list state machine

**Files:**
- Modify: `crates/koushi-state/src/state.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer.rs`
- Modify: `crates/koushi-state/src/effect.rs`

Add:

```rust
pub enum ThreadsListState {
    Closed,
    Loading { room_id: String, request_id: u64 },
    Open { room_id: String, request_id: u64, items: Vec<ThreadsListItem>, is_paginating: bool, end_reached: bool },
    Failed { room_id: String, request_id: u64, failure_kind: OperationFailureKind },
}

pub struct ThreadsListItem {
    pub root_event_id: String,
    pub root_sender: String,
    pub root_sender_label: Option<String>,
    pub root_body_preview: Option<String>,
    pub root_timestamp_ms: Option<u64>,
    pub latest_event_id: Option<String>,
    pub latest_sender: Option<String>,
    pub latest_sender_label: Option<String>,
    pub latest_body_preview: Option<String>,
    pub latest_timestamp_ms: Option<u64>,
    pub reply_count: u32,
}
```

Add actions: `OpenThreadsList`, `ThreadsListOpened`, `ThreadsListUpdated`, `ThreadsListPaginationCompleted`, `ThreadsListFailed`, `PaginateThreadsList`, `CloseThreadsList`.

Add effects: `SubscribeThreadsList`, `PaginateThreadsList`, `UnsubscribeThreadsList`, `UiEvent::ThreadsListChanged`.

Reducer rules:
- Open requires session ready and active room match.
- Loading → Open/Failed only for matching request.
- Paginate only when Open, not already paginating, and not end_reached.
- Room switch / logout clears to Closed.

- [ ] Step 1: Add state/actions/effects
- [ ] Step 2: Implement reducer transitions
- [ ] Step 3: Add `crates/koushi-state/tests/threads_list_state.rs`
- [ ] Step 4: Run state tests
- [ ] Step 5: Commit

### Task 14: ThreadsListActor SDK wrapper

**Files:**
- Create: `crates/koushi-core/src/threads_list.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/lib.rs`

Create `ThreadsListActor` that:
- Gets `Room` via `session.client().get_room(...)`.
- Creates `ThreadListService::new(room)`.
- Takes initial `items()` snapshot and emits `ThreadsListEvent::Opened`.
- Spawns a relay task for `subscribe_to_items_updates()` and `subscribe_to_pagination_state_updates()`.
- Handles `PaginateThreadsList` by calling `service.paginate().await`.
- Drops on unsubscribe/room switch/logout.

Projection to `ThreadsListItem` reuses existing display-label resolution.

- [ ] Step 1: Create actor
- [ ] Step 2: Spawn from AccountActor on new message variant
- [ ] Step 3: Add core integration tests
- [ ] Step 4: Commit

### Task 15: Core commands/events and Tauri DTOs

**Files:**
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`

Add `AppCommand::OpenThreadsList / CloseThreadsList / PaginateThreadsList`. Add `CoreEvent::ThreadsList(ThreadsListEvent)` with variants `Opened`, `Updated`, `PaginationCompleted`, `Failed`.

Wire runtime effects to AccountActor. Mirror `ThreadsListState`/`ThreadsListItem` in Tauri DTO and TypeScript contracts. Update browser fake/harness/mocks.

- [ ] Step 1: Add Rust commands/events
- [ ] Step 2: Wire runtime
- [ ] Step 3: Mirror DTOs and TS contracts
- [ ] Step 4: Run `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` and typecheck
- [ ] Step 5: Commit

---

## Phase 5 — #61 Phase B: Threads List GUI

### Task 16: Threads list UI

**Files:**
- Create: `apps/desktop/src/components/ThreadsListView.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx` or header

Render `snapshot.state.threads_list.items` as rows (root preview, latest reply, reply count). Clicking a row dispatches `api.openThread(root_event_id)`. Add a "Threads" entry in the room header/right panel to open the list.

- [ ] Step 1: Create component
- [ ] Step 2: Wire header entry and App.tsx
- [ ] Step 3: Run typecheck + component tests
- [ ] Step 4: Commit

### Task 17: i18n and browser-headless tests for threads list

**Files:**
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/i18n/messages.test.ts`
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`

Add labels: `threads.title`, `threads.open`, `threads.replyCount`.

Add e2e scenario: open Threads list → reflects Rust projection; click row → thread pane opens that root.

- [ ] Step 1: Add i18n
- [ ] Step 2: Add e2e scenario
- [ ] Step 3: Run focused Playwright lane
- [ ] Step 4: Commit

---

## Phase 6 — Final Integration & Closure

### Task 18: Full verification

Run:

```bash
cargo fmt --check
cargo test -p koushi-state -p koushi-core -p koushi-sdk -p koushi-key -p koushi-search
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
node scripts/desktop-release-gate-check.mjs --no-compile
```

- [ ] Step 1: Run verification
- [ ] Step 2: Fix any failures
- [ ] Step 3: Commit fixes

### Task 19: Issue comments and close

Comment on #40, #56, #61 with token-only evidence and close them. Update #12 umbrella comment.

- [ ] Step 1: Comment/close #40, #56, #61
- [ ] Step 2: Update #12 umbrella

---

## Self-Review

- **Spec coverage:** #40 Phase B, #56 Phase A+B, #61 Phase A+B are all covered.
- **Placeholder scan:** No TBD/TODO placeholders.
- **Type consistency:** `ThreadsListItem`, `AttachmentResult`, and report command signatures match across Rust/TS boundaries.
