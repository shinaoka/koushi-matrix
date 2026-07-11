# Optional Latest-Reply Thread Placement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in Timeline setting that moves one complete thread-root/summary block from its root-event position to its latest-reply activity position, without changing the SDK/React canonical timeline order.

**Architecture:** Keep `TimelineKeyState.items` as the unmodified SDK VectorDiff target. Add a Rust-owned setting and typed latest-reply identity, then derive a separate `TimelineDisplayRow[]` for Room timelines. The projection owns placement, reply-row suppression, and regenerated date dividers; `TimelineView` owns only rendering and reason-tagged layout compensation. A later core hydration path supplies roots that are not in the loaded live window without using backward room pagination.

**Tech Stack:** Rust/serde state and core events, TypeScript/React 19, Vitest, Playwright, Matrix Rust SDK timeline/ThreadListService.

## Global Constraints

- Default is **off**: `TimelineThreadRootOrder::RootEvent` must preserve the current visible ordering exactly.
- `LatestReply` applies only to `TimelineKind::Room`; Thread and Focused timelines retain canonical SDK order.
- Never sort, splice, or otherwise mutate `TimelineKeyState.items`, `itemIndexById`, `itemIdsByTimestamp`, or the VectorDiff application path.
- Move exactly one complete root block: root body, root actions, reactions, receipts, and thread summary. Do not duplicate it.
- The room presentation must not render individual thread-reply rows in addition to the moved root block. Keep reply events available in canonical data; do not restore `hide_threaded_events: true`.
- Placement and date grouping use the latest reply identity/timestamp. The root block's message heading retains the original root timestamp.
- Keep content/root identity separate from activity/latest-reply identity. Actions target the root; visible-range facts target the latest reply.
- A missing old root must use bounded projection hydration only. Never trigger viewport-driven backward pagination or anchor materialization merely to display a latest-reply thread.
- All projection layout corrections must go through TimelineView's reason-tagged scroll-write path and must preserve LiveEdge, Free, and target/jump intent.
- Tests are test-first. Capture RED output before each production-code task and include it in the task report.

---

## File Structure

- `crates/koushi-state/src/state/settings.rs`: typed setting and serde default.
- `crates/koushi-state/tests/settings_state.rs`: default/migration/reducer assertions.
- `crates/koushi-core/src/event.rs`: `ThreadSummaryDto.latest_event_id` and eventual thread-root projection events.
- `crates/koushi-core/src/timeline.rs`: SDK projection of latest event identity and bounded root hydration.
- `apps/desktop/src/domain/coreEvents.ts` and `coreEvents.generated.json`: generated wire contract.
- `apps/desktop/src/domain/types.ts`: settings wire types.
- `apps/desktop/src/domain/timelineDisplayProjection.ts`: pure canonical-to-display projection; no DOM access or state mutation.
- `apps/desktop/src/domain/timelineDisplayProjection.test.ts`: exhaustive pure projection tests.
- `apps/desktop/src/domain/timelineStore.ts`: store only canonical items plus typed root-hydration data; never projected order.
- `apps/desktop/src/components/TimelineView.tsx`: display-row rendering, identity-specific DOM attributes, marker mapping, and layout transactions.
- `apps/desktop/src/components/TimelineView.test.tsx`: DOM/viewport regressions.
- `apps/desktop/src/components/UserSettingsPanel.tsx`: opt-in Timeline setting UI.
- `apps/desktop/src/components/UserSettingsPanel.test.tsx`: setting control contract.
- `apps/desktop/e2e/timeline-thread-latest-placement.spec.ts`: real layout regression coverage.

## Task 1: Add the opt-in setting and latest-reply event identity

**Files:**
- Modify: `crates/koushi-state/src/state/settings.rs`
- Modify: `crates/koushi-state/src/lib.rs`, `crates/koushi-state/src/state/mod.rs`
- Modify: `crates/koushi-state/tests/settings_state.rs`
- Modify: `crates/koushi-core/src/event.rs`, `crates/koushi-core/src/timeline.rs`, `crates/koushi-core/src/runtime.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`, `apps/desktop/src/domain/coreEvents.generated.json`, `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`, `apps/desktop/src/test/appHarnessMain.tsx`, `apps/desktop/src-tauri/tests/golden/frontend_app_state.json`
- Test: `crates/koushi-state/tests/settings_state.rs`, `crates/koushi-core/src/event.rs`, `crates/koushi-core/src/timeline.rs`

**Produces:**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TimelineThreadRootOrder { RootEvent, LatestReply }

pub struct TimelineSettings {
    pub auto_load_older_messages: bool,
    #[serde(default)]
    pub thread_root_order: TimelineThreadRootOrder,
}

pub struct ThreadSummaryDto {
    pub reply_count: u32,
    pub latest_event_id: Option<String>,
    // existing sender/preview/timestamp fields
}
```

- [ ] **Step 1: Write failing Rust setting tests.** Add one test proving `TimelineSettings::default().thread_root_order == RootEvent`; add one reducer test that a settings patch accepts `LatestReply` and a missing serialized field deserializes as `RootEvent`.
- [ ] **Step 2: Run the focused setting test to verify RED.** Run `cargo test -p koushi-state --test settings_state timeline_thread_root_order` and confirm the new type/field is missing.
- [ ] **Step 3: Implement the minimal Rust setting.** Define and re-export `TimelineThreadRootOrder`; add the field with a serde default function returning `RootEvent`; preserve `auto_load_older_messages` defaults; route the regular `SettingsPatch` replacement through `SettingsValues::apply_patch`.
- [ ] **Step 4: Run the focused setting test to verify GREEN.** Re-run the command from Step 2 and confirm all matching tests pass.
- [ ] **Step 5: Write failing wire-contract tests.** Extend the existing `ThreadSummaryDto` serialization test to expect `latest_event_id`, and add a timeline projection test whose SDK latest event ID is preserved.
- [ ] **Step 6: Run the focused Core tests to verify RED.** Run `cargo test -p koushi-core 'thread_summary|timeline_item_serializes_thread_fields'` and confirm failures name the missing ID.
- [ ] **Step 7: Implement the latest-event identity contract.** Populate `ThreadSummaryDto.latest_event_id` from the SDK's ready embedded latest event; update every Rust constructor, runtime fixture, frontend wire type, checked-in generated JSON, browser fake, test harness, and golden fixture in the same change.
- [ ] **Step 8: Run focused Rust and TypeScript contract tests to verify GREEN.** Run `cargo test -p koushi-core 'thread_summary|timeline_item_serializes_thread_fields'` and `npm test -- src/domain/timelineStore.test.ts`; confirm both pass.
- [ ] **Step 9: Commit.** Commit only this cohesive setting/wire-contract slice with message `feat(timeline): add opt-in thread root placement setting`.

## Task 2: Create a pure Room display projection

**Files:**
- Create: `apps/desktop/src/domain/timelineDisplayProjection.ts`
- Create: `apps/desktop/src/domain/timelineDisplayProjection.test.ts`
- Modify: `apps/desktop/src/domain/timelineStore.ts` only if a selector needs canonical root-hydration state; do not alter diff application.

**Consumes:** canonical `TimelineItem[]`, `TimelineKey`, `TimelineThreadRootOrder`.

**Produces:**

```ts
export type TimelineDisplayRow = {
  row_id: string;
  item: TimelineItem;
  kind: "event" | "threadRoot" | "dateDivider";
  content_event_id: string | null;
  activity_event_id: string | null;
  content_timestamp_ms: number | null;
  display_timestamp_ms: number | null;
};

export function projectTimelineDisplayRows(
  canonicalItems: readonly TimelineItem[],
  key: TimelineKey,
  order: TimelineThreadRootOrder,
): TimelineDisplayRow[];
```

- [ ] **Step 1: Write failing projection tests.** Cover: disabled output preserves canonical visible event order; enabled output removes a root from its old slot and inserts one root row at `latest_event_id`; root metadata/action item is unchanged; replies are absent from Room display rows; non-thread order is stable; Thread and Focused keys are untouched; equal timestamps use deterministic original-index then root-ID ordering; canonical input/reference identity is unchanged.
- [ ] **Step 2: Run the projection test to verify RED.** Run `npm test -- src/domain/timelineDisplayProjection.test.ts` and confirm module/export failures.
- [ ] **Step 3: Implement the smallest pure projection.** Build rows without DOM or React; identify roots from `thread_summary`; identify replies from `thread_root`; suppress date-divider synthetic inputs and rebuild date rows from each row's `display_timestamp_ms`; select activity slots by exact `latest_event_id`, never sender/body/timestamp matching; retain the root at its origin if latest identity is unavailable.
- [ ] **Step 4: Extend RED coverage for summary changes.** Add tests for latest reply moving forward, moving backward after redaction/summary update, no replies returning to root position, and date-boundary grouping while the root message timestamp stays original.
- [ ] **Step 5: Implement summary-change behavior and rerun GREEN.** Ensure the selector is deterministic, has no mutation, and passes all projection tests.
- [ ] **Step 6: Commit.** Commit this pure selector and tests with `feat(timeline): project thread roots by latest activity`.

## Task 3: Wire the setting and projection into the desktop UI

**Files:**
- Modify: `apps/desktop/src/components/UserSettingsPanel.tsx`
- Modify: `apps/desktop/src/components/UserSettingsPanel.test.tsx`
- Modify: `apps/desktop/src/i18n/messages.ts` and `messages.test.ts`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`

**Consumes:** `TimelineSettings.thread_root_order` and `projectTimelineDisplayRows`.

**Produces:** Room TimelineView renders projected rows. Every root-block action still receives `content_event_id`; every viewport-visible-range value uses `activity_event_id`.

- [ ] **Step 1: Write failing settings-panel tests.** Assert the Timeline setting is off by default, visible with its helper text, and sends exactly `{ timeline: { ...current, thread_root_order: { kind: "latestReply" } } }` when switched on.
- [ ] **Step 2: Run the focused settings-panel test to verify RED.** Run `npm test -- src/components/UserSettingsPanel.test.tsx` and confirm the control/patch expectation fails.
- [ ] **Step 3: Implement the setting UI and translations.** Add one accessible control under Timeline settings; preserve all other TimelineSettings fields in its patch; use the existing settings update callback; add i18n messages and update message-key tests.
- [ ] **Step 4: Run settings-panel tests to verify GREEN.** Re-run Step 2 and `npm test -- src/i18n/messages.test.ts`.
- [ ] **Step 5: Write failing TimelineView projection tests.** Mount a Room timeline containing a root, reply, and normal event; assert off renders the root at its original position; assert on renders it once at the latest-reply slot, has original body/timestamp and summary, and renders no standalone reply row. Add a Thread timeline assertion proving no reorder.
- [ ] **Step 6: Run the focused TimelineView tests to verify RED.** Run `npm test -- src/components/TimelineView.test.tsx` and confirm current canonical rendering fails the expectations.
- [ ] **Step 7: Integrate display rows without changing canonical store semantics.** Replace only the rendering/height/virtual-range inputs with projected rows; adapt `TimelineItemRow` props to accept explicit content/activity IDs and display/original timestamps; emit `data-row-id`, `data-content-event-id`, and `data-activity-event-id`; retain root actions and thread open behavior.
- [ ] **Step 8: Run focused UI tests to verify GREEN.** Re-run TimelineView and settings panel tests; confirm the canonical timelineStore tests remain unchanged and pass.
- [ ] **Step 9: Commit.** Commit with `feat(desktop): render threads at latest reply position`.

## Task 4: Preserve navigation and viewport intent across projection changes

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/domain/timelineScrollDiagnostics.ts` and tests if adding a reason

**Consumes:** projected rows with content/activity IDs.

**Produces:**

- `visibleEventIds` reports activity IDs for moved roots.
- unread/read-marker matching maps a latest-reply marker to the representing root row.
- a projection revision is compensated as a structural layout mutation.

- [ ] **Step 1: Write failing navigation tests.** Assert a visible moved root reports the latest reply to `observeViewport`, while root context-menu/open-thread callbacks still use the root ID; assert an unread marker for the latest reply renders before the root block.
- [ ] **Step 2: Run the focused TimelineView navigation tests to verify RED.** Run the matching `TimelineView.test.tsx` names and confirm current `data-event-id` behavior fails.
- [ ] **Step 3: Implement identity-aware lookup helpers.** Make visible-range collection, read-marker placement, jump lookup, and row DOM queries select content or activity identity explicitly. Keep read receipt/fully-read behavior canonical and do not send the root ID merely because it is displayed last.
- [ ] **Step 4: Write failing layout tests.** Add deterministic DOM-rect tests for: setting toggle while free-scrolling preserves a non-moving visible row; summary `Set` relocation while at live edge stays at bottom; a moved root is never chosen as the free-scroll anchor; a virtualized list falls back through the height model when the anchor row is unmounted.
- [ ] **Step 5: Run those layout tests to verify RED.** Confirm they fail because current code only captures anchors for prepend batches.
- [ ] **Step 6: Implement a projection layout transaction.** Compare prior and next projected row IDs; on a structural change capture a stable non-moving anchor before render, compensate after layout/measurement through `runWithScrollWriteReason`, coalesce same-frame changes, and cancel on generation/key changes. Add one diagnostics reason only if existing reasons cannot truthfully describe projection compensation.
- [ ] **Step 7: Run focused TimelineView tests to verify GREEN.** Re-run all TimelineView tests and `npm run typecheck`.
- [ ] **Step 8: Commit.** Commit with `fix(timeline): preserve viewport across thread projection changes`.

## Task 5: Hydrate old roots without room backward pagination

**Files:**
- Modify: `crates/koushi-core/src/threads_list.rs`, `crates/koushi-core/src/account.rs`, `crates/koushi-core/src/event.rs`, `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-state/src/action.rs`, `crates/koushi-state/src/state/thread.rs`, `crates/koushi-state/src/reducer/thread.rs`, DTO/state exports as needed
- Modify: `apps/desktop/src/domain/coreEvents.ts`, `apps/desktop/src/domain/timelineStore.ts`, `apps/desktop/src/components/TimelineView.tsx`
- Test: focused Rust actor/state tests, timeline store tests, TimelineView placeholder tests

**Consumes:** active Room timeline key, thread-root ID, a latest-reply activity record.

**Produces:** a bounded `ThreadRootProjection` update keyed by `(room, root_event_id)` that contains the root's renderable timeline item or a typed pending/failed state. It is independent of the visible Threads panel and never modifies canonical Room timeline items.

- [ ] **Step 1: Write failing Rust tests for the projection source.** Cover a recent reply whose root is not in the Room live items: one bounded root request is emitted; a duplicate reply does not emit a second request; success publishes one root snapshot; failure publishes a terminal failed/pending-safe state; no Room `PaginateBackward` or anchor-materialization command is emitted.
- [ ] **Step 2: Run focused Rust tests to verify RED.** Run the new module/state tests and confirm the projection event/state does not exist.
- [ ] **Step 3: Implement a dedicated Rust-owned projection service.** Reuse `ThreadListService`/SDK event-cache data for root metadata, but keep it separate from `ThreadsListState` UI pagination. For an unknown live root, record a provisional activity immediately, dedupe a bounded `room.load_or_fetch_event(root_id, None)` hydration, and emit typed Core timeline projection events. Never set `hide_threaded_events: true` and never use Room backward pagination as hydration.
- [ ] **Step 4: Extend the frontend store and projection tests.** Store root projection snapshots outside canonical `items`; project a stable summary-only row while pending, replace it in place on success, and retain a terminal non-looping placeholder on failure.
- [ ] **Step 5: Run Rust, timeline store, and TimelineView placeholder tests to verify GREEN.** Run the focused suites and confirm no test reports a Room pagination command.
- [ ] **Step 6: Commit.** Commit with `feat(timeline): hydrate thread roots for latest placement`.

## Task 6: Add end-to-end regression coverage and finish

**Files:**
- Create: `apps/desktop/e2e/timeline-thread-latest-placement.spec.ts`
- Modify: the local homeserver QA scenario/binary only when necessary to produce and assert the old-root/latest-reply case.

- [ ] **Step 1: Write the failing Playwright scenario.** Seed an old root, enough normal messages to remove it from initial live coverage, and a latest thread reply. Enable the setting and assert one root/summary block appears at latest activity, no standalone reply row appears, no backward pagination is requested, and opening the block targets the root thread.
- [ ] **Step 2: Run the focused Playwright scenario to verify RED.** Run `npx playwright test e2e/timeline-thread-latest-placement.spec.ts` and confirm the feature assertions fail before final integration.
- [ ] **Step 3: Complete only integration glue required by the failing scenario.** Do not broaden settings or timeline behavior beyond the global constraints.
- [ ] **Step 4: Run focused GREEN verification.** Run the Playwright scenario, `npm test -- src/domain/timelineDisplayProjection.test.ts src/components/TimelineView.test.tsx`, `npm run typecheck`, `npm run lint`, `npm run test:ipc-contract`, and the relevant `cargo test` filters.
- [ ] **Step 5: Run the local homeserver QA.** Extend and run the bounded old-root/latest-reply scenario; record only private-data-safe ordinal/command tokens.
- [ ] **Step 6: Commit.** Commit final QA coverage with `test(timeline): cover latest-reply thread placement`.

## Plan Self-Review

- Every issue #235 acceptance criterion maps to Tasks 1–6.
- No task permits canonical item reordering or default enablement.
- The setting/wire, pure projection, UI projection, viewport mapping, and missing-root service are independently testable/reviewable.
- The plan contains no unresolved work markers or unspecified fallback path.
