# Today Issue Batch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement issues #173-#179 in one branch, one PR, with search source-of-truth debt fixed instead of patched around.

**Architecture:** Keep Matrix/product semantics in Rust and let React render or route typed DTOs and commands. Split the work into four independently testable batches: small UX fixes, room-info/create-room contracts, sidebar category/sort controls, and search lifecycle/navigation/indexing.

**Tech Stack:** Rust workspace (`koushi-state`, `koushi-core`, `koushi-sdk`, Tauri commands), TypeScript/React/Vitest, Playwright browser-headless checks, GitHub CLI.

---

## Global Constraints

- Work only on branch `codex/today-batch-20260703`.
- Keep all seven issues in one final PR.
- Follow verify-first discipline: write the focused failing test, run it and
  confirm the expected RED, then implement the smallest code to turn it GREEN.
- Do not print private Matrix data in tests, QA output, logs, issue comments, or
  PR evidence.
- If state-machine behavior changes, update `docs/architecture/state-machine.md`
  in the same batch.

---

### Task 1: Media save default Downloads directory (#173)

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands/timeline.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs` if command registration is needed
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/App.tsx`
- Test: `apps/desktop/src-tauri/src/commands/timeline.rs`

- [ ] **Step 1: Write the failing Rust test**

Add a test near `save_downloaded_media_tests` proving a helper returns an
absolute Downloads child path when `download_dir` is available and falls back to
the sanitized filename when it is not.

Expected command:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml default_media_save_path_prefers_downloads_directory
```

Expected RED: helper or command is missing.

- [ ] **Step 2: Implement minimal Rust path helper and command**

Expose a Tauri command such as `default_media_save_path(filename) -> String`
that sanitizes the filename and joins it to `app.path().download_dir()` when
available. Keep `save_downloaded_media` destination validation unchanged.

- [ ] **Step 3: Wire frontend defaultPath**

Add `api.defaultMediaSavePath(filename)` and call it before `saveDialog`, using
the returned string as `defaultPath`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml default_media_save_path_prefers_downloads_directory
npm --prefix apps/desktop run typecheck
```

---

### Task 2: DM context User info and active-member Unban visibility (#174, #175)

**Files:**
- Modify: `apps/desktop/src/domain/contextMenus.ts`
- Modify: `apps/desktop/src/components/Shell.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/PeoplePanel.tsx`
- Test: `apps/desktop/src/domain/contextMenus.test.ts`
- Test: `apps/desktop/src/components/PeoplePanel.test.tsx`
- Test: `apps/desktop/src/App.test.tsx` or existing shell/menu integration test

- [ ] **Step 1: Write failing context-menu test**

Assert a one-to-one DM room menu includes `openUserInfo` and a non-DM room menu
does not.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/contextMenus.test.ts
```

Expected RED: action id does not exist or menu lacks it.

- [ ] **Step 2: Write failing profile-panel test**

Render an active room member with `can_unban: true` and assert the Unban button
is absent while Kick/Ban remain available.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/PeoplePanel.test.tsx
```

Expected RED: Unban is rendered.

- [ ] **Step 3: Implement menu and routing**

Add `openUserInfo` to the context-menu action type and build it only when the
room summary has exactly one `dm_user_ids` target. In App routing, select the
room, load room settings as needed, set the people/profile scope to that room,
set the selected profile user id, and open the profile right panel.

- [ ] **Step 4: Hide active-member Unban**

Remove the active-member Unban button path from `ProfilePanel`. Keep future
banned-member support out of scope until Rust exposes a banned-member profile
model.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/contextMenus.test.ts src/components/PeoplePanel.test.tsx
npm --prefix apps/desktop run typecheck
```

---

### Task 3: Room info badges and share link (#176, #177 partial)

**Files:**
- Modify: `crates/koushi-state/src/state/room_settings.rs` or the existing settings state module
- Modify: `crates/koushi-state/src/reducer/room_settings.rs` if snapshot fields are reduced there
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: focused Rust state/DTO tests for the new room-settings fields
- Test: `apps/desktop/src/components/RoomInfoPanel.test.tsx`

- [ ] **Step 1: Write failing Rust/DTO test for share link**

Assert a settings snapshot with canonical alias projects a `matrix.to` link,
then alternate alias, then room-id fallback.

Run:

```bash
cargo test -p koushi-state room_settings_share_link_prefers_aliases
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml room_settings_share_link_serializes
```

Expected RED: fields are missing.

- [ ] **Step 2: Write failing RoomInfoPanel test**

Render public encrypted settings with history visibility and assert encryption,
public, history, and Copy link controls are present. Render no share link and
assert copy is absent.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/RoomInfoPanel.test.tsx
```

Expected RED: badges/link controls are missing.

- [ ] **Step 3: Implement Rust projection**

Extend the room settings snapshot with canonical alias, alternate aliases, and
share link. Generate the share link in Rust, not React, using URL-encoded
`matrix.to/#/<id-or-alias>`.

- [ ] **Step 4: Wire DTO/types/UI**

Update Tauri DTOs, TypeScript types, generated event contract fixture, room
info badges, i18n strings, and CSS. Copy uses only the Rust-projected link.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test -p koushi-state room_settings_share_link_prefers_aliases
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml room_settings_share_link_serializes
npm --prefix apps/desktop run test -- --run src/components/RoomInfoPanel.test.tsx
npm --prefix apps/desktop run typecheck
```

---

### Task 4: Element-style create-room options (#177 remaining)

**Files:**
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/room.rs`
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/commands/room.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/components/dialogs.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Test: Rust command-builder/core tests for create-room options
- Test: React dialog/API tests for option payloads

- [ ] **Step 1: Write failing command-builder test**

Assert `create_room` accepts typed options for private, public, and active-space
standard rooms, and maps them into `RoomCommand::CreateRoom` options without
dropping topic, alias, encryption, or parent-space data.

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml create_room_command_preserves_room_options
```

Expected RED: old command accepts only `name`.

- [ ] **Step 2: Write failing dialog test**

Assert the create-room dialog shows private/public/standard-in-space choices,
validates public alias localpart, hides encryption for public rooms, and submits
the expected typed payload.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/dialogs.test.tsx
```

Expected RED: controls/payload are missing.

- [ ] **Step 3: Implement Rust command and SDK options**

Add a create-room options struct through Tauri, core, and SDK. Public rooms set
Matrix public visibility/preset/local alias. Standard active-space rooms add
space parent initial state and restricted join rule, and still reconcile with
`set_space_child` after creation.

- [ ] **Step 4: Implement React dialog/API wiring**

Keep draft state in React only until submit. Dispatch typed options and let
Rust/core own resulting room state.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml create_room_command_preserves_room_options
npm --prefix apps/desktop run test -- --run src/components/dialogs.test.tsx
npm --prefix apps/desktop run typecheck
```

---

### Task 5: Sidebar category switcher and sort control (#178)

**Files:**
- Modify: `apps/desktop/src/components/Shell.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/components/Shell.test.tsx`

- [ ] **Step 1: Write failing Shell test**

Assert Home sidebar order contains Activity, Explore, Invites, category
switcher, sort control, and only the selected DMs or Rooms list. Assert clicking
DMs dispatches `select_room_list_filter({kind:"people"})`, clicking Rooms
dispatches `{kind:"rooms"}`, and changing sort dispatches
`update_settings({room_list_sort:"normalLocale"})`.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/Shell.test.tsx
```

Expected RED: controls are absent and both sections render.

- [ ] **Step 2: Implement UI from existing Rust projections**

Render the category list from `snapshot.state.ui.room_list.items`; use
`active_filter` for the selected category and `settings.values.room_list_sort`
for sort. Do not compute alternate product semantics in React.

- [ ] **Step 3: Verify GREEN**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/Shell.test.tsx
npm --prefix apps/desktop run typecheck
```

---

### Task 6: Search lifecycle, scope, navigation, and indexing (#179)

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `crates/koushi-state/src/state/search.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/search.rs`
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/src/search.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-search/src/document.rs`
- Modify: `apps/desktop/src-tauri/src/commands/search.rs`
- Modify: `apps/desktop/src-tauri/src/commands/navigation.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/mediaLists.tsx`
- Modify: `apps/desktop/src/components/panes.tsx`
- Modify: `apps/desktop/src/components/rightPanel.tsx`
- Test: Rust reducer/core/Tauri search tests
- Test: browser-headless or Vitest search panel/navigation tests

- [ ] **Step 1: Write failing reducer tests**

Assert close transitions search to `Closed` and clears query/results; assert
submit moves to `Searching`; assert `CurrentSpace` and `Dms` resolve to a room
set instead of all rooms.

Run:

```bash
cargo test -p koushi-state search_close_clears_results search_space_scope_does_not_fall_back_to_global
```

Expected RED: close action and room-set behavior are missing.

- [ ] **Step 2: Write failing indexing/navigation tests**

Assert initial timeline items are forwarded to search indexing before they are
considered visible, live diff forwarding does not use silent `try_send`, and
`select_search_result` opens the main timeline anchor rather than focused
context.

Run:

```bash
cargo test -p koushi-core search_indexes_initial_timeline_items
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml select_search_result_opens_main_timeline_anchor
```

Expected RED: existing behavior drops or routes incorrectly.

- [ ] **Step 3: Write failing frontend tests**

Assert SearchResults renders searching/failed/results from Rust state, closing
search clears the panel without promoting results into the main timeline, and
result click keeps the search panel while selecting the main timeline room.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/mediaLists.test.tsx src/App.test.tsx
```

Expected RED: local query/right-panel behavior shows synthetic zero or promotes
results.

- [ ] **Step 4: Implement Rust-owned lifecycle and room-set scopes**

Add typed close/edit/search lifecycle actions as needed, update the state-machine
doc, resolve DMs/current-space scopes to explicit room sets, and ensure
`submit_search` waits for a settled matching result or returns `Searching`
instead of a stale pre-result snapshot.

- [ ] **Step 5: Implement reliable indexing and main timeline navigation**

Forward initial timeline items and visible diffs into search with reliable
`send().await` or equivalent queued delivery. Change search result selection to
select the room and open the main timeline anchor, not focused context.

- [ ] **Step 6: Implement frontend rendering/routing**

Drive the panel from `SearchState`, add a typed close command, remove inline
main-pane search result promotion, and bind highlights to the active Rust
search state.

- [ ] **Step 7: Verify GREEN**

Run:

```bash
cargo test -p koushi-state search
cargo test -p koushi-core search
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml search
npm --prefix apps/desktop run test -- --run src/components/mediaLists.test.tsx src/App.test.tsx
npm --prefix apps/desktop run typecheck
```

---

### Task 7: Final gates, PR, CI, merge

- [ ] **Step 1: Run focused gates from all batches**

Run the focused commands listed in Tasks 1-6.

- [ ] **Step 2: Run broader affected gates**

Run:

```bash
cargo test -p koushi-state
cargo test -p koushi-core
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
```

- [ ] **Step 3: Open one PR**

Push `codex/today-batch-20260703` and open one PR referencing #173-#179. Use
`Closes` only for issues fully implemented by the PR.

- [ ] **Step 4: Fix CI until green**

Inspect failing checks with `gh`, patch, and rerun focused verification locally.

- [ ] **Step 5: Merge**

After CI is green and the branch is ready, merge the PR and delete the branch.
Use a non-squash merge when repository policy allows it.

## Self-Review

- Spec coverage: #173 Task 1; #174/#175 Task 2; #176 Task 3; #177 Tasks 3-4;
  #178 Task 5; #179 Task 6.
- Ownership: Rust owns product state, React owns transient presentation only.
- Verification: every batch lists a RED command and a GREEN command.
- No implementation placeholders are required for issue scope decisions; exact
  API names may be adjusted only to match existing local conventions while
  preserving the contracts above.

