# UI Readability Navigation Timeline Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve umbrella issue #130 and child issue #129 in one PR with one final #130 commit, with headless tests for every accepted visual/UI-readability requirement and every non-visual umbrella requirement.

**Architecture:** Matrix/product semantics remain Rust-owned. React changes are limited to presentation, panel state, viewport state, and dispatching typed commands/settings already modeled in Rust or added in this plan. Timeline/store order stays canonical; display ordering for thread lists and room lists is a Rust/state projection or settings-driven projection, not an ad hoc React repair.

**Tech Stack:** Rust (`koushi-state`, `koushi-core`, Tauri DTO tests), React + TypeScript (`apps/desktop`), Vitest/browser-headless component tests, Playwright where DOM behavior needs layout, Tauri contract golden tests, release-script asset tests.

---

## File Structure

- `docs/architecture/state-machine.md`: canon amendments for People/Profile panel state, thread ordering setting, room list sort setting, URL extraction/display policy, and avatar privacy.
- `docs/policies/engineering-rules.md`: avatar metadata privacy hardening if the current rule is insufficient for MXC/thumbnail/user-room associations.
- `crates/koushi-state/src/state/settings.rs`: add thread list ordering and room list sort preferences to Rust-owned settings.
- `crates/koushi-state/src/state/room_list.rs` or existing room-list state module: expand `RoomListSort` beyond activity/recent-first and normal locale order.
- `crates/koushi-state/src/state/thread.rs`: add display ordering mode to `ThreadsListState::Open` or sort items before projection.
- `crates/koushi-state/src/reducer/settings.rs`, `crates/koushi-state/src/reducer/thread.rs`, room-list reducer/composer files: apply new settings-driven projections.
- `crates/koushi-state/tests/*`: headless reducer/settings/thread/room-list/avatar privacy tests.
- `crates/koushi-core/src/link_preview.rs` and timeline projection code: centralize Unicode-aware plain-text URL extraction and link-preview card URL policy.
- `crates/koushi-core/tests/*`: URL extraction/projection and privacy tests.
- `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/types.ts`, checked-in event/golden artifacts, fakes, harness snapshots: DTO mirrors for any Rust fields.
- `apps/desktop/src/components/PeoplePanel.tsx`: new standalone People/Profile panel, consuming Rust-projected room member summaries.
- `apps/desktop/src/components/RoomInfoPanel.tsx`: remove dense member-card list and replace it with an entry point only.
- `apps/desktop/src/components/rightPanel.tsx`, `apps/desktop/src/domain/rightPanel.ts`, `apps/desktop/src/App.tsx`: add `people` and `profile` right-panel modes and route header/member actions.
- `apps/desktop/src/components/Shell.tsx`: Home/sidebar structure, room header actions, room/DM sorting presentation.
- `apps/desktop/src/components/panes.tsx`: Activity/Unread sender-centric rows.
- `apps/desktop/src/components/TimelineView.tsx`: compact timeline controls, read marker placement fix, clickable plain URLs/previews, action-menu placement/style hooks, footer baseline.
- `apps/desktop/src/styles.css`: compact controls, People/Profile, Activity rows, menu rows, footer baseline, icon surface fixes.
- `apps/desktop/src/i18n/messages.ts`: labels for People/Profile, controls, sorting settings.
- `apps/desktop/src/components/*.test.tsx`, `apps/desktop/src/domain/*.test.ts`, `apps/desktop/src/scripts/releaseScripts.test.ts`: headless UI/contract/asset tests.
- `apps/desktop/src-tauri/icons/*`, `apps/desktop/src-tauri/tauri.conf.json`: icon asset/config polish.

## Task 1: Canon And Contract Setup

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`
- Test: source/contract checks in existing docs tests if present; otherwise `rg`-based style contract in `apps/desktop/src/styles.contract.test.ts` only for CSS, not behavior.

- [ ] **Step 1: Amend canon before code**

Add the following durable contract:

```markdown
### UI readability #130

- Home is the account-level activity surface. The Home rail button opens
  Home and resets the main content to `Activity / Recent`; normal in-app Home
  navigation may preserve the current Home subsection.
- Activity `Recent` and `Unread` are message-centric inbox views. Event-backed
  rows foreground the sender avatar/display label and timestamp, then show
  context (`DM` or `Room · Space / Room`) and preview. Room-unread placeholder
  rows stay typed and eventless.
- People and Profile are right-panel states separate from Room info. Room info
  may link to People but must not render the dense member-card list.
- Thread-list display ordering is a Rust-owned setting. The SDK timeline/order
  is canonical; the UI receives a projected display order.
- Room/DM list sorting is a Rust-owned room-list projection setting with at
  least recent-first and locale-normal modes.
- Plain-text URL linkification and link-preview URL extraction use one
  Rust-owned Unicode-aware policy. React renders only the projected URL ranges
  and preview DTOs.
- Avatar MXC URIs, thumbnail state, and user-room avatar associations are
  account-scoped sensitive metadata. Debug, logs, tests, QA tokens, fixtures,
  and issue evidence must redact real values.
```

- [ ] **Step 2: Verify canon text is present**

Run:

```bash
rg -n "UI readability #130|People and Profile|Thread-list display ordering|Plain-text URL|Avatar MXC" docs/architecture/state-machine.md docs/policies/engineering-rules.md
```

Expected: all key phrases appear.

## Task 2: Settings And State RED Tests

**Files:**
- Modify tests: `crates/koushi-state/tests/settings_state.rs`
- Modify tests: `crates/koushi-state/tests/navigation_state.rs`
- Modify tests: `crates/koushi-state/tests/timeline_thread_state.rs`
- Modify tests: `crates/koushi-state/tests/room_management_state.rs`

- [ ] **Step 1: Add failing thread ordering settings test**

Add a test that expects default thread ordering to be latest reply and an explicit setting to switch to root chronology. The assertion must inspect `SettingsValues` defaults and `ThreadsListState::Open.items` order after reducer projection.

- [ ] **Step 2: Add failing room/DM sort settings test**

Add a test that expects `RoomListSort` to support `recentFirst` and `normalLocale`, and verifies rooms/DMs are projected in the selected order without React-local sorting.

- [ ] **Step 3: Add failing People/Profile state reset test**

Add a reducer/right-panel-adjacent state test if a serializable UI panel state is introduced. If People/Profile remains React presentation-only, add a browser-headless test in Task 8 instead and document that no Matrix product state changes.

- [ ] **Step 4: Run RED tests**

Run:

```bash
cargo test -p koushi-state --test settings_state thread_list_ordering_setting_defaults_to_latest_reply
cargo test -p koushi-state --test navigation_state room_list_sort_supports_recent_and_locale_modes
cargo test -p koushi-state --test timeline_thread_state threads_list_display_order_follows_setting
```

Expected: fail because settings/types/projection are missing.

## Task 3: URL Policy RED Tests

**Files:**
- Modify tests: `crates/koushi-core/tests/link_preview.rs` or create if absent.
- Modify tests: `crates/koushi-core/tests/timeline_link_preview.rs` where existing link-preview projection tests live.

- [ ] **Step 1: Add Unicode URL extraction tests**

Add tests for:

```text
https://tensor4all.org/blog/パス?q=日本語
https://example.com/foo(bar)
https://example.com/a、次の文
```

Expected policy: include Unicode path/query where valid, keep balanced parentheses, stop before CJK punctuation.

- [ ] **Step 2: Add projection test for plain-text clickable URL ranges and preview extraction**

The same URL policy must feed both plain-text link ranges and link previews.

- [ ] **Step 3: Run RED tests**

Run:

```bash
cargo test -p koushi-core link_preview_url_policy
```

Expected: fail because projected plain-text URL ranges or policy APIs are missing.

## Task 4: Avatar Privacy RED Tests

**Files:**
- Modify tests: `crates/koushi-state/tests/profile_state.rs`
- Modify tests: `crates/koushi-state/tests/room_management_state.rs`
- Modify tests: `crates/koushi-core/tests/event_redaction.rs`
- Modify tests: `apps/desktop/src-tauri/src/dto.rs` test module if DTO privacy contract needs coverage.

- [ ] **Step 1: Add debug redaction tests for avatar associations**

Assert Debug output for profile/avatar/member/room management state does not contain MXC URIs, thumbnail source URLs, or user-room association values.

- [ ] **Step 2: Add DTO/golden contract assertion**

Synthetic fixtures may contain `mxc://example.invalid`, but contract tests must state these are synthetic-only and normal Debug output redacts them.

- [ ] **Step 3: Run RED tests**

Run:

```bash
cargo test -p koushi-state avatar_metadata_debug_redacts_mxc_and_user_room_associations
cargo test -p koushi-core avatar_metadata_events_redact_private_mxc_values
```

Expected: fail where redaction is incomplete.

## Task 5: Frontend RED Tests For Accepted Visual Direction

**Files:**
- Modify tests: `apps/desktop/src/components/Shell.test.tsx`
- Modify tests: `apps/desktop/src/components/panes.test.tsx`
- Modify tests: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify tests: `apps/desktop/src/components/RoomInfoPanel.test.tsx`
- Create tests: `apps/desktop/src/components/PeoplePanel.test.tsx`
- Modify tests: `apps/desktop/src/components/rightPanel.test.tsx` if present, otherwise `apps/desktop/src/domain/rightPanel.test.ts`.

- [ ] **Step 1: Add Home panel tests**

Assert Home renders `Activity`, `Explore`, `Invites`, `Direct Messages`; does not render a Home `Rooms` section; Home rail click resets to Activity Recent.

- [ ] **Step 2: Add Activity row tests**

Assert event-backed Activity rows render sender avatar/name primary, timestamp, context `DM` or `Room · Space / Room`, and preview; both Recent and Unread use same hierarchy.

- [ ] **Step 3: Add Timeline top controls tests**

Assert visible text `Messages`, `Older messages`, `Jump to date` is absent from timeline chrome while accessible labels for older/date/latest exist.

- [ ] **Step 4: Add People/Profile tests**

Assert Room info has no dense member-card list; Members action opens standalone People panel; People search filters members; Invite button exists; selecting member opens Profile detail in same panel.

- [ ] **Step 5: Add Room header action tests**

Assert duplicate room-info/right-panel actions are absent; thread-list entry is hidden when there is no useful content or renders a tested empty state.

- [ ] **Step 6: Add message action menu tests**

Assert menu rows use neutral `.message-action-menu-item` structure, fixed icon column, long-label ellipsis class/style, and placement flips above/below based on viewport.

- [ ] **Step 7: Add footer baseline test**

Keep or extend the existing `message-status-row` test so reactions and read receipts share one parent and stable child order.

- [ ] **Step 8: Add read marker placement tests**

Construct a DM timeline with the user's latest message and a read marker. Assert `Read up to here` renders after the user's latest read event and not as a viewport-edge orphan.

- [ ] **Step 9: Add link preview/click tests**

Assert plain text URLs render as anchors, preview card/image are clickable anchors, and hide preview remains a button.

- [ ] **Step 10: Run RED tests**

Run:

```bash
npm --prefix apps/desktop run test -- --run \
  src/components/Shell.test.tsx \
  src/components/panes.test.tsx \
  src/components/TimelineView.test.tsx \
  src/components/RoomInfoPanel.test.tsx \
  src/components/PeoplePanel.test.tsx \
  src/domain/rightPanel.test.ts
```

Expected: new tests fail before implementation.

## Task 6: Implement Rust State, Settings, URL, Privacy Contracts

**Files:**
- Modify: `crates/koushi-state/src/state/settings.rs`
- Modify: `crates/koushi-state/src/state/thread.rs`
- Modify: room-list state/projection modules
- Modify: `crates/koushi-state/src/reducer/settings.rs`
- Modify: `crates/koushi-core/src/link_preview.rs`
- Modify: timeline projection modules
- Modify: Tauri DTO and TS mirrors.

- [ ] **Step 1: Add settings/types**

Add explicit serde enums:

```rust
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ThreadListOrder {
    LatestReply,
    RootChronology,
}

#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RoomListSort {
    RecentFirst,
    NormalLocale,
}
```

- [ ] **Step 2: Project thread and room-list display order in Rust/state**

Keep SDK order canonical internally and sort only the projected list sent to UI.

- [ ] **Step 3: Centralize URL policy**

Expose one function used by both link-preview candidates and plain-text link ranges.

- [ ] **Step 4: Harden avatar Debug contracts**

Ensure Debug output exposes only kind/count/placeholder values for MXC, thumbnail source, and user-room associations.

- [ ] **Step 5: Update DTO mirrors**

Update `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src/domain/types.ts`, fakes, harness snapshots, generated JSON/goldens.

- [ ] **Step 6: Run GREEN Rust/contract tests**

Run:

```bash
cargo test -p koushi-state --test settings_state
cargo test -p koushi-state --test navigation_state
cargo test -p koushi-state --test timeline_thread_state
cargo test -p koushi-state --test room_management_state
cargo test -p koushi-core link_preview_url_policy
cargo test -p koushi-core avatar_metadata_events_redact_private_mxc_values
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_app_state_golden_matches_maximally_populated_state
```

Expected: pass.

## Task 7: Implement React UI

**Files:**
- Create: `apps/desktop/src/components/PeoplePanel.tsx`
- Modify: `apps/desktop/src/components/rightPanel.tsx`
- Modify: `apps/desktop/src/domain/rightPanel.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/Shell.tsx`
- Modify: `apps/desktop/src/components/panes.tsx`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/RoomInfoPanel.tsx`
- Modify: `apps/desktop/src/styles.css`
- Modify: `apps/desktop/src/i18n/messages.ts`

- [ ] **Step 1: Implement People/Profile panel**

Render simple member rows from Rust-projected `RoomMemberSummary` fields. Search filters visible rows only. Selecting a member switches panel-local view to Profile.

- [ ] **Step 2: Simplify Room info**

Remove dense member-card list. Provide a People entry point that opens `RightPanelMode = "people"`.

- [ ] **Step 3: Update Home/sidebar/header actions**

Match accepted visual direction and remove duplicate/empty thread actions.

- [ ] **Step 4: Update Activity rows**

Render sender-centric hierarchy for event rows and typed fallback for placeholders.

- [ ] **Step 5: Update Timeline controls, read marker, links/previews, action menu, footer**

Use icon controls with accessible labels, corrected read marker placement, anchors for projected plain-text URLs and preview cards, neutral menu rows, and shared footer baseline.

- [ ] **Step 6: Run GREEN frontend tests**

Run:

```bash
npm --prefix apps/desktop run test -- --run \
  src/components/Shell.test.tsx \
  src/components/panes.test.tsx \
  src/components/TimelineView.test.tsx \
  src/components/RoomInfoPanel.test.tsx \
  src/components/PeoplePanel.test.tsx \
  src/domain/rightPanel.test.ts
npm --prefix apps/desktop run typecheck
```

Expected: pass.

## Task 8: App Icon Polish

**Files:**
- Modify: `apps/desktop/src-tauri/icons/icon.svg`
- Regenerate or replace: PNG/ICO/ICNS if needed.
- Modify: `apps/desktop/src-tauri/tauri.conf.json` if bundle icon config is inconsistent.
- Modify tests: `apps/desktop/src/scripts/releaseScripts.test.ts`

- [ ] **Step 1: Add failing asset/config test**

Assert icon config points to a consistent icon family and SVG/PNG do not include an unintended white rounded frame.

- [ ] **Step 2: Run RED test**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/scripts/releaseScripts.test.ts -t "app icon"
```

Expected: fail before icon/config change.

- [ ] **Step 3: Update icon assets/config**

Remove unintended white rounded frame and keep icon references consistent.

- [ ] **Step 4: Run GREEN test**

Run the same command. Expected: pass.

## Task 9: Full Verification And Single Commit

**Files:**
- All modified files.

- [ ] **Step 1: Run focused checks**

Run all focused commands from Tasks 6-8.

- [ ] **Step 2: Run broad headless checks**

Run:

```bash
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop run typecheck
cargo test -p koushi-state
cargo test -p koushi-core --lib
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
git diff --check
```

Expected: pass.

- [ ] **Step 3: Squash branch work into one #130 commit**

Because the branch already has handoff commits, reset/squash only after all tests pass:

```bash
git reset --soft origin/main
git commit -m "fix(#130): polish UI readability navigation and timeline surfaces"
```

Expected: one local commit containing all #130 changes.

- [ ] **Step 4: Push and open one PR**

Run:

```bash
git push --force-with-lease origin codex/linux-handoff-ui-feedback-20260625
gh pr create --repo shinaoka/koushi-matrix --base main --head codex/linux-handoff-ui-feedback-20260625 --draft
```

PR body must list #130 and #129, all headless verification, and explicitly state that all #130 bullets are implemented.

- [ ] **Step 5: Babysit PR to merge**

Run CI checks, address failures with root-cause fixes and headless tests, mark ready, merge with non-squash merge when green.
