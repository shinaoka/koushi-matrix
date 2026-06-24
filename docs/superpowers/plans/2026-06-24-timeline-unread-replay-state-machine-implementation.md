# Timeline Unread Replay State Machine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix #127 by making room timeline replay durable across React view lifecycles and making Activity Unread derive membership from Rust-owned `RoomSummary` unread state.

**Architecture:** Add a typed `ActivityRow` source contract (`Event` vs `RoomUnread`) and make `ActivityProjection` synthesize eventless unread-room rows from `RoomSummary`. Move timeline CoreEvent reducer application out of `TimelineView` and into an App-level bounded projection store, then expose that store to view components while views keep presentation-only event side effects.

**Tech Stack:** Rust `koushi-state`/`koushi-core`, Tauri DTO mirrors, React/TypeScript, Vitest, Cargo tests.

---

## References

- Spec: `docs/superpowers/specs/2026-06-24-timeline-unread-replay-state-machine-design.md`
- Canon: `docs/architecture/state-machine.md` Account Activity section.
- Rules: `REPOSITORY_RULES.md`, `docs/policies/engineering-rules.md`, `AGENTS.md`.

## File Map

- Modify: `crates/koushi-state/src/state/activity.rs`
  - Add typed row source/kind and optional event id.
- Modify: `crates/koushi-state/src/lib.rs`
  - Re-export any new activity row kind/source type.
- Modify: `crates/koushi-state/tests/activity_state.rs`
  - RED and GREEN coverage for typed row serialization/debug and reducer behavior.
- Modify: `crates/koushi-core/src/runtime.rs`
  - Synthesize room-unread placeholder rows, keep event rows preferred, split mark-all event clears from room placeholder clears.
- Modify: `crates/koushi-core/src/timeline.rs`
  - Populate event-backed `ActivityRow` kind/event id.
- Modify: `crates/koushi-core/src/event.rs`
  - Update Activity event serialization/redaction tests/helpers for optional event ids.
- Modify: `crates/koushi-core/tests/runtime_activity.rs`
  - RED and GREEN coverage for placeholder rows and mark-all behavior.
- Modify: `crates/koushi-core/tests/support/mod.rs`
  - Update `activity_row` helper to create event rows.
- Modify: `apps/desktop/src/domain/types.ts`
  - Mirror typed `ActivityRow`.
- Modify: `apps/desktop/src/domain/coreEvents.ts`
  - Mirror Activity event/state row shape if generated file is hand-maintained.
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
  - Update checked-in contract artifact.
- Modify: `apps/desktop/src-tauri/src/dto.rs` and/or `apps/desktop/src-tauri/src/lib.rs`
  - Update Tauri serialization contract tests/fixtures.
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
  - Produce typed event rows and typed room-unread rows.
- Modify: `apps/desktop/src/backend/browserFakeApi.test.ts`
  - Assert browser fake Activity placeholder behavior.
- Modify: `apps/desktop/src/components/panes.tsx`
  - Render Activity `RoomUnread` rows without event open or row-level mark-read.
- Modify: `apps/desktop/src/components/TimelineView.tsx`
  - Stop owning the CoreEvent reducer/store. Accept App-level store context/props
    and keep only presentation state plus view-local event side effects.
- Modify: `apps/desktop/src/components/panes.tsx`
  - Thread timeline props through to `TimelineView`.
- Modify: `apps/desktop/src/App.tsx`
  - Own bounded timeline projection store and the single CoreEvent listener.
- Modify: `apps/desktop/src/domain/timelineStore.ts`
  - Add bounded retention helper if needed.
- Modify: `apps/desktop/src/domain/timelineStore.test.ts`
  - Cover bounded retention helper if introduced.
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
  - RED and GREEN coverage for pre-populated App-level store render after missed `InitialItems`.
- Modify: `docs/architecture/state-machine.md`
  - Update Account Activity notes for RoomSummary-authoritative unread membership and typed room-unread rows.

## Task 1: Activity Row Typed Contract

- [ ] **Step 1: Write failing state tests**

Add tests in `crates/koushi-state/tests/activity_state.rs`:

```rust
#[test]
fn activity_row_event_source_serializes_real_event_id() {
    let row = row("!room", "$event", 10);
    let value = serde_json::to_value(&row).expect("serialize activity row");
    assert_eq!(value["kind"], serde_json::json!("event"));
    assert_eq!(value["event_id"], serde_json::json!("$event"));
}

#[test]
fn activity_row_room_unread_source_has_no_event_id_and_redacted_debug() {
    let row = ActivityRow::room_unread_placeholder(
        "!private-room:example.invalid".to_owned(),
        "Private Room".to_owned(),
        42,
        true,
    );
    let value = serde_json::to_value(&row).expect("serialize activity row");
    assert_eq!(value["kind"], serde_json::json!("roomUnread"));
    assert_eq!(value["event_id"], serde_json::Value::Null);
    let debug = format!("{row:?}");
    assert!(!debug.contains("!private-room:example.invalid"));
    assert!(!debug.contains("Private Room"));
}
```

- [ ] **Step 2: Run RED**

Run: `cargo test -p koushi-state --test activity_state activity_row_`

Expected: FAIL because `ActivityRow::room_unread_placeholder`, `kind`, and nullable `event_id` do not exist.

- [ ] **Step 3: Implement minimal typed row contract**

In `crates/koushi-state/src/state/activity.rs` add:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityRowKind {
    #[default]
    Event,
    RoomUnread,
}
```

Change `ActivityRow` to include `#[serde(default)] pub kind: ActivityRowKind` and `pub event_id: Option<String>`. Add constructors:

```rust
impl ActivityRow {
    pub fn event(room_id: String, event_id: String, room_label: String, sender_label: Option<String>, preview: Option<String>, timestamp_ms: u64, unread: bool, highlight: bool) -> Self { ... }
    pub fn room_unread_placeholder(room_id: String, room_label: String, timestamp_ms: u64, highlight: bool) -> Self { ... }
}
```

Keep `Debug` redacted and expose `kind`, not private ids or labels.

- [ ] **Step 4: Update Rust helpers/fixtures**

Update all Rust `ActivityRow { ... }` literals to use constructors or `kind: ActivityRowKind::Event, event_id: Some(...)`.

- [ ] **Step 5: Run GREEN for state**

Run: `cargo test -p koushi-state --test activity_state`

Expected: PASS.

## Task 2: Rust Activity Projection Root Fix

- [ ] **Step 1: Write failing runtime tests**

In `crates/koushi-core/tests/runtime_activity.rs`, add tests that:

- Open Activity with `RoomListUpdated` containing an unread room that has no `ActivityRowsObserved` row.
- Assert `ActivityState::Open.unread.rows` contains one `RoomUnread` row with `event_id == None`.
- Add one observed event row for another unread room and assert no placeholder is generated for that room.
- Run `MarkActivityRead { target: All }` and assert:
  - placeholder room `unread_count` becomes `0`;
  - `live_signals.rooms[placeholder].fully_read_event_id` stays `None`;
  - `ActivityEvent::MarkedRead.cleared_event_ids` contains only real event ids.

- [ ] **Step 2: Run RED**

Run: `cargo test -p koushi-core --test runtime_activity`

Expected: FAIL because placeholders are not synthesized and event ids are still required.

- [ ] **Step 3: Implement projection and mark-all split**

In `crates/koushi-core/src/runtime.rs`:

- Store observed rows by real event id only.
- When building Unread, track rooms that already have unread event rows.
- For unread/highlight/marked-unread rooms missing an unread event row, append `ActivityRow::room_unread_placeholder(...)`.
- Use `RoomSummary.display_label`, not `display_name`, for observed and placeholder row labels.
- For `ActivityMarkReadTarget::All`, compute both:
  - real cleared event ids for event rows;
  - placeholder room ids for eventless room rows.
- Emit `FullyReadMarkerUpdated` only for real event ids.
- Emit `RoomMarkedAsReadSucceeded` for placeholder room ids and for rooms whose real unread rows are fully cleared.
- Emit `ActivityEvent::MarkedRead` with real cleared event ids only.

- [ ] **Step 4: Run GREEN for core**

Run: `cargo test -p koushi-core --test runtime_activity`

Expected: PASS.

## Task 3: DTO And Frontend Type Mirrors

- [ ] **Step 1: Update TS row shape**

In `apps/desktop/src/domain/types.ts` and `apps/desktop/src/domain/coreEvents.ts`, change `ActivityRow` to:

```ts
export type ActivityRowKind = "event" | "roomUnread";

export interface ActivityRow {
  kind: ActivityRowKind;
  room_id: string;
  event_id: string | null;
  room_label: string;
  sender_label: string | null;
  preview: string | null;
  timestamp_ms: number;
  unread: boolean;
  highlight: boolean;
}
```

- [ ] **Step 2: Update generated/checked contract artifacts**

Update `apps/desktop/src/domain/coreEvents.generated.json` and Tauri DTO tests/fixtures to include `kind` and nullable `event_id`.

- [ ] **Step 3: Update browser fake Activity rows**

In `apps/desktop/src/backend/browserFakeApi.ts`, event rows use `kind: "event"`, placeholder rows use `kind: "roomUnread", event_id: null`.

- [ ] **Step 4: Run mirror checks**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
npm --prefix apps/desktop run typecheck
```

Expected: PASS.

## Task 4: Activity GUI Rendering

- [ ] **Step 1: Write failing GUI/fake tests**

In `apps/desktop/src/backend/browserFakeApi.test.ts`, assert an unread room without an event row appears as `kind: "roomUnread"` with `event_id: null`.

In a component or browser-headless test for `ActivityPane`, assert:

- room-unread rows render room label and unread badge;
- no preview text is required;
- row open button and row-level mark-read button are absent/disabled for `kind: "roomUnread"`.

- [ ] **Step 2: Run RED**

Run: `npm --prefix apps/desktop run test -- --run src/backend/browserFakeApi.test.ts`

Expected: FAIL until fake/types/rendering are updated.

- [ ] **Step 3: Implement rendering**

In `apps/desktop/src/components/panes.tsx`, branch on `row.kind`:

- event row: current behavior, call `onOpenRow(row)` and show row mark-read button with `row.event_id`.
- roomUnread row: render a non-event row body, no event-open call, no row mark-read button.

- [ ] **Step 4: Run GREEN**

Run: `npm --prefix apps/desktop run test -- --run src/backend/browserFakeApi.test.ts`

Expected: PASS.

## Task 5: App-Level Timeline Projection Store

- [ ] **Step 1: Write failing TimelineView test**

In `apps/desktop/src/components/TimelineView.test.tsx`, render `TimelineView` with a pre-populated App-level store containing `InitialItems` for `KEY`. Assert the row renders immediately. Also emit a key-matching presentation event such as `MessageSourceLoaded` and assert the view handles it without invoking the App-level store setter.

- [ ] **Step 2: Run RED**

Run: `npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx`

Expected: FAIL because `TimelineView` still creates its own store and conflates reducer application with view-local event handling.

- [ ] **Step 3: Add App-owned store props**

In `TimelineView.tsx`, replace local reducer ownership with App-level context/props:

```ts
timelineStore: TimelineStoreState;
setTimelineStore: React.Dispatch<React.SetStateAction<TimelineStoreState>>;
```

Keep key-specific event handling for presentation-only side effects, but do not apply store mutations from the view when an App-level store is present.

- [ ] **Step 4: Add App-level listener**

In `App.tsx`, add one `TimelineStoreState` and one reducer-owning `listenCoreEvents` effect for timeline events. Apply all timeline events to the store, and handle `ResyncMarker` with `applyGlobalResync`.

- [ ] **Step 5: Bound retention**

In `apps/desktop/src/domain/timelineStore.ts`, add a helper that prunes inactive keys after event application. Retain selected room, focused context, thread, and eight recent inactive keys. Cover it with `timelineStore.test.ts`.

- [ ] **Step 6: Thread props through**

Pass `timelineStore` and `setTimelineStore` from `App.tsx` to `TimelinePane`, then to `TimelineView`.

- [ ] **Step 7: Run frontend checks**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/timelineStore.test.ts src/components/TimelineView.test.tsx
npm --prefix apps/desktop run typecheck
```

Expected: PASS.

## Task 6: Canon Update

- [ ] **Step 1: Update state-machine docs**

In `docs/architecture/state-machine.md` Account Activity section, add:

- Unread membership is authoritative from `RoomSummary` unread/highlight/marked-unread state.
- Observed timeline rows enrich Activity rows when available.
- Eventless `RoomUnread` rows are room-level placeholders, not Matrix event references.
- Mark-all-read clears event-backed rows through real event ids and room placeholders through room-id clears only.

- [ ] **Step 2: Check docs**

Run: `rg -n "RoomUnread|RoomSummary|Activity" docs/architecture/state-machine.md docs/superpowers/specs/2026-06-24-timeline-unread-replay-state-machine-design.md`

Expected: both docs describe the same contract.

## Task 7: Final Verification

- [ ] **Step 1: Focused Rust checks**

Run:

```bash
cargo test -p koushi-state --test activity_state
cargo test -p koushi-core --test runtime_activity
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact
```

- [ ] **Step 2: Focused frontend checks**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/timelineStore.test.ts src/components/TimelineView.test.tsx src/backend/browserFakeApi.test.ts
npm --prefix apps/desktop run typecheck
```

- [ ] **Step 3: Review dirty diff**

Run: `git diff --stat` and inspect the full diff for unrelated churn, fixture-only behavior, privacy leaks, synthetic Matrix ids, and source-text-only tests replacing behavioral tests.

- [ ] **Step 4: Commit**

Commit with:

```bash
git add docs/superpowers/specs/2026-06-24-timeline-unread-replay-state-machine-design.md docs/superpowers/plans/2026-06-24-timeline-unread-replay-state-machine-implementation.md docs/architecture/state-machine.md crates apps
git commit -m "fix: unify timeline replay and activity unread ownership"
```
