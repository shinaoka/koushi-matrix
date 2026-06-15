# Remaining Core Phase A Batch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the remaining umbrella #12 work by batching Rust-owned Phase A state-machine and headless contracts first, then serializing GUI slices, then running the #9/#31 integration gate.

**Architecture:** Feature issues remain the acceptance units, but implementation is grouped by layer. Core Batch A adds Rust state, reducers, commands, actors, DTOs, Tauri contracts, headless QA tokens, and canon updates before React surfaces. GUI Batch B renders Rust-owned state and dispatches typed commands only. QA Batch Z closes the umbrella after feature checks and integration matrix evidence exist.

**Tech Stack:** Rust workspace (`matrix-desktop-state`, `matrix-desktop-core`, `matrix-desktop-sdk`, `matrix-desktop-search`, `matrix-desktop-key`), Tauri v2 adapter, React/TypeScript frontend, Vitest, Playwright browser-headless tests, local Conduit/Tuwunel headless QA, Linux virtual-display GUI QA.

---

## Execution Rules

- This plan is not an implementation approval by itself. Execute it only after
  the user approves this plan as a batch.
- Keep a single durable batch branch/worktree based on current `origin/main`.
- Keep child issues feature-scoped. Comment on each issue when its Phase A or
  Phase B slice lands. Close an issue only when its own acceptance criteria are
  satisfied.
- Do not add `Closes #...` to commits until the relevant issue is genuinely
  complete.
- Rust-owned state machines come first. React code may render only DTO state and
  dispatch typed commands.
- Main agent owns shared files and final review. Subagents may investigate or
  patch module-local code only.

## Shared Serialization Points

Main agent integrates changes to these files:

- `crates/matrix-desktop-state/src/state.rs`
- `crates/matrix-desktop-state/src/action.rs`
- `crates/matrix-desktop-state/src/reducer.rs`
- `crates/matrix-desktop-core/src/command.rs`
- `crates/matrix-desktop-core/src/event.rs`
- `crates/matrix-desktop-core/src/runtime.rs`
- `apps/desktop/src-tauri/src/dto.rs`
- `apps/desktop/src-tauri/src/commands.rs`
- `apps/desktop/src/domain/coreEvents.ts`
- `apps/desktop/src/domain/coreEvents.generated.json`
- `apps/desktop/src/i18n/messages.ts`
- `apps/desktop/e2e/basic-operations.spec.ts`
- `docs/architecture/overview.md`
- `docs/architecture/state-machine.md`
- `docs/policies/engineering-rules.md`
- `AGENTS.md`

GUI Batch B files are serialized and edited by one owner at a time:

- `apps/desktop/src/App.tsx`
- `apps/desktop/src/components/TimelineView.tsx`
- `apps/desktop/src/components/RoomInfoPanel.tsx`
- `apps/desktop/src/components/SpaceInfoPanel.tsx`
- `apps/desktop/src/components/UserSettingsPanel.tsx`
- room-list/sidebar/right-panel code in `apps/desktop/src`
- `apps/desktop/src/styles.css`

## Subagent Lanes

- **Lane 1 - Timeline and composer core:** #19, #18, #32 Stream 3.
- **Lane 2 - Room, discovery, management, activity core:** #20, #21, #23, #22 GUI handoff constraints.
- **Lane 3 - Platform, security, i18n, notification core:** #7, #10, #30, #32 Streams 1-2.
- **Main lane:** shared contracts, canon, DTOs, generated artifacts, verification, issue comments, closure.

Subagent output format:

```markdown
Scope:
- Issues:
- Files inspected:
- Files changed:
- Tests added:
- Verification run:
- Shared contract changes requested:
- Issue comment draft:
```

## Task 1: Batch Branch And Baseline Inventory

**Files:**
- Read: `REPOSITORY_RULES.md`
- Read: `AGENTS.md`
- Read: `docs/superpowers/specs/2026-06-15-remaining-core-phase-a-batch-design.md`
- Create outside repo: `/tmp/matrix-desktop-core-batch-a/issues.json`

- [ ] **Step 1: Fetch and verify clean baseline**

Run:

```bash
git fetch origin
git status --short --branch
git log --oneline -n 5
```

Expected: branch is based on `origin/main`, and unrelated local changes are not
present.

- [ ] **Step 2: Create or reuse a batch branch**

Run:

```bash
git switch -c codex/core-batch-a origin/main
```

If the branch exists, run:

```bash
git switch codex/core-batch-a
git merge --ff-only origin/main
```

Expected: batch branch is current with `origin/main`.

- [ ] **Step 3: Capture GitHub issue inventory outside the repo**

Run:

```bash
mkdir -p /tmp/matrix-desktop-core-batch-a
gh issue list --state open --limit 200 \
  --json number,title,body,labels,assignees,milestone,createdAt,updatedAt,url \
  > /tmp/matrix-desktop-core-batch-a/issues.json
for n in 7 9 10 11 12 18 19 20 21 22 23 30 31 32; do
  gh issue view "$n" --json number,title,body,comments,labels,state,url,updatedAt \
    > "/tmp/matrix-desktop-core-batch-a/issue-$n.json"
done
```

Expected: issue cache exists under `/tmp` only.

- [ ] **Step 4: Comment on #12 that implementation is starting after approval**

Run:

```bash
gh issue comment 12 --body "Core Batch A implementation is starting from the approved plan on branch \`codex/core-batch-a\`. Child issues remain feature-scoped; Phase A evidence will be reported per issue before GUI Batch B begins."
```

Expected: #12 has the start marker.

## Task 2: Core Batch A0 Contract Allocation And Canon

**Files:**
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/architecture/i18n.md`
- Modify: `docs/policies/engineering-rules.md`
- Modify: `docs/qa/headless-basic-operations.md`
- Modify: `docs/upstream/matrix-rust-sdk-feedback.md` only if the decision text mentions rejected vendored SDK patch rationale

- [ ] **Step 1: Add canon text for restore, health, notification, and CJK decisions**

Record these decisions:

```text
#30: MVP key-backup restore means recovery secret import plus currently joined room-key hydration. Do not claim exhaustive backup-wide restore.
#7: credential-store health is Rust-owned coarse state: unknown, healthy, unavailable, locked_or_inaccessible, missing_credential, reset_required.
#10: native attention decisions are Rust-owned candidates with private-data-minimized fields and platform capability data.
#32: Japanese catalog completeness, CJK normalization, CJK collation, and IME send-vs-commit semantics are Rust-owned contracts.
```

- [ ] **Step 2: Add state-machine sections**

In `docs/architecture/state-machine.md`, add maintained sections for:

```text
Local Encryption Health
Native Attention
Japanese/CJK Display And Search
Backup Restore Scope
```

Each section must define states, accepted actions/events, stale-input behavior,
failure behavior, and logout/account-switch clearing behavior.

- [ ] **Step 3: Run docs verification**

Run:

```bash
git diff --check
rg -n "T[B]D|T[O]DO|F[I]XME" docs/architecture docs/policies docs/qa docs/upstream
```

Expected: `git diff --check` exits 0. The `rg` command exits 1 because no
placeholder strings are found.

- [ ] **Step 4: Commit A0 canon**

Run:

```bash
git add docs/architecture docs/policies docs/qa docs/upstream
git commit -m "Document core batch A0 contract decisions"
```

Expected: commit succeeds. If `docs/upstream/matrix-rust-sdk-feedback.md` was
not changed, omit it from `git add`.

## Task 3: Shared State And Command Skeleton Integration

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Test: `crates/matrix-desktop-state/tests/core_batch_a_state.rs`
- Test: `apps/desktop/src-tauri/src/dto.rs`

- [ ] **Step 1: Add failing reducer snapshot test**

Create `crates/matrix-desktop-state/tests/core_batch_a_state.rs` with tests
that construct `AppState::default()` and assert default empty/idle state for:

```rust
assert_eq!(state.room_interactions.len(), 0);
assert_eq!(state.activity.kind(), "closed");
assert_eq!(state.local_encryption.kind(), "unknown");
assert_eq!(state.native_attention.kind(), "idle");
```

Expected now: test fails because the fields do not exist.

- [ ] **Step 2: Run failing test**

Run:

```bash
cargo test -p matrix-desktop-state --test core_batch_a_state
```

Expected: compile failure naming missing fields.

- [ ] **Step 3: Add serializable state fields**

Add these state units to `state.rs` with `Clone`, `Debug`, `Eq`,
`PartialEq`, `Serialize`, `Deserialize`, and `Default` where applicable:

```rust
pub struct RoomInteractionState { /* pinned events and pin op state */ }
pub struct ActivityState { /* closed/opening/open plus active tab */ }
pub struct DirectoryState { /* closed/querying/results/joining/failure */ }
pub struct RoomManagementState { /* selected room settings and operations */ }
pub struct LocalEncryptionState { /* unknown/healthy/unavailable/missing */ }
pub struct NativeAttentionState { /* summaries, candidate, capabilities */ }
pub struct CjkTextPolicyState { /* normalization and collation metadata */ }
```

Add fields to `AppState` with empty/idle defaults.

- [ ] **Step 4: Add placeholder-free action variants**

Add `AppAction` variants for reducer-owned transitions:

```rust
RoomPinnedEventsUpdated { room_id: String, pinned: Vec<PinnedEvent> }
PinEventRequested { request_id: u64, room_id: String, event_id: String }
PinEventCompleted { request_id: u64, room_id: String }
PinEventFailed { request_id: u64, room_id: String, kind: OperationFailureKind }
DirectoryQueryRequested { request_id: u64, query: DirectoryQuery }
ActivityOpened { request_id: u64 }
LocalEncryptionHealthChanged { health: LocalEncryptionHealth }
NativeAttentionUpdated { summary: NativeAttentionSummary }
JapaneseCatalogProfileChanged { profile: JapaneseCatalogProfile }
```

Use existing local naming conventions. If an existing failure enum already
matches the need, reuse it instead of creating `OperationFailureKind`.

- [ ] **Step 5: Add CoreCommand/CoreEvent skeletons**

Add request-correlated command and event variants for the Phase A surfaces.
Every new command must be included in `CoreCommand::request_id()` and
`requires_ready_session()` when appropriate. Every secret or message-bearing
field must have redacted `Debug`.

- [ ] **Step 6: Update DTO and TS wire contracts**

Update `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/coreEvents.ts`,
browser fake snapshots, and the checked-in generated contract artifact.

Run the Rust contract artifact test that regenerates or verifies
`coreEvents.generated.json` according to the repository's existing workflow.

- [ ] **Step 7: Run focused contract checks**

Run:

```bash
cargo test -p matrix-desktop-state --test core_batch_a_state
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
npm --prefix apps/desktop run typecheck
```

Expected: all commands exit 0.

- [ ] **Step 8: Commit shared skeleton**

Run:

```bash
git add crates/matrix-desktop-state apps/desktop/src-tauri apps/desktop/src/domain apps/desktop/src/test
git commit -m "Add core batch A shared state contracts"
```

Expected: commit succeeds.

## Task 4: Core Batch A1 - #19 Reply Quotes And Pinned Events

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-core/src/timeline.rs`
- Modify: `crates/matrix-desktop-core/src/room.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Test: `crates/matrix-desktop-state/tests/message_interactions_state.rs`
- Test: `crates/matrix-desktop-core/src/tests.rs`

- [ ] **Step 1: Write failing state tests**

Create `message_interactions_state.rs` with tests for:

```text
pin request enters Pending when session is Ready
second pin in same room is ignored while Pending
stale pin completion is ignored
pinned state update replaces room pinned list
logout clears room_interactions
reply_quote is absent for non-reply timeline items
```

Run:

```bash
cargo test -p matrix-desktop-state --test message_interactions_state
```

Expected: compile failure or failing assertions before implementation.

- [ ] **Step 2: Implement reducer state**

Implement `ReplyQuote`, `ReplyQuoteState`, `PinnedEvent`,
`PinOperationState`, and `RoomInteractionState` per
`docs/superpowers/specs/2026-06-15-message-interactions-phase-a1-design.md`.

- [ ] **Step 3: Add SDK wrappers**

In `matrix-desktop-sdk`, add wrappers around public SDK pin/unpin support. Keep
the wrapper narrow:

```rust
pub async fn pin_event(&self, room_id: &str, event_id: &str) -> Result<(), SdkError>;
pub async fn unpin_event(&self, room_id: &str, event_id: &str) -> Result<(), SdkError>;
```

Use existing error mapping patterns and do not expose raw SDK errors across
core boundaries.

- [ ] **Step 4: Project reply quotes in timeline actor**

Resolve reply quote previews from available timeline context and SDK reply
details. For unavailable targets, emit `ReplyQuoteState::Missing`; for known
state/non-message targets, emit `Unsupported`; for redacted targets, emit
`Redacted`.

- [ ] **Step 5: Route pin/unpin through RoomActor**

Add `RoomCommand::PinEvent` and `RoomCommand::UnpinEvent`, route them through
`RoomActor`, and emit reducer actions only from Rust outcomes or room-state
projection.

- [ ] **Step 6: Extend headless QA**

Add a `message_interactions` or equivalent stage in `headless-core-qa.rs` that
prints exactly these private-data-free tokens:

```text
reply_quote=ok
pin_event=ok
pinned_state=ok
unpin_event=ok
```

- [ ] **Step 7: Run focused checks**

Run:

```bash
cargo test -p matrix-desktop-state --test message_interactions_state
cargo test -p matrix-desktop-sdk pin
cargo test -p matrix-desktop-core message_interactions
cargo test -p matrix-desktop-core --features qa-bin --bin headless-core-qa
```

Expected: tests exit 0. If the local homeserver binaries are available, also
run the matching `qa:headless-local` scenario.

- [ ] **Step 8: Commit and comment**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri apps/desktop/src/domain docs AGENTS.md
git commit -m "Implement message interactions core phase A1"
gh issue comment 19 --body "Phase A1 core implementation landed on branch \`codex/core-batch-a\`: reply quote projection, Rust-owned pinned-event state, pin/unpin command path, and private-data-free headless tokens. #19 remains open for Phase B GUI and deferred message-interaction slices."
```

## Task 5: Core Batch A1 - #18 Composer Semantics And #32 IME Contract

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-state/src/composer_shortcuts.rs`
- Modify: `crates/matrix-desktop-core/src/timeline.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `apps/desktop/src/domain/composerKeyEvents.ts`
- Test: `crates/matrix-desktop-state/tests/composer_semantics_state.rs`
- Test: `crates/matrix-desktop-state/tests/composer_shortcut_resolver.rs`
- Test: `apps/desktop/src/domain/composerKeyEvents.test.ts`

- [ ] **Step 1: Write failing composer tests**

Create Rust tests for:

```text
is_composing true returns CommitImeCandidate and never Send
main/thread/edit composers use the same resolver input model
mention intent produces structured mention candidate state
markdown send request keeps plain body plus formatted body
unknown slash command returns structured local failure
```

Run:

```bash
cargo test -p matrix-desktop-state composer
```

Expected: failing tests before implementation.

- [ ] **Step 2: Add composer intent DTOs**

Add serializable DTOs for:

```rust
pub struct ComposerKeyFacts { pub key: String, pub is_composing: bool, /* modifiers and selection */ }
pub enum ComposerResolvedAction { Send, InsertNewline, Cancel, CommitImeCandidate, Noop }
pub struct MentionIntent { /* target user/room ids and display labels */ }
pub struct FormattedMessageDraft { /* plain body plus safe formatted form */ }
pub enum SlashCommandIntent { Join, Invite, Me, PlainText, Unsupported }
```

Use current resolver patterns in `composer_shortcuts.rs` and existing
`SettingsState.keyboard`.

- [ ] **Step 3: Implement Rust-owned send payload construction**

Build `TimelineCommand` request payloads so final `m.mentions`, markdown or
formatted body, and slash-command dispatch are derived in Rust/core. React may
send typed draft facts and selection facts only.

- [ ] **Step 4: Keep frontend key handling as a typed client**

Update `composerKeyEvents.ts` to pass `isComposing` for main, thread, edit, and
autocomplete contexts. The TypeScript resolver must not add a fallback send path
when the Rust-owned resolver returns no-op.

- [ ] **Step 5: Extend headless QA tokens**

Add private-data-free tokens:

```text
mention_send=ok
markdown_send=ok
slash_command=ok
ime_guard=ok
```

- [ ] **Step 6: Run focused checks**

Run:

```bash
cargo test -p matrix-desktop-state composer
cargo test -p matrix-desktop-core composer
npm --prefix apps/desktop run test -- --run src/domain/composerKeyEvents.test.ts
npm --prefix apps/desktop run typecheck
```

Expected: all commands exit 0.

- [ ] **Step 7: Commit and comment**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src/domain apps/desktop/src-tauri docs AGENTS.md
git commit -m "Implement composer core semantics"
gh issue comment 18 --body "Phase A core work landed on branch \`codex/core-batch-a\`: Rust-owned mention intent, markdown/formatted send semantics, slash-command parsing, and shared IME-aware composer resolver. #18 remains open for GUI autocomplete, toolbar, pill rendering, and browser-headless checks."
gh issue comment 32 --body "Core IME contract slice landed on branch \`codex/core-batch-a\`: main/thread/edit/autocomplete composer key facts now preserve composing-enter as Rust-owned commit/no-send behavior. #32 remains open for Japanese catalog, CJK normalization/collation, and GUI checks."
```

## Task 6: Core Batch A2 - #20 Public Directory And Explore Core

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-core/src/room.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Test: `crates/matrix-desktop-state/tests/directory_state.rs`

- [ ] **Step 1: Write failing directory state tests**

Test these transitions:

```text
Closed -> Querying on DirectoryQueryRequested
Querying -> Results on DirectoryQuerySucceeded with matching request id
Querying -> Failed on DirectoryQueryFailed with matching request id
stale query completion ignored
join by alias enters Joining and settles from Rust room event
logout clears directory state
```

Run:

```bash
cargo test -p matrix-desktop-state --test directory_state
```

Expected: failing tests before implementation.

- [ ] **Step 2: Add public directory DTOs and commands**

Use app-owned DTOs:

```rust
pub struct PublicRoomDirectoryResult { /* name/topic/avatar flags/member count/canonical alias */ }
pub enum DirectoryQueryState { Closed, Querying, Results, Failed }
pub enum DirectoryJoinState { Idle, Joining, Failed }
```

Do not expose Matrix room IDs or raw SDK errors in QA tokens or Debug output.

- [ ] **Step 3: Add SDK public room wrappers**

Wrap public room directory query, pagination token, and join-by-alias/server
logic. Reject bare room-id joins from directory flow unless the Matrix spec path
requires via hints and those hints are present.

- [ ] **Step 4: Add headless QA**

Add tokens:

```text
directory_query=ok
directory_join=ok
```

- [ ] **Step 5: Run focused checks**

Run:

```bash
cargo test -p matrix-desktop-state --test directory_state
cargo test -p matrix-desktop-sdk directory
cargo test -p matrix-desktop-core directory
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit and comment**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri apps/desktop/src/domain docs AGENTS.md
git commit -m "Implement public directory core contract"
gh issue comment 20 --body "Phase A core directory contract landed on branch \`codex/core-batch-a\`: Rust-owned directory query/results/join state, SDK wrapper, typed command/event path, and private-data-free headless tokens. #20 remains open for Explore GUI and browser-headless operation checks."
```

## Task 7: Core Batch A2 - #21 Room Settings And Moderation Core

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-core/src/room.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Test: `crates/matrix-desktop-state/tests/room_management_state.rs`

- [ ] **Step 1: Write failing permission and settings tests**

Test:

```text
room settings snapshot replaces existing room settings state
topic/name/avatar update records Pending with request id
matching completion clears Pending
stale completion is ignored
moderation command rejected when permission facts disallow it
logout clears room management state
```

Run:

```bash
cargo test -p matrix-desktop-state --test room_management_state
```

Expected: failing tests before implementation.

- [ ] **Step 2: Add app-owned room management DTOs**

Add DTOs for settings, power levels, permission facts, and operation state:

```rust
pub struct RoomSettingsSnapshot { /* name/topic/avatar/join/history visibility */ }
pub struct RoomPermissionFacts { /* can_edit_settings/can_kick/can_ban/can_unban */ }
pub enum RoomManagementOperation { Idle, Pending, Failed }
```

- [ ] **Step 3: Add guarded room commands**

Add commands for setting name/topic/avatar/join rule/history visibility and
kick/ban/unban. Commands must be guarded in Rust using permission facts and
SDK result mapping.

- [ ] **Step 4: Add SDK wrappers**

Add narrow wrappers for the SDK room settings and moderation calls. Map raw SDK
errors into private-data-free failure kinds.

- [ ] **Step 5: Add headless QA**

Add tokens:

```text
room_settings=ok
moderation=ok
permission_guard=ok
```

- [ ] **Step 6: Run focused checks**

Run:

```bash
cargo test -p matrix-desktop-state --test room_management_state
cargo test -p matrix-desktop-sdk room
cargo test -p matrix-desktop-core room_management
```

Expected: all commands exit 0.

- [ ] **Step 7: Commit and comment**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri apps/desktop/src/domain docs AGENTS.md
git commit -m "Implement room management core contract"
gh issue comment 21 --body "Phase A room management core landed on branch \`codex/core-batch-a\`: Rust-owned settings snapshots, permission facts, guarded mutation/moderation commands, and private-data-free headless tokens. #21 remains open for GUI panels and browser-headless operation checks."
```

## Task 8: Core Batch A2 - #23 Activity Recent And Unread Core

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-core/src/room.rs`
- Modify: `crates/matrix-desktop-core/src/timeline.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-core/src/bin/headless-core-qa.rs`
- Test: `crates/matrix-desktop-state/tests/activity_state.rs`

- [ ] **Step 1: Write failing activity state tests**

Test:

```text
Closed -> Opening -> Open for Activity view
Recent rows sorted newest first
Unread rows use fully-read boundary and include stale unread rooms
viewing Unread does not clear unread
jump/mark-read clears only through Rust action
muted or low-priority rooms are excluded
pagination preserves separate Recent and Unread bounds
```

Run:

```bash
cargo test -p matrix-desktop-state --test activity_state
```

Expected: failing tests before implementation.

- [ ] **Step 2: Add Activity state and row DTOs**

Add:

```rust
pub enum ActivityTab { Recent, Unread }
pub enum ActivityState { Closed, Opening, Open { active_tab: ActivityTab, recent: ActivityStream, unread: ActivityStream } }
pub struct ActivityRow { /* room label, sender label, preview, timestamp, event reference token */ }
```

Store event references in a way that can drive `OpenFocusedContext` without
printing event IDs in QA output.

- [ ] **Step 3: Add core projection and mark-read commands**

Project account-wide rows from Rust-owned timeline/live-signal data. Reuse
fully-read state from #16 and room tag data from #22. Add commands:

```text
OpenActivityView
CloseActivityView
SetActivityTab
PaginateActivity
MarkActivityRead
MarkAllActivityRead
```

- [ ] **Step 4: Add headless QA**

Add tokens:

```text
activity_recent=ok
activity_unread=ok
activity_markread=ok
```

Include a stale-unread room in the scenario.

- [ ] **Step 5: Run focused checks**

Run:

```bash
cargo test -p matrix-desktop-state --test activity_state
cargo test -p matrix-desktop-core activity
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit and comment**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core apps/desktop/src-tauri apps/desktop/src/domain docs AGENTS.md
git commit -m "Implement activity view core contract"
gh issue comment 23 --body "Phase A Activity core landed on branch \`codex/core-batch-a\`: Rust-owned Recent/Unread projection, separate bounds, focused-context jump contract, mark-read commands, and private-data-free headless tokens. #23 remains open for GUI tabs and browser-headless operation checks."
```

## Task 9: Core Batch A3 - #7 Credential-Store Health

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-core/src/store.rs`
- Modify: `crates/matrix-desktop-core/src/account.rs`
- Modify: `crates/matrix-desktop-core/src/failure.rs`
- Modify: `crates/matrix-desktop-key/src/lib.rs`
- Test: `crates/matrix-desktop-state/tests/local_encryption_state.rs`
- Test: `crates/matrix-desktop-key/tests/key_management.rs`

- [ ] **Step 1: Write failing health state tests**

Test:

```text
default health is Unknown
StoreActor healthy signal sets Healthy
missing credential sets MissingCredential
locked or inaccessible store sets LockedOrInaccessible
release unavailable state is fail-closed
logout clears account-specific health details
```

Run:

```bash
cargo test -p matrix-desktop-state --test local_encryption_state
```

Expected: failing tests before implementation.

- [ ] **Step 2: Add health DTO and reducer actions**

Use private-data-free coarse states:

```rust
pub enum LocalEncryptionHealthKind {
    Unknown,
    Healthy,
    Unavailable,
    LockedOrInaccessible,
    MissingCredential,
    ResetRequired,
}
```

The snapshot must not include raw OS errors, local paths, account identifiers,
tokens, store keys, or recovery material.

- [ ] **Step 3: Feed health from StoreActor**

Map existing store outcomes into health actions. Keep debug/test file store
behind the existing debug/test-only configuration. Release builds must ignore
file credential env variables.

- [ ] **Step 4: Run focused checks**

Run:

```bash
cargo test -p matrix-desktop-state --test local_encryption_state
cargo test -p matrix-desktop-key
cargo test -p matrix-desktop-core store
node scripts/desktop-release-gate-check.mjs --no-compile
```

Expected: all commands exit 0.

- [ ] **Step 5: Commit and comment**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-key apps/desktop/src-tauri apps/desktop/src/domain docs AGENTS.md scripts
git commit -m "Implement credential-store health state"
gh issue comment 7 --body "Phase A credential-store health landed on branch \`codex/core-batch-a\`: Rust-owned local-encryption status, StoreActor-fed coarse failure kinds, fail-closed release behavior evidence, and private-data-free snapshot contract. #7 remains open for Settings/Security GUI and native prompt/status smoke."
```

## Task 10: Core Batch A3 - #10 Native Attention And Notification Core

**Files:**
- Modify: `crates/matrix-desktop-state/src/state.rs`
- Modify: `crates/matrix-desktop-state/src/action.rs`
- Modify: `crates/matrix-desktop-state/src/reducer.rs`
- Modify: `crates/matrix-desktop-core/src/runtime.rs`
- Modify: `crates/matrix-desktop-core/src/event.rs`
- Modify: `crates/matrix-desktop-core/src/command.rs`
- Modify: `crates/matrix-desktop-sdk/src/lib.rs`
- Test: `crates/matrix-desktop-state/tests/attention_surface.rs`
- Test: `crates/matrix-desktop-sdk/tests/attention_surface.rs`

- [ ] **Step 1: Extend failing attention tests**

Extend existing attention tests to cover:

```text
initial sync/backfill suppressed
self message suppressed
focused room suppressed
highlight beats DM beats message
low-priority/muted rooms excluded
badge clears at zero
native adapter capability unavailable does not corrupt Matrix state
candidate dedupe prevents duplicate notifications
```

Run:

```bash
cargo test -p matrix-desktop-state --test attention_surface
cargo test -p matrix-desktop-sdk --test attention_surface
```

Expected: failing tests for missing states.

- [ ] **Step 2: Add native attention DTOs**

Add:

```rust
pub enum AttentionKind { None, Message, Dm, Mention }
pub struct RoomAttentionSummary { /* counts and safe room display label */ }
pub struct NativeAttentionSummary { /* badge count, highest kind, candidate */ }
pub struct NativeAttentionCapabilities { /* toast/badge/tray/sound/activation support */ }
```

No message body, sender ID, room ID, event ID, transaction ID, token, or raw SDK
error may appear in normal Debug, QA tokens, or snapshots beyond already
allowed visible room labels.

- [ ] **Step 3: Project attention from Rust state**

Feed unread/highlight counts, DM classification, thread counts, fully-read
clears, room tags, and credential/platform capability state into Rust-owned
attention summaries.

- [ ] **Step 4: Add headless tokens**

Add tokens:

```text
notification_candidate=ok
badge_state=ok
suppress_focus=ok
clear_badge=ok
```

- [ ] **Step 5: Run focused checks**

Run:

```bash
cargo test -p matrix-desktop-state --test attention_surface
cargo test -p matrix-desktop-sdk --test attention_surface
cargo test -p matrix-desktop-core attention
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit and comment**

Run:

```bash
git add crates/matrix-desktop-state crates/matrix-desktop-core crates/matrix-desktop-sdk apps/desktop/src-tauri apps/desktop/src/domain docs AGENTS.md
git commit -m "Implement native attention core state"
gh issue comment 10 --body "Phase A native attention core landed on branch \`codex/core-batch-a\`: Rust-owned notification candidates, badge/attention summaries, suppression/dedupe policy, capability DTOs, and private-data-free headless tokens. #10 remains open for GUI/native adapter smoke."
```

## Task 11: Core Batch A3 - #30 Backup Restore Decision

**Files:**
- Modify: `docs/architecture/overview.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/qa/headless-basic-operations.md`
- Modify: `docs/upstream/matrix-rust-sdk-feedback.md`
- Modify: `crates/matrix-desktop-sdk/src/lib.rs` only if wording or summaries need to expose joined-room semantics more clearly
- Test: `crates/matrix-desktop-state/tests/e2ee_trust_state.rs`

- [ ] **Step 1: Add restore-scope assertions**

Extend tests so `KeyBackupStatus::Restoring { total_rooms: None }` and restore
summary wording never claim exhaustive backup-wide restore.

Run:

```bash
cargo test -p matrix-desktop-state --test e2ee_trust_state
```

Expected: failing assertion before wording/state update if existing semantics
claim too much.

- [ ] **Step 2: Document MVP semantics**

Record:

```text
RestoreKeyBackup imports/recover secrets, waits for enabled/downloading state, and hydrates currently joined rooms through public SDK room-scoped APIs. It is not exhaustive backup-wide restore.
```

Add rejected-option rationale: no vendored SDK patch for convenience.

- [ ] **Step 3: Run checks**

Run:

```bash
cargo test -p matrix-desktop-state --test e2ee_trust_state
cargo test -p matrix-desktop-sdk e2ee
cargo test -p matrix-desktop-core e2ee
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 4: Commit and close #30 if criteria are satisfied**

Run:

```bash
git add docs/architecture docs/qa docs/upstream crates/matrix-desktop-state crates/matrix-desktop-sdk crates/matrix-desktop-core
git commit -m "Document backup restore scope decision"
gh issue comment 30 --body "Decision landed on branch \`codex/core-batch-a\`: MVP restore scope is recovery secret import plus joined-room key hydration using public SDK APIs. Exhaustive backup-wide restore is not claimed, and no vendored SDK accessor is added. Verification is recorded in the commit."
```

Close #30 only after the committed docs and tests satisfy all acceptance items:

```bash
gh issue close 30 --comment "Closed by backup restore scope decision: joined-room hydration semantics are documented and verified; exhaustive backup-wide restore remains out of MVP scope without an upstream/public SDK API."
```

## Task 12: Core Batch A3 - #32 Japanese Catalog And CJK Core

**Files:**
- Modify: `docs/architecture/i18n.md`
- Modify: `crates/matrix-desktop-state/src/locale_profile.rs`
- Modify: `crates/matrix-desktop-search/src/verify.rs`
- Modify: `crates/matrix-desktop-search/src/document.rs`
- Modify: `crates/matrix-desktop-core/src/search.rs`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Test: `crates/matrix-desktop-state/tests/locale_display_profile.rs`
- Test: `crates/matrix-desktop-search/tests/search_adapter.rs`
- Test: `apps/desktop/src/i18n/messages.test.ts`

- [ ] **Step 1: Write failing Japanese catalog tests**

Change `messages.test.ts` so representative `ja` messages must not equal the
English fallback except for an explicit allowlist of proper nouns and symbols.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
```

Expected: failure because `ja` currently copies `en`.

- [ ] **Step 2: Populate Japanese catalog**

Replace `const ja: Catalog = { ...en }` with complete Japanese strings for
all shipped `MessageId` values. Keep intentionally identical values in a named
allowlist used by the test.

- [ ] **Step 3: Add CJK normalization and collation tests**

Add tests for:

```text
full-width query matches half-width indexed text
half-width query matches full-width indexed text
room/person sort keys are deterministic for kana/kanji/Latin mixed names
```

Run:

```bash
cargo test -p matrix-desktop-search --test search_adapter
cargo test -p matrix-desktop-state --test locale_display_profile
```

Expected: failing tests before implementation.

- [ ] **Step 4: Implement Rust-owned CJK policy**

Add normalization and sort-key helpers in Rust-owned modules. GUI code may
consume sort order and display profile only; it must not normalize or collate
locally.

- [ ] **Step 5: Run focused checks**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/i18n/messages.test.ts
cargo test -p matrix-desktop-search --test search_adapter
cargo test -p matrix-desktop-state --test locale_display_profile
npm --prefix apps/desktop run typecheck
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit and comment**

Run:

```bash
git add docs/architecture crates/matrix-desktop-state crates/matrix-desktop-search crates/matrix-desktop-core apps/desktop/src/i18n apps/desktop/src/domain docs AGENTS.md
git commit -m "Implement Japanese and CJK core contracts"
gh issue comment 32 --body "Phase A Japanese/CJK core landed on branch \`codex/core-batch-a\`: real Japanese catalog gate, Rust-owned CJK normalization/collation policy, and search/display matching evidence. #32 remains open for GUI line-breaking/truncation and Linux virtual-display checks."
```

## Task 13: Core Batch A Integration Gate

**Files:**
- Modify: issue comments only unless tests expose a defect
- Read: `docs/superpowers/specs/2026-06-15-remaining-core-phase-a-batch-design.md`
- Read: `docs/architecture/state-machine.md`

- [ ] **Step 1: Run core and contract gates**

Run:

```bash
cargo fmt --check
cargo test -p matrix-desktop-state -p matrix-desktop-sdk -p matrix-desktop-core -p matrix-desktop-search -p matrix-desktop-key
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo check --target wasm32-unknown-unknown -p matrix-desktop-state -p matrix-desktop-search
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop run test:ipc-contract
npm --prefix apps/desktop run qa:secret-scan
node scripts/desktop-release-gate-check.mjs --no-compile
```

Expected: all commands exit 0.

- [ ] **Step 2: Run local homeserver QA when tools exist**

First check tools:

```bash
node scripts/desktop-headless-local-qa.mjs --check-tools
```

If tools are available, run:

```bash
npm --prefix apps/desktop run qa:headless-local -- --server=both --core --core-backend=both --timeout-ms=240000
```

Expected: command exits 0 and prints only private-data-free tokens.

If tools are missing, record the exact missing-tool output in #12 and do not
claim local homeserver QA passed.

- [ ] **Step 3: Issue comments for Phase A summary**

Comment on #12 with:

```bash
head_sha="$(git rev-parse --short HEAD)"
gh issue comment 12 --body "Core Batch A completed on branch \`codex/core-batch-a\` through commit \`$head_sha\`.

Included Phase A slices: #19, #18, #20, #21, #23, #7, #10, #30, #32.

Verification evidence is recorded in the implementation comments for each child issue and in the command output from the Core Batch A integration gate.

Remaining: GUI Batch B and QA Batch Z."
```

Do not close feature issues except #30 if Task 11 closed it.

- [ ] **Step 4: Merge or push branch according to repo flow**

Run:

```bash
git status --short --branch
git push origin codex/core-batch-a
```

Expected: branch is pushed. Open or update a draft PR if the user requests PR
workflow. If the user requested direct merge to `origin/main`, merge with a
non-squash method after approval and green gates.

## Task 14: GUI Batch B1 - #19 Reply Quotes And Pins

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/domain/timelineStore.ts`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/App.test.tsx`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] **Step 1: Write failing GUI tests**

Add tests asserting:

```text
reply quote block renders from TimelineItem.reply_quote
pin click invokes pin_event typed command
unpin click invokes unpin_event typed command
pinned banner/list updates only after Rust-shaped snapshot/event
```

Run:

```bash
npm --prefix apps/desktop run test -- --run src/App.test.tsx
npx --prefix apps/desktop playwright test e2e/basic-operations.spec.ts --grep "reply quote|pin"
```

Expected: tests fail before GUI implementation.

- [ ] **Step 2: Implement rendering and typed dispatch**

Render quote/pin data from DTOs only. Do not infer quote source, pin state, or
operation success from DOM state.

- [ ] **Step 3: Run focused checks**

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run src/App.test.tsx src/i18n/messages.test.ts
npx --prefix apps/desktop playwright test e2e/basic-operations.spec.ts --grep "reply quote|pin" --workers=1
```

Expected: all commands exit 0.

- [ ] **Step 4: Commit and comment**

Run:

```bash
git add apps/desktop docs AGENTS.md
git commit -m "Render reply quotes and pinned events"
gh issue comment 19 --body "Phase B slice for reply quotes and pins landed: GUI renders Rust-owned quote/pin state and dispatches typed pin/unpin commands. #19 remains open for deferred message actions/link previews unless those slices are also complete."
```

## Task 15: GUI Batch B2 - #22 Room Tags And #11 Section IA

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: room-list/sidebar components under `apps/desktop/src`
- Modify: `apps/desktop/src/domain/desktopModel.ts`
- Modify: `apps/desktop/src/domain/contextMenus.ts`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/domain/desktopModel.test.ts`
- Test: `apps/desktop/src/domain/contextMenus.test.ts`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] **Step 1: Write failing section tests**

Assert:

```text
favourite rooms render in Favourites from RoomSummary.tags
low-priority rooms render in Low priority from RoomSummary.tags
DM rooms remain People even when favourite/low-priority tags are orthogonal where designed
context menu tag action dispatches set_room_tag/remove_room_tag
row moves only after Rust-shaped state update
```

- [ ] **Step 2: Implement sections and actions**

Use existing Rust-owned tags only. React must not maintain independent tag
membership or local section state.

- [ ] **Step 3: Run focused checks**

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run src/domain/desktopModel.test.ts src/domain/contextMenus.test.ts src/i18n/messages.test.ts
npx --prefix apps/desktop playwright test e2e/basic-operations.spec.ts --grep "room tag|favourite|low priority" --workers=1
```

Expected: all commands exit 0.

- [ ] **Step 4: Commit and close #22 if acceptance is complete**

Run:

```bash
git add apps/desktop docs AGENTS.md
git commit -m "Render room tag sections"
gh issue comment 22 --body "Phase B landed: room-list sections and tag actions render Rust-owned RoomSummary.tags and dispatch typed commands. Verification commands are recorded in the commit notes."
```

Close #22 only after the Linux virtual-display lane or accepted equivalent
evidence covers the GUI-operation test.

## Task 16: GUI Batch B3 - #18 Composer GUI

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: composer components in `apps/desktop/src`
- Modify: `apps/desktop/src/domain/composerKeyEvents.ts`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/domain/composerKeyEvents.test.ts`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] **Step 1: Write failing GUI tests**

Assert:

```text
typing @ opens autocomplete from Rust-shaped suggestions
selecting a member inserts a pill and sends typed mention intent
markdown toolbar dispatches formatted intent
slash command dispatches slash command intent
Enter with isComposing true never sends and never accepts autocomplete
```

- [ ] **Step 2: Implement GUI as typed client**

React owns popover focus and draft presentation. Rust/core owns mention payload,
markdown conversion, slash dispatch, and IME send-vs-commit decision.

- [ ] **Step 3: Run focused checks**

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run src/domain/composerKeyEvents.test.ts src/App.test.tsx src/i18n/messages.test.ts
npx --prefix apps/desktop playwright test e2e/basic-operations.spec.ts --grep "mention|markdown|slash|IME" --workers=1
```

Expected: all commands exit 0.

- [ ] **Step 4: Commit and comment**

Run:

```bash
git add apps/desktop docs AGENTS.md
git commit -m "Wire composer mention and formatting UI"
gh issue comment 18 --body "Phase B landed: composer autocomplete, mention pills, markdown/formatting controls, slash-command UI, and IME headless checks render/dispatch through Rust-owned contracts."
```

Close #18 only after browser-headless and Linux virtual-display evidence meet
the issue acceptance criteria.

## Task 17: GUI Batch B4 - #20, #21, #23, #7, #10, #32 Serialized GUI Slices

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx`
- Modify: `apps/desktop/src/components/SpaceInfoPanel.tsx`
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Modify: `apps/desktop/src/domain/desktopAttention.ts`
- Modify: `apps/desktop/src/domain/desktopNotification.ts`
- Modify: `apps/desktop/src/domain/desktopModel.ts`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/styles.css`
- Test: related `apps/desktop/src/**/*.test.ts`
- Test: `apps/desktop/e2e/basic-operations.spec.ts`

- [ ] **Step 1: Implement #20 Explore GUI**

Write failing browser-headless tests for open Explore, search, Join, and
Rust-shaped room-list update. Implement the view and typed commands. Commit:

```bash
git commit -m "Wire Explore public directory UI"
```

- [ ] **Step 2: Implement #21 room management GUI**

Write failing browser-headless tests for topic change and seeded-member kick.
Implement right-panel/settings/member action UI from Rust permission facts.
Commit:

```bash
git commit -m "Wire room management UI"
```

- [ ] **Step 3: Implement #23 Activity GUI**

Write failing browser-headless tests for Recent order, Unread stale room,
focused-context jump, row mark-read, and mark-all-read. Implement Activity rail
entry and tabs. Commit:

```bash
git commit -m "Wire account activity UI"
```

- [ ] **Step 4: Implement #7 Settings/Security status GUI**

Write failing component/browser-headless tests for Linux/macOS/Windows
capability profiles and health statuses. Render localized private-data-free
messages from Rust-owned state. Commit:

```bash
git commit -m "Render credential health status"
```

- [ ] **Step 5: Implement #10 notification/native adapter GUI**

Write failing tests for badge/attention rendering, adapter routing, permission
state, muted/low-priority behavior, and clearing on logout. Implement mockable
adapter ports and native smoke hooks. Commit:

```bash
git commit -m "Wire native attention adapter UI"
```

- [ ] **Step 6: Implement #32 Japanese/CJK GUI checks**

Write failing tests for `catalog_locale: "ja"`, root `lang`, Japanese visible
labels, CJK line-breaking/truncation, and no English leakage for shipped keys.
Implement only presentation wiring. Commit:

```bash
git commit -m "Verify Japanese and CJK GUI behavior"
```

- [ ] **Step 7: Run serialized GUI gate**

Run:

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npx --prefix apps/desktop playwright test --workers=1
npm --prefix apps/desktop run qa:secret-scan
```

Expected: all commands exit 0.

## Task 18: Linux Virtual-Display GUI Evidence

**Files:**
- Modify: `AGENTS.md` only if new lane behavior is learned
- Write artifacts under ignored `artifacts/`

- [ ] **Step 1: Check tools**

Run:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH node scripts/desktop-linux-gui-qa.mjs --check-tools
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH node scripts/desktop-linux-gui-qa.mjs --list
```

Expected: tools are present or missing-tool output is recorded.

- [ ] **Step 2: Run feature lanes**

Run focused scenarios as they exist or are added:

```bash
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-login --server=conduit --artifact-dir=artifacts/linux-gui-local-login-core-batch --timeout-ms=180000
PATH=/tmp/matrix-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:linux-gui -- --scenario=local-send --server=conduit --artifact-dir=artifacts/linux-gui-local-send-core-batch --timeout-ms=180000
```

Add and run feature-specific scenarios for pin, tag, Explore, moderation,
Activity, credential status, and notification status when those scenarios land.

- [ ] **Step 3: Update AGENTS.md only for durable operational lessons**

If a new GUI QA footgun is discovered, add a concise operational note to
`AGENTS.md`. Promote durable rules to canon docs instead.

- [ ] **Step 4: Commit QA updates**

Run:

```bash
git add AGENTS.md scripts apps/desktop/e2e apps/desktop/package.json
git commit -m "Add GUI QA evidence for remaining roadmap"
```

If only ignored artifacts changed, do not commit artifacts.

## Task 19: QA Batch Z - #9 And #31 Integration Gate

**Files:**
- Modify: `docs/qa/integration-edge-cases.md`
- Modify: `docs/qa/headless-basic-operations.md`
- Modify: `apps/desktop/e2e/basic-operations.spec.ts`
- Modify: `apps/desktop/e2e/*.spec.ts`
- Modify: `scripts/*.mjs` as needed for gates
- Test: Rust/headless/browser/native gates

- [ ] **Step 1: Freeze matrix rows for this release pass**

Mark each #31 row as one of:

```text
covered-headless
covered-linux-gui
native-release-smoke
deferred-out-of-scope
```

The classification must be in `docs/qa/integration-edge-cases.md` and mirrored
to #31.

- [ ] **Step 2: Add missing headless walkthroughs**

Add browser-headless walkthroughs for settings, i18n, font/emoji, Japanese/CJK,
E2EE trust, invites/DMs, media, live signals, profiles, composer, message
interactions, reactions, design shell, discovery, account-wide activity,
moderation, credential status, and notifications.

- [ ] **Step 3: Add audits**

Add or update:

```text
raw user-facing string scan or allowlist
logical CSS property audit or allowlist
private-data-free QA artifact check
IPC contract regeneration check
```

- [ ] **Step 4: Run final gates**

Run:

```bash
cargo fmt --check
cargo test -p matrix-desktop-state -p matrix-desktop-sdk -p matrix-desktop-core -p matrix-desktop-search -p matrix-desktop-key
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo check --target wasm32-unknown-unknown -p matrix-desktop-state -p matrix-desktop-search
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npx --prefix apps/desktop playwright test --workers=1
npm --prefix apps/desktop run test:ipc-contract
npm --prefix apps/desktop run qa:secret-scan
node scripts/desktop-release-gate-check.mjs --no-compile
```

Expected: all commands exit 0. Run local homeserver and Linux virtual-display
lanes when available; record exact skipped reason when unavailable.

- [ ] **Step 5: Commit QA Batch Z**

Run:

```bash
git add docs/qa scripts apps/desktop/e2e apps/desktop/src crates AGENTS.md
git commit -m "Complete roadmap integration QA gate"
```

- [ ] **Step 6: Close feature issues that meet criteria**

For each child issue, verify:

```text
Phase A evidence exists
Phase B GUI-operation evidence exists
Linux virtual-display evidence exists where required
docs and issue comments are synchronized
```

Then run:

```bash
gh issue close 19 --comment "Message interactions acceptance is complete. Evidence is recorded in #19 comments and the final #12 verification summary."
gh issue close 18 --comment "Composer mentions, markdown, and slash commands acceptance is complete. Evidence is recorded in #18 comments and the final #12 verification summary."
gh issue close 20 --comment "Public room directory and Explore acceptance is complete. Evidence is recorded in #20 comments and the final #12 verification summary."
gh issue close 21 --comment "Room management and moderation acceptance is complete. Evidence is recorded in #21 comments and the final #12 verification summary."
gh issue close 23 --comment "Account-wide Activity acceptance is complete. Evidence is recorded in #23 comments and the final #12 verification summary."
gh issue close 7 --comment "Credential-store strategy and encrypted local data status acceptance is complete. Evidence is recorded in #7 comments and the final #12 verification summary."
gh issue close 10 --comment "Notifications, sound, badges, and space attention acceptance is complete. Evidence is recorded in #10 comments and the final #12 verification summary."
gh issue close 32 --comment "Japanese/CJK localization and text handling acceptance is complete. Evidence is recorded in #32 comments and the final #12 verification summary."
```

Do not close an issue with missing evidence.

## Task 20: Final Merge And Umbrella Closure

**Files:**
- No code files unless a final verification defect is found
- GitHub issues and PR body

- [ ] **Step 1: Sync with origin**

Run:

```bash
git fetch origin
git status --short --branch
git merge --ff-only origin/main
```

Expected: branch is current or merge is clean.

- [ ] **Step 2: Run standing gates again**

Run the full command set from Task 19 Step 4. If any command fails, fix the
failure before continuing.

- [ ] **Step 3: Prepare final PR or merge**

If PR workflow is used:

```bash
git push origin codex/core-batch-a
gh pr create --draft --title "Complete remaining roadmap core and GUI batches" --body-file /tmp/matrix-desktop-core-batch-a/pr-body.md
```

If direct merge is approved by the user and repository policy allows it:

```bash
git switch main
git pull --ff-only origin main
git merge --no-ff codex/core-batch-a
git push origin main
```

Use a non-squash merge.

- [ ] **Step 4: Close #12 only after audit**

Before closing #12, verify every child issue in #12 is closed or explicitly
superseded and #9/#31 evidence is recorded. Then run:

```bash
gh issue close 12 --comment "Umbrella complete: all child issues are closed or superseded, Phase A/Phase B evidence is recorded, #31 integration matrix passed, and standing gates passed on main."
```

## Verification Summary Required In Final Response

When reporting batch completion, include:

- final commit range
- child issue status table
- exact verification commands run and exit status
- skipped native/local-homeserver gates with concrete reason
- remaining risks or deferred out-of-scope items

Do not mark the thread goal complete until this evidence proves every #12 close
criterion.
