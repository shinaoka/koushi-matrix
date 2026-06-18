# Umbrella #12 Non-Mac Completion Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the remaining umbrella #12 child issues that do not require macOS physical hardware: #38 Phase B (device/session manager GUI), #39 Phase B (sliding-sync room-list filters and mark read/unread GUI), and #44 (per-room notification mode + read-receipt/typing privacy opt-out, Phase A and B).

**Architecture:** Reuse the existing Rust-owned Phase A contracts where they already exist (#38 and #39), add the missing Rust contract for #44, then wire Tauri transport, TypeScript client, React components, browser fakes, and browser-headless GUI-operation tests. Product semantics stay Rust-owned; React renders snapshots and dispatches typed commands only.

**Tech Stack:** Rust workspace (`matrix-desktop-state`, `matrix-desktop-core`, `matrix-desktop-sdk`), Tauri v2 adapter, React/TypeScript frontend, Vitest, Playwright browser-headless tests, Linux virtual-display GUI QA.

---

## Execution Rules

- Work in a single feature branch based on current `origin/main`.
- Refresh the GitHub open-issue inventory before changing code.
- Phase A precedes Phase B for #44; #38 and #39 Phase A is already complete.
- Main agent owns shared files: `state.rs`, `action.rs`, `reducer.rs`, `command.rs`, `event.rs`, `runtime.rs`, Tauri DTO/commands, `coreEvents.ts`, `coreEvents.generated.json`, `types.ts`, `messages.ts`, `styles.css`, `App.tsx`, `browserFakeApi.ts`, and `tauriIpcMock.ts`.
- Subagents own module-local files and tests.
- Browser fakes may seed Rust-shaped snapshots and record typed commands, but must not recompute product semantics such as room-list filters, notification policy, unread counts, or privacy behavior.
- Update issue comments when each Phase lands; do not claim closure until acceptance criteria and the local gate set are satisfied.

## Selected Issues And Rationale

| Issue | Mac hardware? | Phase A | Phase B | Why selected |
|-------|---------------|---------|---------|--------------|
| #38 Device/session manager | No | Done (`6fc075f`) | Open | Tauri/UI wiring only; unblocks #47 account management. |
| #39 Sliding sync room-list filters | No | Done (`6fc075f`) | Open | Tauri/UI wiring only; unblocks #48 and #49. |
| #44 Per-room notifications + privacy | No | Not started | Not started | Self-contained extension of existing notification settings; no hardware dependency. |

Excluded: #66/#67 need macOS/native prompt or DBus smoke; #37/#45/#47 are large auth features best planned separately; #48/#49 depend on #39 Phase B; #50-#52/#56/#57/#59-#61 are large new event models; #9/#31 are final QA gates.

---

## Task 1: #38 Phase B — Device / Session Manager GUI

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/components/UserSettingsPanel.test.tsx`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

### Step 1: Add Tauri command wrappers for device/session operations

In `apps/desktop/src-tauri/src/commands.rs`, add these public command handlers after `update_settings`:

```rust
#[tauri::command]
pub async fn query_devices(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::QueryDevices { request_id }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn rename_device(
    device_ordinal: u64,
    display_name: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::RenameDevice {
            request_id,
            device_ordinal,
            display_name,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn delete_devices(
    device_ordinals: Vec<u64>,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::DeleteDevices {
            request_id,
            device_ordinals,
            auth: None,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}
```

Note: there is no `delete_other_devices` Tauri command. The React UI computes the list of non-current ordinals from the Rust-projected snapshot and dispatches `delete_devices` with explicit ordinals. The adapter must not derive product semantics.

Run:

```bash
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Expected: compiles.

### Step 2: Register the new Tauri commands

In `apps/desktop/src-tauri/src/lib.rs`, locate the `.invoke_handler` builder and add:

```rust
.query_devices,
.rename_device,
.delete_devices,
```

Use the exact pattern already used for neighbouring commands.

Run:

```bash
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Expected: compiles.

### Step 3: Add client methods

In `apps/desktop/src/backend/client.ts`, add to `DesktopApi` interface:

```typescript
queryDevices(): Promise<DesktopSnapshot>;
renameDevice(deviceOrdinal: number, displayName: string): Promise<DesktopSnapshot>;
deleteDevices(deviceOrdinals: number[]): Promise<DesktopSnapshot>;
```

Implement in `TauriDesktopApi`:

```typescript
async queryDevices(): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("query_devices");
}

async renameDevice(deviceOrdinal: number, displayName: string): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("rename_device", { deviceOrdinal, displayName });
}

async deleteDevices(deviceOrdinals: number[]): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("delete_devices", { deviceOrdinals });
}
```

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: type errors in `BrowserFakeApi` because methods are missing.

### Step 4: Implement browser fake device/session methods

In `apps/desktop/src/backend/browserFakeApi.ts`, add to `BrowserFakeApi`:

```typescript
async queryDevices(): Promise<DesktopSnapshot> {
  if (!this.canUseSyncedViews()) {
    return this.getSnapshot();
  }
  this.snapshot.state.device_sessions = {
    kind: "loaded",
    devices: [
      {
        device_ordinal: 1,
        display_name: "Current session",
        current: true,
        verified: true,
        inactive: false
      },
      {
        device_ordinal: 2,
        display_name: "Other session",
        current: false,
        verified: false,
        inactive: true
      }
    ]
  };
  return this.getSnapshot();
}

async renameDevice(deviceOrdinal: number, displayName: string): Promise<DesktopSnapshot> {
  if (this.snapshot.state.device_sessions.kind === "loaded") {
    for (const device of this.snapshot.state.device_sessions.devices) {
      if (device.device_ordinal === deviceOrdinal) {
        device.display_name = displayName;
      }
    }
  }
  return this.getSnapshot();
}

async deleteDevices(deviceOrdinals: number[]): Promise<DesktopSnapshot> {
  if (this.snapshot.state.device_sessions.kind === "loaded") {
    this.snapshot.state.device_sessions.devices =
      this.snapshot.state.device_sessions.devices.filter(
        (d) => !deviceOrdinals.includes(d.device_ordinal)
      );
  }
  return this.getSnapshot();
}
```

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: typecheck passes.

### Step 5: Update browser-headless mocks

In `apps/desktop/src/test/tauriIpcMock.ts` and `appHarnessMain.tsx`, add recorded invocations for the three new command names and default responses. Keep the existing `device_sessions: { kind: "idle" }` default unless a test seeds loaded state. Do not record passwords in mock invocations.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/test
```

Expected: passes.

### Step 6: Add i18n labels

In `apps/desktop/src/i18n/messages.ts`, add:

```typescript
"settings.sessions": "Sessions",
"settings.currentSession": "Current session",
"settings.otherSessions": "Other sessions",
"settings.signOut": "Sign out",
"settings.signOutOthers": "Sign out all other sessions",
"settings.renameDevice": "Rename",
"settings.deviceVerified": "Verified",
"settings.deviceUnverified": "Unverified",
"settings.deviceInactive": "Inactive",
"settings.deviceNamePlaceholder": "Device name",
"settings.sessionsLoading": "Loading sessions…",
"settings.sessionsLoadFailed": "Could not load sessions."
```

Run:

```bash
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
```

Expected: passes.

### Step 7: Render sessions list in UserSettingsPanel

In `apps/desktop/src/components/UserSettingsPanel.tsx`:

1. Add props:

```typescript
deviceSessions: DeviceSessionListState;
accountManagement: AccountManagementState;
onQueryDevices: () => void;
onRenameDevice: (deviceOrdinal: number, displayName: string) => void;
onDeleteDevices: (deviceOrdinals: number[]) => void;
onSubmitAccountManagementUia: (flowId: number, password: string) => void;
```

2. Add a new "Sessions" section after the existing Security section.
3. On mount, call `onQueryDevices()` when the session is ready and `deviceSessions.kind === "idle"`.
4. Render:
   - Loading state
   - Failure state with retry
   - Current device
   - Other devices list
   - Rename button that opens an inline field
   - Sign-out button per device
   - "Sign out all other sessions" button (computes non-current ordinals from the snapshot and calls `onDeleteDevices`)
5. When `accountManagement.kind === "awaitingUia"`, render a password prompt and call `onSubmitAccountManagementUia`. Do not persist the password in React state after dispatch.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/UserSettingsPanel.test.tsx
```

Expected: existing tests pass; new tests may fail until Step 8.

### Step 8: Wire UserSettingsPanel in App.tsx

In `apps/desktop/src/App.tsx`:

1. Add async functions:

```typescript
async function queryDevices() {
  setSnapshot(await api.queryDevices());
}

async function renameDevice(deviceOrdinal: number, displayName: string) {
  setSnapshot(await api.renameDevice(deviceOrdinal, displayName));
}

async function deleteDevices(deviceOrdinals: number[]) {
  setSnapshot(await api.deleteDevices(deviceOrdinals));
}

async function submitAccountManagementUia(flowId: number, password: string) {
  setSnapshot(await api.submitAccountManagementUia(flowId, password));
}
```

2. Pass `snapshot.state.device_sessions`, `snapshot.state.account_management`, and the four handlers into `ContextualRightPanel` and then into `UserSettingsPanel`.

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run src/App.test.tsx
```

Expected: passes.

### Step 9: Add component tests

In `apps/desktop/src/components/UserSettingsPanel.test.tsx`, add tests that:

- Seed `deviceSessions` as `loaded` with two devices.
- Click rename and assert `onRenameDevice` is called.
- Click sign-out and assert `onDeleteDevices` is called with the correct ordinal.
- Click "Sign out all other sessions" and assert `onDeleteDevices` is called with all non-current ordinals.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/UserSettingsPanel.test.tsx
```

Expected: passes.

### Step 10: Add browser-headless GUI-operation test

In `apps/desktop/e2e/basic-operations.spec.ts`, add a test that:

- Seeds a ready session with `device_sessions` loaded.
- Opens user settings.
- Clicks the Sessions nav item.
- Clicks rename on the second device, changes the name, submits.
- Asserts the command invocation `rename_device` is recorded.
- Clicks sign-out on the second device.
- Asserts `delete_devices` invocation with the expected ordinal.

Keep assertion output token-only; do not print device IDs or display names.

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "device session manager" --workers=1
```

Expected: passes.

### Step 11: Add CSS tokens for sessions list

In `apps/desktop/src/styles.css`, add minimal layout classes for `.sessions-list`, `.session-row`, `.session-row-current`, `.session-actions`. Follow existing spacing/sizing tokens.

Run:

```bash
npm --prefix apps/desktop run test -- --run
```

Expected: passes.

### Step 12: Verify and comment

Run the focused gate set:

```bash
cargo fmt --check
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo test -p matrix-desktop-state
cargo test -p matrix-desktop-core
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "device session manager" --workers=1
```

Expected: all exit 0.

Then commit and comment:

```bash
git add apps/desktop/src-tauri apps/desktop/src/backend apps/desktop/src/components apps/desktop/src/test apps/desktop/src/App.tsx apps/desktop/src/i18n/messages.ts apps/desktop/src/styles.css apps/desktop/e2e/basic-operations.spec.ts docs/superpowers/plans/2026-06-17-umbrella-12-non-mac-completion.md
git commit -m "feat: #38 Phase B device/session manager GUI"
gh issue comment 38 --body "Phase B GUI wiring landed on main: Tauri commands, React sessions panel, rename/sign-out flows, UIA password prompt, browser-headless and component tests. Verification gate passed; closing if acceptance criteria are satisfied."
```

Expected: commit succeeds; comment posted. Close #38 only if all acceptance criteria are verified.

---

## Task 2: #39 Phase B — Sliding-Sync Room-List Filters And Mark Read/Unread GUI

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `apps/desktop/src/domain/desktopModel.ts` and tests
- Modify: `apps/desktop/src/components/WorkspaceSidebar.tsx` (or the file that renders the room list sidebar)
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/domain/desktopModel.test.ts`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

### Step 1: Add Tauri command wrappers for room-list filter and mark read/unread

In `apps/desktop/src-tauri/src/commands.rs`, add:

```rust
#[tauri::command]
pub async fn select_room_list_filter(
    filter: RoomListFilter,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::SelectRoomListFilter { request_id, filter }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn mark_room_as_read(
    room_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::MarkRoomAsRead { request_id, room_id }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn mark_room_as_unread(
    room_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::MarkRoomAsUnread { request_id, room_id }),
    )
    .await?;
    current_snapshot(state.inner()).await
}
```

`RoomListFilter` is already imported from `matrix_desktop_state`; use the serde DTO enum, not a string.

Run:

```bash
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Expected: compiles.

### Step 2: Register the new Tauri commands

In `apps/desktop/src-tauri/src/lib.rs`, add:

```rust
.select_room_list_filter,
.mark_room_as_read,
.mark_room_as_unread,
```

Run:

```bash
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Expected: compiles.

### Step 3: Add client methods

In `apps/desktop/src/backend/client.ts`, add to `DesktopApi`:

```typescript
selectRoomListFilter(filter: RoomListFilter): Promise<DesktopSnapshot>;
markRoomAsRead(roomId: string): Promise<DesktopSnapshot>;
markRoomAsUnread(roomId: string): Promise<DesktopSnapshot>;
```

Implement in `TauriDesktopApi`:

```typescript
async selectRoomListFilter(filter: RoomListFilter): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("select_room_list_filter", { filter });
}

async markRoomAsRead(roomId: string): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("mark_room_as_read", { roomId });
}

async markRoomAsUnread(roomId: string): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("mark_room_as_unread", { roomId });
}
```

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: type errors in `BrowserFakeApi`.

### Step 4: Implement browser fake filter and mark read/unread

Browser fakes must not compute room-list filters or unread semantics. They record the typed command and return a pre-seeded Rust-shaped snapshot. Tests and harnesses supply the seeded `room_list` snapshot.

In `apps/desktop/src/backend/browserFakeApi.ts`:

```typescript
async selectRoomListFilter(filter: RoomListFilter): Promise<DesktopSnapshot> {
  if (!this.canUseSyncedViews()) {
    return this.getSnapshot();
  }
  this.snapshot.state.room_list.active_filter = filter;
  // Do NOT recompute items. Tests seed room_list.items directly via setRoomListProjection().
  return this.getSnapshot();
}

async markRoomAsRead(roomId: string): Promise<DesktopSnapshot> {
  // Do NOT mutate unread counts. Tests seed the expected Rust-shaped snapshot.
  void roomId;
  return this.getSnapshot();
}

async markRoomAsUnread(roomId: string): Promise<DesktopSnapshot> {
  // Do NOT mutate unread counts. Tests seed the expected Rust-shaped snapshot.
  void roomId;
  return this.getSnapshot();
}

setRoomListProjection(projection: RoomListProjection): void {
  this.snapshot.state.room_list = projection;
}
```

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: passes.

### Step 5: Update mocks and snapshots

In `apps/desktop/src/test/tauriIpcMock.ts` and `appHarnessMain.tsx`, add default `room_list` with a few items and record invocations for the three new commands.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/test
```

Expected: passes.

### Step 6: Make sidebar consume Rust-projected room list

In `apps/desktop/src/domain/desktopModel.ts`, change `roomListSections` so that `room_list.items` is authoritative whenever the `room_list` projection exists. An empty `items` array is a valid Rust projection (for example, the Unread filter with no unread rooms) and must not fall back to rendering unfiltered rooms.

If older code paths or tests require the legacy fallback, introduce an explicit marker such as `room_list.items === null` to mean "projection unavailable". Once the projection contract is present, React/sidebar code must not apply additional filters or derive membership from `rooms`/`invites`.

```typescript
export function roomListSections(
  roomList: RoomListProjection,
  spaces: SpaceSummary[],
  rooms: RoomSummary[],
  invites: InvitePreview[]
): RoomListSections { ... }
```

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/desktopModel.test.ts
```

Expected: passes after adding new tests.

### Step 7: Add filter tab bar to the room-list sidebar

Locate the sidebar component that renders the room list (likely `apps/desktop/src/components/WorkspaceSidebar.tsx` or inline in `App.tsx`). Add a filter tab bar with buttons for Rooms, Unread, People, Favourites, Invites. Each button calls `onSelectRoomListFilter(filter)` and shows active state from `roomList.active_filter`.

If the component does not yet exist as a separate file, create `apps/desktop/src/components/RoomListFilterTabs.tsx`.

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: passes.

### Step 8: Add mark read/unread context menu items

In the room-row context menu builder (likely `apps/desktop/src/domain/contextMenus.ts` or inline in the sidebar), add:

```typescript
{ id: "markRoomAsRead", label: t("room.markAsRead") }
{ id: "markRoomAsUnread", label: t("room.markAsUnread") }
```

Wire the actions in `App.tsx` `runContextMenuAction` to call `api.markRoomAsRead(roomId)` / `api.markRoomAsUnread(roomId)`.

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: passes.

### Step 9: Add i18n labels

In `apps/desktop/src/i18n/messages.ts`:

```typescript
"roomList.filterRooms": "Rooms",
"roomList.filterUnread": "Unread",
"roomList.filterPeople": "People",
"roomList.filterFavourites": "Favourites",
"roomList.filterInvites": "Invites",
"room.markAsRead": "Mark as read",
"room.markAsUnread": "Mark as unread"
```

Run:

```bash
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
```

Expected: passes.

### Step 10: Update desktopModel tests

In `apps/desktop/src/domain/desktopModel.test.ts`, add tests that:

- When `room_list.items` is non-empty, `roomListSections` uses that order and membership.
- When `room_list.items` is empty, the existing fallback still works.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/desktopModel.test.ts
```

Expected: passes.

### Step 11: Add browser-headless GUI-operation test

In `apps/desktop/e2e/basic-operations.spec.ts`, add a test that:

- Seeds a ready session with rooms and invites and a non-empty `room_list.items`.
- Clicks the "Unread" filter tab.
- Asserts `select_room_list_filter` invocation with `filter: { kind: "unread" }`.
- Right-clicks a room row and selects "Mark as unread".
- Asserts `mark_room_as_unread` invocation.

Keep assertion output token-only.

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "room list filter and mark unread" --workers=1
```

Expected: passes.

### Step 12: Add CSS for filter tabs

In `apps/desktop/src/styles.css`, add `.room-list-filter-tabs`, `.room-list-filter-tab`, `.room-list-filter-tab-active`.

Run:

```bash
npm --prefix apps/desktop run test -- --run
```

Expected: passes.

### Step 13: Verify and comment

Run the focused gate set:

```bash
cargo fmt --check
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo test -p matrix-desktop-state
cargo test -p matrix-desktop-core
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "room list filter and mark unread" --workers=1
```

Expected: all exit 0.

Then commit and comment:

```bash
git add apps/desktop/src-tauri apps/desktop/src/backend apps/desktop/src/components apps/desktop/src/domain apps/desktop/src/test apps/desktop/src/App.tsx apps/desktop/src/i18n/messages.ts apps/desktop/src/styles.css apps/desktop/e2e/basic-operations.spec.ts docs/superpowers/plans/2026-06-17-umbrella-12-non-mac-completion.md
git commit -m "feat: #39 Phase B room-list filters and mark read/unread GUI"
gh issue comment 39 --body "Phase B GUI wiring landed on main: Tauri commands, filter tab bar, room-row mark read/unread context menu, browser-headless GUI-operation test, and fake updates. Verification gate passed; closing if acceptance criteria are satisfied."
```

Expected: commit succeeds; comment posted. Close #39 only if all acceptance criteria are verified.

---

## Task 3: #44 — Per-Room Notification Mode + Read-Receipt/Typing Privacy Opt-Out

#44 requires a full Phase A contract before any React work. Do not start Step 10 until Steps 1-9 pass.

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-state/src/effect.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `crates/matrix-desktop-core/src/timeline.rs`
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/tauriIpcMock.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx`
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `crates/matrix-desktop-state/tests/notification_settings_state.rs`
- Test: `crates/matrix-desktop-core/tests/runtime_notification_settings.rs`
- Test: `apps/desktop/src/components/RoomInfoPanel.test.tsx`
- Test: `apps/desktop/src/components/UserSettingsPanel.test.tsx`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

### Step 1: Define the Rust state contract

Add to `crates/matrix-desktop-state/src/state.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomNotificationMode {
    All,
    Mentions,
    Mute,
}

impl Default for RoomNotificationMode {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct RoomNotificationSettings {
    pub mode: RoomNotificationMode,
    pub operation: RoomNotificationModeOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomNotificationModeOperation {
    Idle,
    Pending { request_id: u64 },
    Failed { request_id: u64; kind: OperationFailureKind },
}

impl Default for RoomNotificationModeOperation {
    fn default() -> Self {
        Self::Idle
    }
}
```

Extend the existing `NotificationSettings` (do not redefine it) by adding two fields with `serde(default)` so old persisted JSON deserializes safely:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotificationSettings {
    pub desktop_notifications: bool,
    pub sound: bool,
    pub badges: bool,
    #[serde(default = "default_true")]
    pub send_read_receipts: bool,
    #[serde(default = "default_true")]
    pub send_typing_notifications: bool,
}

fn default_true() -> bool {
    true
}
```

Add `AppState.room_notification_settings: HashMap<String, RoomNotificationSettings>` with `Default` empty.

Update `SettingsPatch` to include the same notification fields.

### Step 2: Add reducer actions and guards

In `crates/matrix-desktop-state/src/action.rs`:

```rust
RoomNotificationModeSet {
    request_id: u64,
    room_id: String,
    mode: RoomNotificationMode,
}
RoomNotificationModeCompleted {
    request_id: u64,
    room_id: String,
}
RoomNotificationModeFailed {
    request_id: u64,
    room_id: String,
    kind: OperationFailureKind,
}
SettingsNotificationPrivacyChanged {
    send_read_receipts: bool,
    send_typing_notifications: bool,
}
```

In `crates/matrix-desktop-state/src/reducer.rs`:

- `RoomNotificationModeSet` updates `state.room_notification_settings[room_id].mode` only if `room_id` is in `state.rooms` or `state.invites` (known-room guard). It also sets `operation` to `Pending { request_id }`, clearing any prior failed state for that room.
- `RoomNotificationModeCompleted` sets `operation` to `Idle` for the matching `(request_id, room_id)`.
- `RoomNotificationModeFailed` records a per-room recoverable failure by setting `operation` to `Failed { request_id, kind }`.
- `SettingsNotificationPrivacyChanged` updates `state.settings.values.notifications`.
- Logout clears `room_notification_settings`.

### Step 3: Write reducer tests

Create `crates/matrix-desktop-state/tests/notification_settings_state.rs`:

```rust
#[test]
fn default_room_notification_mode_is_all() { ... }

#[test]
fn set_room_notification_mode_updates_known_room_and_sets_pending() { ... }

#[test]
fn set_room_notification_mode_ignores_unknown_room() { ... }

#[test]
fn completed_mode_set_clears_pending() { ... }

#[test]
fn failed_mode_set_is_recorded_per_room() { ... }

#[test]
fn privacy_settings_persist_defaults() { ... }

#[test]
fn old_persisted_notification_json_defaults_privacy_to_true() { ... }

#[test]
fn logout_clears_room_notification_settings() { ... }
```

Run:

```bash
cargo test -p matrix-desktop-state --test notification_settings_state
```

Expected: passes after implementation.

### Step 4: Define the Matrix push-rule contract

Document the contract in a new file `docs/superpowers/specs/2026-06-17-room-notification-mode-contract.md`:

Use Matrix push-rule overrides scoped to the room. Rule IDs owned by Kagome are prefixed `org.matrix.desktop.notify.room.<room_id>.` so they can be added and removed idempotently.

- `All`: remove any Kagome-owned override rule for the room. Default global rules continue to apply.
- `Mentions`: add a Kagome-owned **underride** rule at default order, matching the room by `room_id`, with action `"dont_notify"`. Because underride priority is below override/content rules, mention rules (`.m.rule.contains_display_name`, `.m.rule.contains_user_name`, `.m.rule.roomnotif`) and highlight rules still fire. The underride suppresses only the lower-priority generic message rules (`.m.rule.message`, `.m.rule.room_one_to_one`, `.m.rule.encrypted`).
- `Mute`: add a Kagome-owned **override** rule at high priority, matching the room by `room_id`, with action `"dont_notify"`. Override priority suppresses all lower-priority rules for that room, including mentions and highlights.
- On mode change, remove the previous Kagome-owned rule for the room before adding the new one.
- Failures map to `OperationFailureKind` coarse kinds (`network`, `timeout`, `forbidden`, `sdk`).
- Never log raw rule IDs, room IDs, or SDK errors in QA output or Debug output.

### Step 5: Add SDK push-rule wrappers

In `crates/matrix-desktop-sdk/src/lib.rs`, add:

```rust
pub async fn set_room_notification_mode(
    &self,
    room_id: &RoomId,
    mode: RoomNotificationMode,
) -> Result<(), SdkError>;
```

Implementation uses the documented push-rule override IDs. Errors map to `SdkError` coarse kinds only.

Run:

```bash
cargo test -p matrix-desktop-sdk notification
```

Expected: passes.

### Step 6: Add core command and runtime handler

In `crates/matrix-desktop-core/src/command.rs`, add to `RoomCommand`:

```rust
SetRoomNotificationMode {
    request_id: RequestId,
    room_id: String,
    mode: RoomNotificationMode,
}
```

In `crates/matrix-desktop-core/src/runtime.rs`, route to `RoomActor`. The actor:

1. Validates the room is known.
2. Emits `AppAction::RoomNotificationModeSet` optimistically (sets `mode` and `operation: Pending`).
3. Calls SDK wrapper.
4. On success, emits `AppAction::RoomNotificationModeCompleted` (sets `operation: Idle`).
5. On failure, emits `AppAction::RoomNotificationModeFailed` (sets `operation: Failed`).

In `crates/matrix-desktop-core/src/account.rs` and `timeline.rs`, gate `send_read_receipt` and `set_typing` by `state.settings.values.notifications.send_read_receipts` / `send_typing_notifications`. If disabled, skip the SDK call and return success without emitting a live-signal event.

Run:

```bash
cargo test -p matrix-desktop-core notification
```

Expected: passes.

### Step 7: Integrate notification modes into native attention

In `crates/matrix-desktop-core/src/runtime.rs` or the attention projector, suppress notification candidates for rooms whose mode is `Mute`. For `Mentions`, allow only highlight candidates. Keep the candidate private-data-minimized.

Add reducer/core tests proving the integration.

Run:

```bash
cargo test -p matrix-desktop-state --test attention_surface
cargo test -p matrix-desktop-core attention
```

Expected: passes.

### Step 8: Update Tauri DTO and TypeScript types

In `apps/desktop/src-tauri/src/dto.rs`, add `room_notification_settings: HashMap<String, FrontendRoomNotificationSettings>` to `FrontendAppState`.

In `apps/desktop/src/domain/types.ts`, add:

```typescript
export type RoomNotificationMode =
  | { kind: "all" }
  | { kind: "mentions" }
  | { kind: "mute" };

export interface RoomNotificationSettings {
  mode: RoomNotificationMode;
}

export interface NotificationSettings {
  desktop_notifications: boolean;
  sound: boolean;
  badges: boolean;
  send_read_receipts: boolean;
  send_typing_notifications: boolean;
}

export interface AppState {
  ...
  room_notification_settings: Record<string, RoomNotificationSettings>;
}
```

Update `coreEvents.generated.json` if the contract artifact changes; otherwise rely on `StateChanged`.

Update browser fake defaults, app harness snapshots, and `tauriIpcMock.ts` defaults to include `room_notification_settings: {}` and the expanded `notifications` defaults.

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
npm --prefix apps/desktop run typecheck
```

Expected: passes.

### Step 9: Add Tauri commands

In `apps/desktop/src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn set_room_notification_mode(
    room_id: String,
    mode: RoomNotificationMode,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::SetRoomNotificationMode {
            request_id,
            room_id,
            mode,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}
```

Privacy toggles use the existing `update_settings` command with a `SettingsPatch`; do **not** add a separate Tauri command. This keeps settings persistence on the existing Rust path.

Register `set_room_notification_mode` in `lib.rs`.

Run:

```bash
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Expected: compiles.

### Step 10: Add client methods and browser fake

In `apps/desktop/src/backend/client.ts`:

```typescript
setRoomNotificationMode(roomId: string, mode: RoomNotificationMode): Promise<DesktopSnapshot>;
```

Implement in `TauriDesktopApi`:

```typescript
async setRoomNotificationMode(roomId: string, mode: RoomNotificationMode): Promise<DesktopSnapshot> {
  return invoke<DesktopSnapshot>("set_room_notification_mode", { roomId, mode });
}
```

Implement fake in `browserFakeApi.ts`:

```typescript
async setRoomNotificationMode(roomId: string, mode: RoomNotificationMode): Promise<DesktopSnapshot> {
  if (!this.knownRoomOrInvite(roomId)) {
    return this.getSnapshot();
  }
  this.snapshot.state.room_notification_settings[roomId] = { mode, operation: { kind: "idle" } };
  return this.getSnapshot();
}
```

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: passes.

### Step 11: Add i18n labels

In `apps/desktop/src/i18n/messages.ts`:

```typescript
"room.notifications": "Notifications",
"room.notifyModeAll": "All messages",
"room.notifyModeMentions": "Mentions & keywords",
"room.notifyModeMute": "Mute",
"settings.sendReadReceipts": "Send read receipts",
"settings.sendTypingNotifications": "Send typing notifications"
```

Run:

```bash
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
```

Expected: passes.

### Step 12: Render per-room notification menu

In `apps/desktop/src/components/RoomInfoPanel.tsx`, add a "Notifications" section with a select for All / Mentions / Mute. Call `onSetRoomNotificationMode(roomId, mode)`.

Run component tests.

### Step 13: Render privacy toggles in UserSettingsPanel

In `apps/desktop/src/components/UserSettingsPanel.tsx`, add two toggles under the Notifications section:

- "Send read receipts"
- "Send typing notifications"

Call `onUpdateSettings({ notifications: { ...selectedNotifications, send_read_receipts: value, send_typing_notifications: value } })`.

Run component tests.

### Step 14: Wire handlers in App.tsx

Add:

```typescript
async function setRoomNotificationMode(roomId: string, mode: RoomNotificationMode) {
  setSnapshot(await api.setRoomNotificationMode(roomId, mode));
}
```

Pass `onSetRoomNotificationMode` to `RoomInfoPanel`.

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
```

Expected: passes.

### Step 15: Add Rust core tests

Create `crates/matrix-desktop-core/tests/runtime_notification_settings.rs` with tests that:

- `set_room_notification_mode` for a known room emits the expected action.
- `set_room_notification_mode` for an unknown room is ignored.
- `update_settings` privacy patch gates live-signal dispatch.
- Native attention suppresses candidates for muted rooms and allows only highlights for mentions mode.

Run:

```bash
cargo test -p matrix-desktop-core notification
```

Expected: passes.

### Step 16: Add browser-headless GUI-operation test

In `apps/desktop/e2e/basic-operations.spec.ts`, add a test that:

- Opens a room info panel.
- Changes notification mode to "Mute".
- Asserts `set_room_notification_mode` invocation with coarse payload shape.
- Opens user settings, toggles "Send read receipts" off.
- Asserts `update_settings` payload contains `notifications.send_read_receipts: false`.

Keep assertion output token-only.

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "per-room notification mode and privacy" --workers=1
```

Expected: passes.

### Step 17: Verify and comment

Run the focused gate set:

```bash
cargo fmt --check
cargo test -p matrix-desktop-state
cargo test -p matrix-desktop-core
cargo test -p matrix-desktop-sdk
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts -g "per-room notification mode and privacy" --workers=1
```

Expected: all exit 0.

Then commit and comment:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri apps/desktop/src/backend apps/desktop/src/components apps/desktop/src/domain apps/desktop/src/test apps/desktop/src/App.tsx apps/desktop/src/i18n/messages.ts apps/desktop/src/styles.css apps/desktop/e2e/basic-operations.spec.ts docs/superpowers/plans/2026-06-17-umbrella-12-non-mac-completion.md docs/superpowers/specs/2026-06-17-room-notification-mode-contract.md
git commit -m "feat: #44 per-room notification mode and privacy opt-out"
gh issue comment 44 --body "Phase A and Phase B landed on main: Rust-owned per-room notification modes with known-room guard and push-rule contract, account-wide read-receipt/typing privacy toggles via existing settings path, native-attention integration, SDK wrapper, Tauri/TS contracts, React settings UI, and browser-headless GUI-operation tests. Verification gate passed; closing if acceptance criteria are satisfied."
```

Expected: commit succeeds; comment posted. Close #44 only if all acceptance criteria are verified.

---

## Verification

Run the full local gate set after all tasks:

```bash
cargo fmt --check
cargo test --workspace
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop exec -- playwright test e2e/basic-operations.spec.ts --workers=1
```

Expected: all exit 0.

## Umbrella #12 Update

After the child issues close, comment on umbrella #12:

```bash
gh issue comment 12 --body "Completed non-mac child issues #38 Phase B, #39 Phase B, and #44. Mac-hardware-dependent issues #66 and #67 remain. Remaining open feature issues (#37, #40, #45, #47-#52, #56, #57, #59-#61, #49, #48) continue to be tracked and planned separately."
```
