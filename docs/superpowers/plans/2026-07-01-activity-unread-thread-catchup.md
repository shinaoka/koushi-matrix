# Activity Unread And Thread Catch-Up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Activity / Unread work as an event-level unread inbox, and make room threads easier to discover without reordering the main timeline.

**Architecture:** Keep unread and thread catch-up state Rust-owned. Add `ActivityStreamSummary` to the Activity DTO, project known unread events into `ActivityStream.unread.rows`, keep room unread placeholders only as unresolved fallbacks, and render the Unread tab count from Rust-provided summary metadata. Keep room threads room-scoped and expose the existing Threads right-panel button whenever an active room exists.

**Tech Stack:** Rust state/core reducers and runtime projection, Tauri DTO golden tests, TypeScript domain types, React Activity/Timeline pane components, Vitest, Cargo tests.

## Global Constraints

- React must not derive unread totals by scanning DOM rows or visible timeline rows.
- Activity event rows must open through `openActivityEvent(roomId, eventId)`, not search or focused-context navigation.
- Room-level Activity placeholders remain valid only when no unread event id is known.
- The main timeline must not duplicate old thread roots at the bottom.
- Muted and low-priority rooms stay excluded from Activity streams.
- Thread catch-up remains room-scoped in this pass; account-wide thread inbox is out of scope.
- Do not log Matrix room ids, event ids, user ids, message bodies, MXC URIs, local paths, or raw SDK errors in diagnostics or QA tokens.

---

## File Map

- Modify `crates/koushi-state/src/state/activity.rs`: add `ActivityStreamSummary`, add `ActivityStream::new`, and summarize streams from rows.
- Modify `crates/koushi-state/src/state/mod.rs` and `crates/koushi-state/src/lib.rs`: re-export `ActivityStreamSummary` with `ActivityStream`.
- Modify `crates/koushi-state/src/reducer/activity.rs`: recompute summary after filtering/sorting streams.
- Modify `crates/koushi-state/tests/activity_state.rs`: assert summary metadata and unchanged request-correlation behavior.
- Modify `crates/koushi-core/src/runtime.rs`: project known unread event rows before unresolved room placeholders.
- Modify `crates/koushi-core/tests/runtime_activity.rs`: cover known unread event rows, fallback placeholders, mark-read behavior, and muted-room filtering.
- Modify `crates/koushi-core/src/event.rs`: update Activity event serialization helpers for the new summary field.
- Modify `apps/desktop/src/domain/types.ts`: mirror `ActivityStreamSummary`.
- Modify `apps/desktop/src/domain/coreEvents.ts`: mirror `ActivityStreamSummary` for event wire types.
- Modify `apps/desktop/src/domain/coreEvents.generated.json`: update the checked-in wire contract artifact.
- Modify `apps/desktop/src/components/panes.tsx`: render Unread tab labels from stream summary and preserve placeholder/event row behavior.
- Modify `apps/desktop/src/components/panes.test.tsx`: cover unread tab counts and event-row direct-open wiring.
- Modify `apps/desktop/src/backend/browserFakeApi.ts`: emit event-level unread rows in the browser fake and compute stream summaries.
- Modify `apps/desktop/src/backend/browserFakeApi.test.ts`: update Activity unread expectations from all placeholders to event rows plus fallback placeholders.
- Modify `apps/desktop/src/test/appHarnessMain.tsx`: update direct Activity stream literals to include summaries.
- Modify `apps/desktop/src/components/panes.tsx`: always show the room-header Threads action when an active room exists.
- Modify `apps/desktop/src/components/Shell.test.tsx` and `apps/desktop/src/App.test.tsx`: update tests that expect conditional Threads visibility.
- Modify `apps/desktop/src-tauri/src/dto.rs`, `apps/desktop/src-tauri/src/lib.rs`, and `apps/desktop/src-tauri/tests/golden/frontend_app_state.json`: update golden DTO construction and wire snapshots.

## Task 1: Add Rust-Owned Activity Stream Summary

**Files:**
- Modify: `crates/koushi-state/src/state/activity.rs`
- Modify: `crates/koushi-state/src/state/mod.rs`
- Modify: `crates/koushi-state/src/lib.rs`
- Modify: `crates/koushi-state/src/reducer/activity.rs`
- Modify: `crates/koushi-state/tests/activity_state.rs`
- Modify: `crates/koushi-core/src/event.rs`

**Interfaces:**
- Produces: `ActivityStreamSummary { event_count, room_count, highlight_count, unresolved_room_count }`
- Produces: `ActivityStream::new(rows: Vec<ActivityRow>, next_batch: Option<String>) -> ActivityStream`
- Consumes: existing `ActivityRow.kind`, `ActivityRow.room_id`, and `ActivityRow.highlight`

- [ ] **Step 1: Write failing state tests for summary metadata**

In `crates/koushi-state/tests/activity_state.rs`, update the local helper so tests can construct summarized streams:

```rust
fn stream(rows: Vec<ActivityRow>, next_batch: Option<&str>) -> ActivityStream {
    ActivityStream::new(rows, next_batch.map(str::to_owned))
}
```

Add a focused test:

```rust
#[test]
fn activity_stream_summary_counts_event_rows_rooms_mentions_and_unresolved_rooms() {
    let event_a1 = row("!a", "$a1", 30);
    let mut event_a2 = row("!a", "$a2", 20);
    event_a2.highlight = true;
    let placeholder = ActivityRow::room_unread_placeholder(
        "!b".to_owned(),
        "Room B".to_owned(),
        10,
        true,
    );

    let stream = ActivityStream::new(vec![event_a1, event_a2, placeholder], None);

    assert_eq!(stream.summary.event_count, 2);
    assert_eq!(stream.summary.room_count, 2);
    assert_eq!(stream.summary.highlight_count, 2);
    assert_eq!(stream.summary.unresolved_room_count, 1);
}
```

- [ ] **Step 2: Run the failing state test**

Run:

```bash
cargo test -p koushi-state --test activity_state activity_stream_summary_counts_event_rows_rooms_mentions_and_unresolved_rooms
```

Expected: FAIL because `ActivityStream::new` and `ActivityStreamSummary` do not exist.

- [ ] **Step 3: Implement `ActivityStreamSummary` and `ActivityStream::new`**

In `crates/koushi-state/src/state/activity.rs`, add:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityStreamSummary {
    pub event_count: u32,
    pub room_count: u32,
    pub highlight_count: u32,
    pub unresolved_room_count: u32,
}
```

Update `ActivityStream`:

```rust
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityStream {
    pub rows: Vec<ActivityRow>,
    pub next_batch: Option<String>,
    #[serde(default)]
    pub summary: ActivityStreamSummary,
}
```

Add:

```rust
impl ActivityStream {
    pub fn new(rows: Vec<ActivityRow>, next_batch: Option<String>) -> Self {
        let summary = ActivityStreamSummary::from_rows(&rows);
        Self {
            rows,
            next_batch,
            summary,
        }
    }

    pub fn refresh_summary(&mut self) {
        self.summary = ActivityStreamSummary::from_rows(&self.rows);
    }
}

impl ActivityStreamSummary {
    pub fn from_rows(rows: &[ActivityRow]) -> Self {
        use std::collections::BTreeSet;

        let mut room_ids = BTreeSet::new();
        let mut event_count = 0_u32;
        let mut highlight_count = 0_u32;
        let mut unresolved_room_count = 0_u32;

        for row in rows {
            room_ids.insert(row.room_id.as_str());
            if row.highlight {
                highlight_count = highlight_count.saturating_add(1);
            }
            match row.kind {
                ActivityRowKind::Event => event_count = event_count.saturating_add(1),
                ActivityRowKind::RoomUnread => {
                    unresolved_room_count = unresolved_room_count.saturating_add(1);
                }
            }
        }

        Self {
            event_count,
            room_count: room_ids.len().try_into().unwrap_or(u32::MAX),
            highlight_count,
            unresolved_room_count,
        }
    }
}
```

- [ ] **Step 4: Refresh summaries after reducer filtering**

In `crates/koushi-state/src/reducer/activity.rs`, update `normalize_activity_stream` after sorting, and refresh after any row mutation such as mark-read removal:

```rust
stream.rows.sort_by(|left, right| {
    right
        .timestamp_ms
        .cmp(&left.timestamp_ms)
        .then_with(|| left.room_id.cmp(&right.room_id))
        .then_with(|| left.event_id.cmp(&right.event_id))
});
stream.refresh_summary();
stream
```

- [ ] **Step 5: Re-export summary and update remaining Rust struct literals**

Re-export `ActivityStreamSummary` alongside `ActivityStream` in `crates/koushi-state/src/state/mod.rs` and `crates/koushi-state/src/lib.rs`.

Replace direct `ActivityStream { rows, next_batch }` literals in:

- `crates/koushi-core/src/runtime.rs`
- `crates/koushi-core/src/event.rs`
- `apps/desktop/src-tauri/src/dto.rs`
- `apps/desktop/src-tauri/src/lib.rs`

with `ActivityStream::new(rows, next_batch)`.

- [ ] **Step 6: Run Task 1 verification**

Run:

```bash
cargo test -p koushi-state --test activity_state
cargo test -p koushi-core --lib activity
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_app_state_golden_matches_maximally_populated_state
```

Expected: state/core tests pass; the Tauri golden may fail until Task 3 updates the JSON.

## Task 2: Project Event-Level Unread Rows Before Room Placeholders

**Files:**
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/tests/runtime_activity.rs`

**Interfaces:**
- Consumes: `ActivityProjection::rows_by_event_id`
- Produces: `ActivityStream.unread.rows` with `ActivityRowKind::Event` rows whenever an unread event id is known
- Preserves: `ActivityRowKind::RoomUnread` fallback for unresolved unread rooms

- [ ] **Step 1: Write failing runtime tests for event-level unread rows**

In `crates/koushi-core/tests/runtime_activity.rs`, use the existing test style (`CoreRuntime::start()`, `runtime.attach()`, `runtime.inject_actions(...)`, and helpers from `tests/support/mod.rs` such as `activity_row`, `unread_room_summary`, and `wait_for_state`). Add a test that observes an activity row, opens Activity, and expects Unread to contain an event row:

```rust
#[tokio::test]
async fn activity_unread_prefers_known_event_rows_over_room_placeholders() {
    // Follow the existing runtime_activity.rs arrangement:
    // start the runtime, attach a session, inject an unread_room_summary,
    // inject AppAction::ActivityRowsObserved, then AppAction::OpenActivity.
    // Use wait_for_state to read the ActivityState::Open snapshot.
    let ActivityState::Open { unread, .. } = snapshot.state.activity else {
        panic!("activity should be open");
    };

    assert!(
        unread.rows.iter().any(|row| {
            row.kind == ActivityRowKind::Event
                && row.room_id == "!room-a:example.test"
                && row.event_id.as_deref() == Some("$known-unread")
        }),
        "known unread event should be directly openable from Activity"
    );
    assert!(
        unread.rows.iter().all(|row| {
            row.kind != ActivityRowKind::RoomUnread || row.room_id != "!room-a:example.test"
        }),
        "placeholder should not be synthesized when a known unread event row exists"
    );
    assert_eq!(unread.summary.event_count, 1);
    assert_eq!(unread.summary.unresolved_room_count, 0);
}
```

Use existing test helpers where names differ; keep the assertions identical.

- [ ] **Step 2: Write failing runtime test for fallback placeholders**

Add:

```rust
#[tokio::test]
async fn activity_unread_keeps_room_placeholder_when_no_event_row_is_known() {
    // Use CoreRuntime::start(), attach(), inject unread_room_summary(...),
    // then open Activity as existing tests do.
    let ActivityState::Open { unread, .. } = snapshot.state.activity else {
        panic!("activity should be open");
    };

    assert!(
        unread.rows.iter().any(|row| {
            row.kind == ActivityRowKind::RoomUnread
                && row.room_id == "!room-b:example.test"
                && row.event_id.is_none()
        }),
        "unresolved unread room should still be openable as a room fallback"
    );
    assert_eq!(unread.summary.event_count, 0);
assert_eq!(unread.summary.unresolved_room_count, 1);
}
```

- [ ] **Step 3: Add coverage for the fully-read marker boundary**

Add a test with multiple observed rows in the same room and a known `fully_read_event_id`. Assert that only rows after the marker are projected into `unread.rows`, while older observed rows remain in `recent.rows`.

Add a separate latest-event fallback test where the fully-read marker is not in `rows_by_event_id`; assert that the room contributes at most one unread event row and no duplicate recent row. This guards the current fallback logic from over-projecting every observed row in long rooms.

- [ ] **Step 4: Run failing runtime tests**

Run:

```bash
cargo test -p koushi-core --test runtime_activity activity_unread_
```

Expected: the event-row test fails because Unread only contains room placeholders.

- [ ] **Step 5: Implement event-row projection**

In `ActivityProjection::snapshot` in `crates/koushi-core/src/runtime.rs`, track rooms that already have a known unread event:

```rust
let mut unread_event_room_ids = BTreeSet::new();
```

Inside the `for row in self.rows_by_event_id.values()` loop, after the projected `row` is built and pushed to `recent`, add:

```rust
if row.unread {
    unread_event_room_ids.insert(row.room_id.clone());
    unread.push(row.clone());
}
```

Inside the room `latest_event` fallback loop, replace the whole row push block so the row is pushed to `recent` exactly once and cloned into `unread` only when unread:

```rust
if row.unread {
    unread_event_room_ids.insert(row.room_id.clone());
    unread.push(row.clone());
}
recent.push(row);
```

- [ ] **Step 6: Suppress placeholders for rooms with known unread event rows**

In the placeholder loop, before creating `ActivityRow::room_unread_placeholder`, add:

```rust
if unread_event_room_ids.contains(&room.room_id) {
    continue;
}
```

Return streams through `ActivityStream::new(recent, None)` and `ActivityStream::new(unread, None)` so summaries are correct.

- [ ] **Step 7: Verify mark-read behavior still works**

Run:

```bash
cargo test -p koushi-core --test runtime_activity
cargo test -p koushi-state --test activity_state
```

Expected: all Activity runtime and state tests pass. Update existing assertions that intentionally expected all unread rows to be placeholders.

## Task 3: Mirror Activity Summary Through Tauri, TypeScript, And Browser Fake

**Files:**
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.test.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/tests/golden/frontend_app_state.json`

**Interfaces:**
- Produces TS `ActivityStreamSummary`
- Produces browser fake `createActivityStream(rows, nextBatch)`
- Preserves `openActivityEvent(roomId, eventId)` direct anchor behavior

- [ ] **Step 1: Add TS type tests by updating browser fake expectations**

In `apps/desktop/src/backend/browserFakeApi.test.ts`, replace checks that require every unread row to be `roomUnread` with assertions for event-level rows:

```ts
expect(
  opened.state.domain.activity.unread.rows.some(
    (row) =>
      row.kind === "event" &&
      row.event_id === "$alpha-update" &&
      row.room_id === "!room-alpha:example.invalid"
  )
).toBe(true);
expect(opened.state.domain.activity.unread.summary.event_count).toBeGreaterThan(0);
expect(opened.state.domain.activity.unread.summary.room_count).toBeGreaterThan(0);
```

Keep an assertion that a room with unread count and no known event row still renders `roomUnread`.

- [ ] **Step 2: Run failing browser fake test**

Run:

```bash
npm --prefix apps/desktop test -- src/backend/browserFakeApi.test.ts
```

Expected: FAIL because TS types and browser fake do not expose `summary`.

- [ ] **Step 3: Add TypeScript summary interfaces**

In both `apps/desktop/src/domain/types.ts` and `apps/desktop/src/domain/coreEvents.ts`, add:

```ts
export interface ActivityStreamSummary {
  event_count: number;
  room_count: number;
  highlight_count: number;
  unresolved_room_count: number;
}
```

Update `ActivityStream`:

```ts
export interface ActivityStream {
  rows: ActivityRow[];
  next_batch: string | null;
  summary: ActivityStreamSummary;
}
```

- [ ] **Step 4: Add browser fake stream summary helpers**

In `apps/desktop/src/backend/browserFakeApi.ts`, add:

```ts
function createActivityStream(rows: ActivityRow[], nextBatch: string | null): ActivityStream {
  const sortedRows = sortActivityRows(rows);
  return {
    rows: sortedRows,
    next_batch: nextBatch,
    summary: summarizeActivityRows(sortedRows)
  };
}

function summarizeActivityRows(rows: ActivityRow[]): ActivityStream["summary"] {
  const roomIds = new Set(rows.map((row) => row.room_id));
  return {
    event_count: rows.filter((row) => row.kind === "event").length,
    room_count: roomIds.size,
    highlight_count: rows.filter((row) => row.highlight).length,
    unresolved_room_count: rows.filter((row) => row.kind === "roomUnread").length
  };
}
```

Update `createActivityStreams` so unread rows include event rows from `activityRows(...)` for unread rooms and unresolved placeholders only for unread rooms that have no event row.

Also update browser fake mutations that replace or filter streams, including pagination and mark-read paths, to rebuild streams through `createActivityStream(...)` so the UI never reads stale summary counts.

Update direct Activity stream literals in `apps/desktop/src/test/appHarnessMain.tsx` to include `summary`, or route them through a shared helper if the file already has one.

- [ ] **Step 5: Update Tauri serialization fixtures and golden**

Update Rust test fixture stream literals in `apps/desktop/src-tauri/src/dto.rs` and `apps/desktop/src-tauri/src/lib.rs` to use `ActivityStream::new(...)`.

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_app_state_golden_matches_maximally_populated_state
```

Expected: FAIL with a golden diff containing `summary`.

Regenerate or edit `apps/desktop/src-tauri/tests/golden/frontend_app_state.json` so each Activity stream includes:

```json
"summary": {
  "event_count": 1,
  "room_count": 1,
  "highlight_count": 0,
  "unresolved_room_count": 0
}
```

Use the actual values from the failing golden output.

Update `apps/desktop/src/domain/coreEvents.generated.json` by running the existing contract-generation path used by the repository, or by applying the exact expected diff reported by `core_event_wire_format_matches_checked_in_contract_artifact`.

- [ ] **Step 6: Run Task 3 verification**

Run:

```bash
npm --prefix apps/desktop test -- src/backend/browserFakeApi.test.ts
npm --prefix apps/desktop run typecheck
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_app_state_golden_matches_maximally_populated_state
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
```

Expected: all pass.

## Task 4: Render Activity Unread Counts And Preserve Direct Event Opening

**Files:**
- Modify: `apps/desktop/src/components/panes.tsx`
- Modify: `apps/desktop/src/components/panes.test.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts`
- Modify: `apps/desktop/src/App.test.tsx`

**Interfaces:**
- Consumes: `ActivityStream.summary`
- Produces: `activityTabLabel(tab, activity)` that renders count labels only from Rust-provided summary metadata
- Preserves: `row.kind === "event"` calls `openActivityRow(row.room_id, row.event_id)`
- Preserves: `row.kind === "roomUnread"` calls `openActivityRoom(row.room_id)`

- [ ] **Step 1: Write ActivityPane count tests**

In `apps/desktop/src/components/panes.test.tsx`, extend the existing `activityState(rows: ActivityRow[])` helper to build `ActivityStream.summary` from rows, or add explicit local helpers for `activityStream`, `activityEventRow`, and `activityRoomUnreadRow`. Do not rely on undeclared helpers. Add a test with an open Activity state where unread contains two event rows and one unresolved placeholder:

```tsx
it("renders unread tab counts from stream summary", () => {
  render(
    <ActivityPane
      activity={activityOpen({
        active_tab: "recent",
        unread: activityStream([
          activityEventRow("$a", "!a", 10),
          activityEventRow("$b", "!b", 20),
          activityRoomUnreadRow("!c", 30)
        ])
      })}
      onClose={() => undefined}
      onLoadMore={() => undefined}
      onMarkRead={() => undefined}
      onOpenRow={() => undefined}
      onSetTab={() => undefined}
    />
  );

  expect(screen.getByRole("tab", { name: "Unread (2)" })).toBeTruthy();
});
```

Add a second test where summary has zero events and two unresolved rooms:

```tsx
expect(screen.getByRole("tab", { name: "Unread (2 rooms)" })).toBeTruthy();
```

Use singular copy for one unresolved room:

```tsx
expect(screen.getByRole("tab", { name: "Unread (1 room)" })).toBeTruthy();
```

- [ ] **Step 2: Run failing ActivityPane tests**

Run:

```bash
npm --prefix apps/desktop test -- src/components/panes.test.tsx
```

Expected: FAIL because the tab label is still static.

- [ ] **Step 3: Add localized count messages**

In `apps/desktop/src/i18n/messages.ts`, add keys to the message-key union:

```ts
| "activity.unreadWithCount"
| "activity.unreadWithRoomCount"
| "activity.unreadWithOneRoom"
```

Add English messages:

```ts
"activity.unreadWithCount": "Unread ({count})",
"activity.unreadWithOneRoom": "Unread (1 room)",
"activity.unreadWithRoomCount": "Unread ({count} rooms)",
```

Add Japanese messages:

```ts
"activity.unreadWithCount": "未読 ({count})",
"activity.unreadWithOneRoom": "未読 (1ルーム)",
"activity.unreadWithRoomCount": "未読 ({count}ルーム)",
```

- [ ] **Step 4: Render labels from summary**

In `apps/desktop/src/components/panes.tsx`, replace `activityTabLabel(tab)` with:

```ts
function activityTabLabel(
  tab: ActivityTab,
  activity?: Extract<ActivityState, { kind: "open" }>
): string {
  if (tab === "recent") {
    return t("activity.recent");
  }
  const summary = activity?.unread.summary;
  if (!summary) {
    return t("activity.unread");
  }
  if (summary.event_count > 0) {
    return t("activity.unreadWithCount", { count: String(summary.event_count) });
  }
  if (summary.unresolved_room_count > 0) {
    if (summary.unresolved_room_count === 1) {
      return t("activity.unreadWithOneRoom");
    }
    return t("activity.unreadWithRoomCount", {
      count: String(summary.unresolved_room_count)
    });
  }
  return t("activity.unread");
}
```

Use it in the tablist and section label:

```tsx
const activityOpenState = activity.kind === "open" ? activity : undefined;
...
{activityTabLabel(tab, activityOpenState)}
...
<section className="activity-scroll" aria-label={activityTabLabel(activeTab, activityOpenState)}>
```

- [ ] **Step 5: Keep App routing tests explicit**

Keep `apps/desktop/src/App.test.tsx` assertions:

```ts
expect(openActivityRowSource).toContain(".openActivityEvent(roomId, eventId)");
expect(activityRenderSource).toContain("openActivityRow(row.room_id, row.event_id)");
expect(activityRenderSource).toContain('row.kind === "roomUnread"');
expect(activityRenderSource).toContain("openActivityRoom(row.room_id)");
```

If formatting changes, update the source-slice assertions without weakening the behavior.

- [ ] **Step 6: Run Task 4 verification**

Run:

```bash
npm --prefix apps/desktop test -- src/components/panes.test.tsx src/App.test.tsx
npm --prefix apps/desktop run typecheck
```

Expected: all pass.

## Task 5: Make Room Threads Entry Always Discoverable

**Files:**
- Modify: `apps/desktop/src/components/panes.tsx`
- Modify: `apps/desktop/src/components/Shell.test.tsx`
- Modify: `apps/desktop/src/App.test.tsx`

**Interfaces:**
- Consumes: `timelineRoomId`
- Produces: room-header Threads button whenever `timelineRoomId` is non-null
- Preserves: sidebar Threads button and `ThreadsListView` right-panel flow

- [ ] **Step 1: Write/update failing header visibility test**

In `apps/desktop/src/App.test.tsx`, update the test named `room header wires People and media actions and conditionally shows threads` so it expects the room header to show Threads for an active room without requiring `thread_attention` counts. The source assertion should check:

```ts
expect(paneSource).toContain("showThreadsHeader = Boolean(timelineRoomId)");
expect(paneSource).toContain("onOpenThreadsStable");
```

Remove assertions that require `threadAttention.kind === "tracking"` for header visibility.

In `apps/desktop/src/components/panes.test.tsx`, add a render-level test where `timelineRoomId` is present and `thread_attention.kind === "closed"`; assert the header Threads button is visible and calls `onOpenThreads`.

- [ ] **Step 2: Run failing app test**

Run:

```bash
npm --prefix apps/desktop test -- src/App.test.tsx
```

Expected: FAIL because `showThreadsHeader` still depends on `thread_attention`.

- [ ] **Step 3: Simplify room-header Threads visibility**

In `apps/desktop/src/components/panes.tsx`, replace:

```ts
const threadAttention = snapshot.state.domain.thread_attention;
const showThreadsHeader =
  timelineRoomId &&
  threadAttention.kind === "tracking" &&
  threadAttention.room_id === timelineRoomId &&
  (threadAttention.notification_count > 0 ||
    threadAttention.highlight_count > 0 ||
    threadAttention.live_event_marker_count > 0);
```

with:

```ts
const showThreadsHeader = Boolean(timelineRoomId);
```

Keep the button's `aria-label`, `title`, and `onClick={onOpenThreadsStable}` unchanged.

- [ ] **Step 4: Preserve sidebar thread attention badges**

Do not remove `threadAttention` usage in `apps/desktop/src/components/Shell.tsx`. Sidebar badges still reflect current Rust-owned `thread_attention`; the room header button is now purely discoverability.

- [ ] **Step 5: Run Task 5 verification**

Run:

```bash
npm --prefix apps/desktop test -- src/components/Shell.test.tsx src/App.test.tsx src/components/TimelinePane.renderIsolation.test.tsx
npm --prefix apps/desktop run typecheck
```

Expected: all pass.

## Task 6: Full Regression Verification

**Files:**
- No new production files.
- Update tests only if compile or golden failures expose exact expected wire-shape changes from earlier tasks.

**Interfaces:**
- Verifies all task outputs together.

- [ ] **Step 1: Run Rust state/core Activity tests**

Run:

```bash
cargo test -p koushi-state --test activity_state
cargo test -p koushi-core --test runtime_activity
```

Expected: all pass.

- [ ] **Step 2: Run Tauri DTO and contract tests**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml frontend_app_state_golden_matches_maximally_populated_state
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml open_activity_event_sets_bottom_scroll_anchor_without_opening_focused_context
```

Expected: all pass.

- [ ] **Step 3: Run focused frontend tests**

Run:

```bash
npm --prefix apps/desktop test -- src/components/panes.test.tsx src/backend/browserFakeApi.test.ts src/App.test.tsx src/components/Shell.test.tsx src/components/TimelinePane.renderIsolation.test.tsx
npm --prefix apps/desktop run typecheck
```

Expected: all pass.

- [ ] **Step 4: Run formatting and diff hygiene**

Run:

```bash
cargo fmt
npm --prefix apps/desktop run lint -- --quiet
git diff --check
```

Expected: `cargo fmt` exits 0, ESLint exits 0, and `git diff --check` exits 0.

- [ ] **Step 5: Manual smoke target**

Run the desktop harness or app, open Activity, switch to Unread, and verify:

- The Unread tab shows a count.
- Known event rows show sender, timestamp, room context, and preview.
- Clicking a known event row opens the room and scrolls to that event.
- A room with unresolved unread still opens as a room fallback.
- The room header always shows the Threads button when a room is active.
- Opening Threads shows the existing latest-reply ordered right-panel list.

Record the result in the PR body or final handoff.
