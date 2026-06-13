# Local GUI Basic Operations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build real room creation, space creation, and reply controls in the Linux desktop UI, verified only against disposable local homeservers until the final real-homeserver compatibility gate.

**Architecture:** Matrix operation semantics stay in Rust: `matrix-desktop-state` owns serializable UI/product state, `matrix-desktop-core` owns command routing and SDK effects, and React renders controls plus ephemeral presentation state. GUI QA is the last layer: first prove Rust state, Tauri IPC, and headless UI behavior, then drive the real Tauri app under Xvfb against local Conduit/Tuwunel.

**Tech Stack:** Rust `matrix-desktop-state`/`matrix-desktop-core`, Tauri v2 IPC, React/TypeScript/Vitest/Playwright, WebDriverIO + `tauri-driver`, local Conduit/Tuwunel homeservers.

---

## Current Understanding

- The blocker is not missing Matrix capability in core. `RoomCommand::CreateRoom`, `RoomCommand::CreateSpace`, `RoomCommand::SetSpaceChild`, and `TimelineCommand::SendReply` already exist and are covered by headless local QA.
- The blocker is the product interaction state model and UI/IPC exposure:
  - `apps/desktop/src/backend/client.ts` has no `createRoom`, `createSpace`, `setSpaceChild`, or `sendReply`.
  - `apps/desktop/src-tauri/src/commands.rs` exposes no Tauri commands for those operations.
  - `WorkspaceRail` renders `Add workspace` with no handler.
  - `Sidebar` renders `New message` with no handler.
  - the main composer sends only plain text.
  - the thread composer is an inert textarea.
  - `TimelineView` production rows do not expose reply actions.
- GUI iteration must be local-only. matrix.org is reserved for a final compatibility pass after local headless and Linux virtual-display lanes are green.

## Ownership Rules

- Rust owns product logic and operation semantics:
  - selected room/space/thread target
  - reply target and composer mode
  - pending Matrix operation status
  - operation success/failure interpretation
  - room/space list projection after create/link
- React may own presentation-only state:
  - dialog open/closed state
  - raw unsent form text before submit
  - focus, hover, menu position, viewport metrics, scroll anchors
- If a state value changes the shape of a Matrix command, it belongs in Rust state or a Rust command/event response, not only in React.
- Local GUI scenarios use synthetic data and disposable local homeservers. They must not touch matrix.org.

## Files And Responsibilities

- `AGENTS.md`: operational guardrails for all agents.
- `docs/qa/headless-basic-operations.md`: QA lane contract and local-only GUI policy.
- `crates/matrix-desktop-state/src/state.rs`: serializable product state for composer mode and pending basic operations.
- `crates/matrix-desktop-state/src/action.rs`: reducer actions for composer target and operation status.
- `crates/matrix-desktop-state/src/reducer.rs`: state transitions and UI events.
- `crates/matrix-desktop-state/tests/basic_operation_state.rs`: Rust reducer contract for create/reply state.
- `crates/matrix-desktop-core/src/command.rs`: public command boundary additions.
- `crates/matrix-desktop-core/src/runtime.rs`: `AppCommand` handling for UI-semantic state.
- `crates/matrix-desktop-core/src/room.rs`: create/link success/failure status projection.
- `crates/matrix-desktop-core/src/timeline.rs`: reply send status projection if the existing send completion signal is not enough for the UI.
- `apps/desktop/src-tauri/src/commands.rs`: transport-only Tauri commands and command builders.
- `apps/desktop/src-tauri/src/lib.rs`: Tauri invoke handler registration.
- `apps/desktop/src/domain/types.ts`: generated or hand-maintained snapshot types matching Rust state.
- `apps/desktop/src/backend/browserFakeApi.ts`: browser preview implementation for the new API methods.
- `apps/desktop/src/backend/client.ts`: Tauri IPC client methods.
- `apps/desktop/src/App.tsx`: render dialogs, composer modes, and handlers.
- `apps/desktop/src/components/TimelineView.tsx`: production timeline row reply affordance.
- `apps/desktop/src/domain/qaTitle.ts`: private-data-free QA tokens for create/reply status.
- `scripts/desktop-linux-gui-qa.mjs`: fast local GUI scenarios and skip-build iteration support.

## Task 1: Keep The Guardrails Visible

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/qa/headless-basic-operations.md`

- [x] **Step 1: Add Rust-owned product state rule**

`AGENTS.md` now says that Matrix operation semantics belong in
`matrix-desktop-state`/`matrix-desktop-core`; React may keep only ephemeral
presentation state.

- [x] **Step 2: Add local-only GUI operation policy**

`AGENTS.md` and `docs/qa/headless-basic-operations.md` now say that room,
space, and reply GUI iteration uses local Conduit/Tuwunel only. matrix.org is
reserved for the final compatibility gate.

- [x] **Step 3: Verify docs formatting**

Run:

```bash
git diff --check AGENTS.md docs/qa/headless-basic-operations.md docs/superpowers/plans/2026-06-13-local-gui-basic-operations.md
```

Expected: exit 0.

## Task 2: Add Rust State For Composer Mode And Operation Status

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-state/src/lib.rs`
- Create: `crates/matrix-desktop-state/tests/basic_operation_state.rs`

- [x] **Step 1: Write failing reducer tests**

Create `crates/matrix-desktop-state/tests/basic_operation_state.rs`:

```rust
use matrix_desktop_state::{
    reduce, AppAction, AppState, BasicOperationState, ComposerMode,
    ComposerState, RoomSummary, SessionInfo, SessionState, TimelinePaneState,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@qa:localhost".to_owned(),
            device_id: "LOCALDEVICE".to_owned(),
        }),
        rooms: vec![RoomSummary {
            room_id: "!room:localhost".to_owned(),
            display_name: "QA Seed Room".to_owned(),
            is_dm: false,
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: vec![],
        }],
        timeline: TimelinePaneState {
            room_id: Some("!room:localhost".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: ComposerState::default(),
        },
        ..AppState::default()
    }
}

#[test]
fn composer_reply_target_is_rust_state() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::ComposerReplyTargetSelected {
            room_id: "!room:localhost".to_owned(),
            event_id: "$root:localhost".to_owned(),
        },
    );

    assert_eq!(
        state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: "$root:localhost".to_owned(),
        }
    );

    reduce(&mut state, AppAction::ComposerReplyCancelled);
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
}

#[test]
fn room_creation_status_is_rust_state() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::BasicOperationStarted {
            operation: BasicOperationState::CreatingRoom {
                name: "Local QA Room".to_owned(),
            },
        },
    );

    assert_eq!(
        state.basic_operation,
        BasicOperationState::CreatingRoom {
            name: "Local QA Room".to_owned(),
        }
    );

    reduce(&mut state, AppAction::BasicOperationFinished);
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
}
```

Run:

```bash
cargo test -p matrix-desktop-state --test basic_operation_state
```

Expected: fails because the types/actions do not exist.

- [x] **Step 2: Add serializable Rust state**

In `crates/matrix-desktop-state/src/state.rs`, extend `AppState` and
`ComposerState`:

```rust
pub struct AppState {
    pub session: SessionState,
    pub auth: AuthDiscoveryState,
    pub sync: SyncState,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub timeline: TimelinePaneState,
    pub thread: ThreadPaneState,
    pub search: SearchState,
    pub basic_operation: BasicOperationState,
    pub errors: Vec<AppError>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerState {
    pub pending_transaction_id: Option<String>,
    pub draft: String,
    pub mode: ComposerMode,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ComposerMode {
    #[default]
    Plain,
    Reply { in_reply_to_event_id: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BasicOperationState {
    #[default]
    Idle,
    CreatingRoom { name: String },
    CreatingSpace { name: String },
    LinkingSpaceChild { space_id: String, child_room_id: String },
}
```

Update `Default for AppState` with:

```rust
basic_operation: BasicOperationState::Idle,
```

Re-export `BasicOperationState` and `ComposerMode` from `src/lib.rs`.

- [x] **Step 3: Add reducer actions**

In `crates/matrix-desktop-state/src/action.rs`, import the new type and add:

```rust
BasicOperationState,
```

Add `AppAction` variants:

```rust
BasicOperationStarted { operation: BasicOperationState },
BasicOperationFinished,
BasicOperationFailed { message: String },
ComposerReplyTargetSelected { room_id: String, event_id: String },
ComposerReplyCancelled,
```

- [x] **Step 4: Implement reducer transitions**

In `crates/matrix-desktop-state/src/reducer.rs`, add match arms:

```rust
AppAction::BasicOperationStarted { operation } => {
    if !is_session_ready(state) {
        return Vec::new();
    }
    state.basic_operation = operation;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}
AppAction::BasicOperationFinished => {
    if state.basic_operation == BasicOperationState::Idle {
        return Vec::new();
    }
    state.basic_operation = BasicOperationState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}
AppAction::BasicOperationFailed { message } => {
    state.basic_operation = BasicOperationState::Idle;
    state.errors.push(AppError {
        code: "basic_operation_failed".to_owned(),
        message,
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}
AppAction::ComposerReplyTargetSelected { room_id, event_id } => {
    if !is_session_ready(state)
        || state.timeline.room_id.as_deref() != Some(room_id.as_str())
    {
        return Vec::new();
    }
    state.timeline.composer.mode = ComposerMode::Reply {
        in_reply_to_event_id: event_id,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}
AppAction::ComposerReplyCancelled => {
    let Some(room_id) = state.timeline.room_id.clone() else {
        return Vec::new();
    };
    if state.timeline.composer.mode == ComposerMode::Plain {
        return Vec::new();
    }
    state.timeline.composer.mode = ComposerMode::Plain;
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}
```

Run:

```bash
cargo test -p matrix-desktop-state --test basic_operation_state
cargo test -p matrix-desktop-state
```

Expected: both pass.

## Task 3: Route UI-Semantic State Through Core

**Files:**
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Test: `crates/matrix-desktop-core/src/tests.rs`

- [x] **Step 1: Write failing core command tests**

Add tests to `crates/matrix-desktop-core/src/tests.rs` proving that
`AppCommand` updates composer reply state through the reducer:

```rust
use std::time::Duration;

use matrix_desktop_state::{
    AppAction, ComposerMode, RoomSummary, SessionInfo, SessionState,
};

use crate::{
    AppCommand, CoreCommand, CoreConnection, CoreRuntime, executor,
};

#[tokio::test]
async fn app_command_sets_and_clears_reply_target() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime.inject_actions(vec![
        AppAction::RestoreSessionSucceeded(session_info()),
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary("!room:example.test")],
        },
        AppAction::SelectRoom {
            room_id: "!room:example.test".to_owned(),
        },
        AppAction::TimelineSubscribed {
            room_id: "!room:example.test".to_owned(),
        },
    ]).await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    let set_request = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SetComposerReplyTarget {
        request_id: set_request,
        room_id: "!room:example.test".to_owned(),
        event_id: "$root:example.test".to_owned(),
    }))
    .await
    .expect("set reply target command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            state.timeline.composer.mode,
            ComposerMode::Reply { ref in_reply_to_event_id }
                if in_reply_to_event_id == "$root:example.test"
        )
    })
    .await;
    assert!(matches!(snapshot.timeline.composer.mode, ComposerMode::Reply { .. }));

    let cancel_request = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::CancelComposerReply {
        request_id: cancel_request,
    }))
    .await
    .expect("cancel reply target command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.mode == ComposerMode::Plain
    })
    .await;
    assert_eq!(snapshot.timeline.composer.mode, ComposerMode::Plain);
}

fn room_summary(room_id: &str) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: "QA Room".to_owned(),
        is_dm: false,
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        parent_space_ids: vec![],
    }
}

async fn wait_for_state<F>(
    connection: &mut CoreConnection,
    predicate: F,
) -> matrix_desktop_state::AppState
where
    F: Fn(&matrix_desktop_state::AppState) -> bool,
{
    for _ in 0..200 {
        let snapshot = connection.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }
    panic!("state predicate was not satisfied");
}
```

Run:

```bash
cargo test -p matrix-desktop-core app_command_sets_and_clears_reply_target
```

Expected: fails because the commands are missing.

- [x] **Step 2: Add `AppCommand` variants**

In `crates/matrix-desktop-core/src/command.rs`:

```rust
#[derive(Debug)]
pub enum AppCommand {
    Shutdown { request_id: RequestId },
    SetComposerReplyTarget {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    CancelComposerReply { request_id: RequestId },
}
```

Update `CoreCommand::request_id()` for the new variants.

- [x] **Step 3: Reduce app commands in `AppActor`**

In `crates/matrix-desktop-core/src/runtime.rs`, replace the current
`CoreCommand::App(_)` branch with:

```rust
CoreCommand::App(app_command) => match app_command {
    AppCommand::Shutdown { .. } => false,
    AppCommand::SetComposerReplyTarget {
        room_id,
        event_id,
        ..
    } => {
        let effects = reduce(
            &mut self.state,
            AppAction::ComposerReplyTargetSelected { room_id, event_id },
        );
        let _ = effects;
        true
    }
    AppCommand::CancelComposerReply { .. } => {
        let effects = reduce(&mut self.state, AppAction::ComposerReplyCancelled);
        let _ = effects;
        true
    }
},
```

Run:

```bash
cargo test -p matrix-desktop-core app_command_sets_and_clears_reply_target
cargo test -p matrix-desktop-core --lib
```

Expected: both pass.

## Task 4: Expose Room, Space, And Reply IPC

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/domain/types.ts`
- Test: `apps/desktop/src-tauri/src/commands.rs`
- Test: `apps/desktop/src/domain/desktopModel.test.ts`

- [x] **Step 1: Add failing Tauri command-builder tests**

Extend `tauri_command_routes_build_expected_core_commands` in
`apps/desktop/src-tauri/src/commands.rs` to assert builders for:

```rust
build_create_room_command(fake_request_id(16), "Local QA Room".to_owned())
build_create_space_command(fake_request_id(17), "Local QA Space".to_owned())
build_set_space_child_command(
    fake_request_id(18),
    "!space:example.org".to_owned(),
    "!room:example.org".to_owned(),
    "example.org".to_owned(),
)
build_send_reply_command(
    fake_request_id(19),
    active_account_key.clone(),
    room_id.clone(),
    "desktop-reply-1".to_owned(),
    "$root".to_owned(),
    "reply body".to_owned(),
)
```

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml tauri_command_routes_build_expected_core_commands
```

Expected: fails because the builders are missing.

- [x] **Step 2: Add transport-only Tauri commands**

Add commands:

```rust
#[tauri::command]
pub async fn create_room(
    name: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>

#[tauri::command]
pub async fn create_space(
    name: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>

#[tauri::command]
pub async fn set_space_child(
    space_id: String,
    child_room_id: String,
    via_server: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>

#[tauri::command]
pub async fn set_composer_reply_target(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>

#[tauri::command]
pub async fn cancel_composer_reply(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>

#[tauri::command]
pub async fn send_reply(
    room_id: String,
    in_reply_to_event_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String>
```

The commands must allocate request IDs, submit only `CoreCommand`, update the
QA title, and return `current_snapshot`. The `create_*` commands should wait
for the correlated `RoomEvent` before returning when practical, following the
existing `select_room` event-wait pattern.

- [x] **Step 3: Register commands**

Add the new commands to `tauri::generate_handler!` in
`apps/desktop/src-tauri/src/lib.rs`.

- [x] **Step 4: Add TS API methods**

Extend `DesktopApi` and `TauriDesktopApi`:

```ts
createRoom(name: string): Promise<DesktopSnapshot>;
createSpace(name: string): Promise<DesktopSnapshot>;
setSpaceChild(spaceId: string, childRoomId: string, viaServer: string): Promise<DesktopSnapshot>;
setComposerReplyTarget(roomId: string, eventId: string): Promise<DesktopSnapshot>;
cancelComposerReply(): Promise<DesktopSnapshot>;
sendReply(roomId: string, inReplyToEventId: string, body: string): Promise<DesktopSnapshot>;
```

- [x] **Step 5: Update browser fake**

`createRoom` appends a non-DM room, selects it, and composes the sidebar.
`createSpace` appends a space and selects it. `setSpaceChild` adds the room ID
to `SpaceSummary.child_room_ids` and the space ID to
`RoomSummary.parent_space_ids`. `sendReply` appends a timeline message with
`reply_count: 0` and increments the root message `reply_count`.

Run:

```bash
npm --prefix apps/desktop run test -- src/domain/desktopModel.test.ts
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Expected: both pass.

## Task 5: Wire Minimal Product Controls

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/App.test.tsx`

- [ ] **Step 1: Add failing render tests**

Add tests to `apps/desktop/src/App.test.tsx`:

```ts
test("workspace rail exposes create space control", async () => {
  vi.stubGlobal("window", { location: { search: "" } });
  const { WorkspaceRail } = await import("./App");
  const api = createBrowserFakeApi();
  const snapshot = await api.getSnapshot();

  const markup = renderToStaticMarkup(
    <WorkspaceRail
      snapshot={snapshot}
      onCreateSpace={() => undefined}
      onOpenContextMenu={() => undefined}
      onOpenUserSettings={() => undefined}
      onSelectSpace={() => undefined}
    />
  );

  expect(markup).toContain('aria-label="Create space"');
});

test("composer renders reply mode from snapshot state", async () => {
  vi.stubGlobal("window", { location: { search: "" } });
  const { Composer } = await import("./App");

  const markup = renderToStaticMarkup(
    <Composer
      composerMode={{ kind: "reply", in_reply_to_event_id: "$root" }}
      isSending={false}
      roomName="QA Room"
      value="reply"
      onCancelReply={() => undefined}
      onSend={() => undefined}
      onValueChange={() => undefined}
    />
  );

  expect(markup).toContain("Replying");
  expect(markup).toContain('aria-label="Cancel reply"');
});
```

Run:

```bash
npm --prefix apps/desktop run test -- src/App.test.tsx
```

Expected: fails because the props/controls do not exist.

- [ ] **Step 2: Add create room and create space dialogs**

Use the existing icon-button visual language. The dialog state and typed form
text may remain React-local; submit calls the Rust-backed API. Required
controls:

- Workspace rail plus button: `aria-label="Create space"`.
- Sidebar new message button: `aria-label="Create room"`.
- Dialog text input: `aria-label="Space name"` or `aria-label="Room name"`.
- Submit button: `aria-label="Submit create space"` or
  `aria-label="Submit create room"`.
- Cancel button: `aria-label="Cancel create"`.

After `createRoom`, select the created room if the backend snapshot exposes it
as active. After `createSpace`, select the created space. Keep `isBusy` and
`snapshot.state.basic_operation` aligned so the UI cannot double-submit.

- [ ] **Step 3: Add reply action to production timeline rows**

Extend `TimelineView`:

```ts
export interface TimelineRowActionHandlers {
  onReply: (roomId: string, eventId: string) => void;
}
```

Render a small icon button for real event rows:

```tsx
{"Event" in item.id ? (
  <button
    className="message-action"
    type="button"
    aria-label="Reply to message"
    onClick={() => onReply(roomId, item.id.Event.event_id)}
  >
    <MessageCircle size={14} />
  </button>
) : null}
```

Add `data-reply="true"` to rows where `item.in_reply_to_event_id` is present.
Do not expose the raw reply event ID in QA title tokens.

- [ ] **Step 4: Wire composer reply mode**

In `App.tsx`, derive composer mode from
`snapshot.state.timeline.composer.mode`. `sendText()` should dispatch
`api.sendReply(...)` when mode is reply, otherwise `api.sendText(...)`.
Successful sends clear the React draft; Rust clears the semantic reply mode
through `cancelComposerReply` or a reducer action associated with send
completion.

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- src/App.test.tsx src/domain/desktopModel.test.ts
```

Expected: both pass.

## Task 6: Add Headless UI And IPC Contract Coverage

**Files:**
- Modify: `apps/desktop/e2e`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`

- [ ] **Step 1: Add Playwright scenarios**

Add headless UI tests that mock Tauri IPC and prove:

- create room dialog opens, submits `create_room`, and closes on success.
- create space dialog opens, submits `create_space`, and closes on success.
- reply button sets Rust-backed reply target through `set_composer_reply_target`.
- composer submit calls `send_reply` when the snapshot says reply mode.

Run:

```bash
npm --prefix apps/desktop run test:ui-headless
```

Expected: pass before GUI QA begins.

## Task 7: Make Linux GUI QA Fast Enough For Iteration

**Files:**
- Modify: `scripts/desktop-linux-gui-qa.mjs`
- Modify: `AGENTS.md`
- Test: `apps/desktop/src/scripts/releaseScripts.test.ts`

- [ ] **Step 1: Add skip-build support**

Add `--skip-build` and `--app-binary=PATH` support. When `--skip-build` is
present, the runner must not execute:

```bash
npm run tauri build -- --debug --no-bundle
```

Instead it resolves the existing binary through `--app-binary` or
`resolveDebugAppBinary()`.

Fast loop:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run tauri build -- --debug --no-bundle

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=local-send \
    --server=conduit \
    --skip-build \
    --artifact-dir=artifacts/linux-gui-local-send-fast \
    --timeout-ms=180000
```

- [ ] **Step 2: Keep local server startup disposable**

Do not add persistent matrix.org profiles or real-account login to the GUI
basic-operation runner. Disposable local server startup is cheaper than
debugging leaked remote artifacts.

- [ ] **Step 3: Document the fast loop**

Add the skip-build command to `AGENTS.md` under `Fast Linux GUI Inner Loop`.

- [ ] **Step 4: Add source-level release check for skip-build**

Extend `apps/desktop/src/scripts/releaseScripts.test.ts` so the release guard
asserts the Linux GUI runner supports:

```text
--skip-build
```

Run:

```bash
node scripts/desktop-linux-gui-qa.mjs --check-tools
npm --prefix apps/desktop run test -- src/scripts/releaseScripts.test.ts
```

Expected: both pass.

## Task 8: Add Local GUI Basic Operation Scenarios

**Files:**
- Modify: `scripts/desktop-linux-gui-qa.mjs`
- Modify: `docs/qa/headless-basic-operations.md`
- Test: `apps/desktop/src/scripts/releaseScripts.test.ts`

- [ ] **Step 1: Add scenario names**

Extend the runner's scenario switch:

```js
if (guiScenario === "local-create-room") {
  await runLocalCreateRoomScenario();
  return;
}
if (guiScenario === "local-create-space") {
  await runLocalCreateSpaceScenario();
  return;
}
if (guiScenario === "local-reply") {
  await runLocalReplyScenario();
  return;
}
```

Add list tokens:

```text
scenario local-create-room
scenario local-create-space
scenario local-reply
```

- [ ] **Step 2: Drive real controls**

Each scenario must call `startLocalGuiScenario()`, complete FIFO login, wait
for `qaStatusIsReady`, then use WebDriver controls:

- `local-create-room`: click `aria-label="Create room"`, fill
  `aria-label="Room name"`, click `aria-label="Submit create room"`, wait for
  the room count or title token to increase, print `gui_local_create_room=ok`.
- `local-create-space`: click `aria-label="Create space"`, fill
  `aria-label="Space name"`, click `aria-label="Submit create space"`, wait for
  `spaces` to increase, print `gui_local_create_space=ok`.
- `local-reply`: click the first real event row's `aria-label="Reply to message"`,
  fill the main composer, press Enter, wait for `reply=sent` or a
  `data-reply="true"` row containing the synthetic body, print
  `gui_local_reply=ok`.

- [ ] **Step 3: Keep assertions private-data-free**

Allowed stdout tokens:

```text
gui_local_create_room=ok
gui_local_create_space=ok
gui_local_reply=ok
run_dir=...
window_state_path_contract=ok
notification_dbus=ok
```

Do not print access tokens, passwords, recovery secrets, Matrix IDs from real
accounts, or raw SDK errors.

- [ ] **Step 4: Run focused local GUI lanes**

Run:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=local-create-room \
    --server=conduit \
    --artifact-dir=artifacts/linux-gui-local-create-room \
    --timeout-ms=180000

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=local-create-space \
    --server=conduit \
    --artifact-dir=artifacts/linux-gui-local-create-space \
    --timeout-ms=180000

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=local-reply \
    --server=conduit \
    --artifact-dir=artifacts/linux-gui-local-reply \
    --timeout-ms=180000
```

Expected: each command prints its `gui_local_*=ok` token with no errors.

- [ ] **Step 5: Add source-level release checks for scenario names**

Extend `apps/desktop/src/scripts/releaseScripts.test.ts` so the release guard
asserts the Linux GUI runner supports:

```text
scenario local-create-room
scenario local-create-space
scenario local-reply
```

Run:

```bash
npm --prefix apps/desktop run test -- src/scripts/releaseScripts.test.ts
```

Expected: pass.

## Task 9: Reviewer Gates

**Files:**
- All files touched by Tasks 2-8

- [ ] **Step 1: Rust gates**

Run:

```bash
cargo test -p matrix-desktop-state
cargo test -p matrix-desktop-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo check --target wasm32-unknown-unknown -p matrix-desktop-state -p matrix-desktop-search
```

Expected: all pass.

- [ ] **Step 2: Frontend gates**

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run qa:secret-scan
```

Expected: all pass.

- [ ] **Step 3: Local Matrix operation gates**

Run:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:headless-basic:local

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=local-create-room \
    --server=conduit \
    --artifact-dir=artifacts/linux-gui-local-create-room-review \
    --timeout-ms=180000

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=local-create-space \
    --server=conduit \
    --artifact-dir=artifacts/linux-gui-local-create-space-review \
    --timeout-ms=180000

PATH=/tmp/matrix-desktop-local-qa-bin:$PATH \
  npm --prefix apps/desktop run qa:linux-gui -- \
    --scenario=local-reply \
    --server=conduit \
    --artifact-dir=artifacts/linux-gui-local-reply-review \
    --timeout-ms=180000
```

Expected: all local lanes pass. Do not run matrix.org for this implementation
review.

## Handoff

Execution model:

- Implementation: mini subagent per task or per small task batch.
- Audit: main agent reviews diffs, enforces Rust-owned state, runs gates, and
  checks docs.
- Commit cadence: one focused commit after each green task group.

Subagent start checklist:

1. Read `AGENTS.md`, this plan, `docs/architecture/overview.md`, and
   `docs/policies/engineering-rules.md`.
2. Confirm the current branch is `codex/prerequisite-spikes`.
3. Confirm no real homeserver command is required for the assigned task.
4. Start with the failing test for the assigned task.
5. Keep React-local state presentation-only. If it affects Matrix command
   semantics, move it to Rust state or stop for main-agent review.
6. Never print or commit credentials. Local synthetic credentials generated by
   the QA runner stay in ignored artifacts only.

Main-agent audit checklist:

1. `git diff --check` is clean.
2. No matrix.org GUI scenario or real-account retry path was added.
3. New Matrix operation semantics are represented in Rust state/core before
   React consumes them.
4. Tauri commands are transport adapters only: no SDK calls, no Matrix
   semantics, no credential handling.
5. UI controls have stable accessible labels for WebDriver.
6. QA title/stdout tokens are private-data-free.
7. Local Conduit/Tuwunel headless gates pass before Linux GUI gates are used
   for confidence.

Final acceptance for this plan:

```bash
cargo test -p matrix-desktop-state
cargo test -p matrix-desktop-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test
npm --prefix apps/desktop run test:ui-headless
npm --prefix apps/desktop run qa:secret-scan
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-basic:local
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-room --server=conduit --artifact-dir=artifacts/linux-gui-local-create-room-final --timeout-ms=180000
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-create-space --server=conduit --artifact-dir=artifacts/linux-gui-local-create-space-final --timeout-ms=180000
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-reply --server=conduit --artifact-dir=artifacts/linux-gui-local-reply-final --timeout-ms=180000
```
