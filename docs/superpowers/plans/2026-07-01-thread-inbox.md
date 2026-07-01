# Thread Inbox Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Threads button open a useful room thread inbox, fix the false "No threads" state, and make thread timelines report viewport facts so right-panel reply quotes and thread unread badges update correctly.

**Architecture:** Keep `ThreadListService` as the thread-root source, but explicitly load its first page before showing an empty inbox. Extend viewport reporting from room-only to `TimelineKey`-keyed so room, thread, and focused timelines all address the correct Rust actor. Clear thread attention from the thread actor when the open thread viewport reaches the live edge.

**Tech Stack:** Rust core actors/reducers, Tauri command bridge, React/Vitest UI tests, Matrix Rust SDK `ThreadListService` and `TimelineKind::Thread`.

## Global Constraints

- Do not flatten all thread replies into one global chronological event stream.
- Do not subscribe every thread timeline just to build the inbox.
- Do not change room timeline unread semantics in this step.
- Use failing tests before production changes.
- Keep commits task-sized.

---

## File Structure

- `crates/koushi-core/src/threads_list.rs`: Owns thread inbox subscription and pagination from Matrix SDK `ThreadListService`.
- `apps/desktop/src/components/TimelineView.tsx`: Reports viewport observations from any timeline view.
- `apps/desktop/src/components/TimelineView.test.tsx`: Proves thread timelines report a thread key, not the room key.
- `apps/desktop/src/App.tsx`: Browser/Tauri transport adapter for viewport reports.
- `apps/desktop/src/backend/client.ts`: Desktop API wrapper for the updated IPC argument shape.
- `apps/desktop/src/backend/browserFakeApi.ts`: Fake desktop API signature compatibility.
- `apps/desktop/src/test/harnessMain.tsx`: E2E harness transport signature compatibility.
- `apps/desktop/src-tauri/src/commands/mod.rs`: Builds `ObserveViewport` with a concrete `TimelineKey`.
- `apps/desktop/src-tauri/src/commands/navigation.rs`: Accepts optional timeline key from the webview and normalizes its account key.
- `crates/koushi-core/src/timeline.rs`: Clears thread attention when a thread timeline is observed at bottom.

---

### Task 1: Load the Thread Inbox Initial Page Before Showing Empty

**Files:**
- Modify: `crates/koushi-core/src/threads_list.rs`

**Interfaces:**
- Consumes: `ThreadListService::new(room)`, `ThreadListService::paginate().await`, `ThreadListService::items()`, `ThreadListService::pagination_state()`.
- Produces: `ThreadsListEvent::Opened` and `AppAction::ThreadsListOpened` only after the first page attempt has completed or failed.

- [ ] **Step 1: Write the failing source-order regression test**

Add this test at the bottom of `crates/koushi-core/src/threads_list.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn open_subscription_loads_initial_page_before_emitting_opened() {
        let source = include_str!("threads_list.rs");
        let open_subscription = source
            .split("async fn open_subscription")
            .nth(1)
            .expect("open_subscription body")
            .split("async fn emit_opened")
            .next()
            .expect("open_subscription section");
        let paginate_index = open_subscription
            .find("service.paginate().await")
            .expect("open_subscription must load the first thread page");
        let emit_index = open_subscription
            .find("self.emit_opened")
            .expect("open_subscription must emit opened");

        assert!(
            paginate_index < emit_index,
            "ThreadListService::new() starts empty; paginate before emitting Opened"
        );
    }
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run: `cargo test -p koushi-core threads_list::tests::open_subscription_loads_initial_page_before_emitting_opened`

Expected: FAIL with `open_subscription must load the first thread page`.

- [ ] **Step 3: Implement first-page loading before `emit_opened`**

In `open_subscription`, replace the initial snapshot block:

```rust
let service = Arc::new(ThreadListService::new(room));
let items = service.items();
let projected: Vec<ThreadsListItem> = items.iter().map(project_item).collect();
let end_reached = matches!(
    service.pagination_state(),
    ThreadListPaginationState::Idle { end_reached: true }
);

self.emit_opened(request_id, projected.clone(), end_reached)
    .await;

let (items_tx, mut items_rx) = mpsc::channel(64);
let (pagination_tx, mut pagination_rx) = mpsc::channel(16);
```

with:

```rust
let service = Arc::new(ThreadListService::new(room));
let (_, subscriber) = service.subscribe_to_items_updates();

if let Err(_) = service.paginate().await {
    self.emit_failed(request_id, OperationFailureKind::Sdk).await;
    return None;
}

let items = service.items();
let projected: Vec<ThreadsListItem> = items.iter().map(project_item).collect();
let end_reached = matches!(
    service.pagination_state(),
    ThreadListPaginationState::Idle { end_reached: true }
);

self.emit_opened(request_id, projected.clone(), end_reached)
    .await;

let (items_tx, mut items_rx) = mpsc::channel(64);
let (pagination_tx, mut pagination_rx) = mpsc::channel(16);
```

Then update the items relay to use the already-created subscriber:

```rust
let items_relay_handle = {
    let service = Arc::clone(&service);
    let mut subscriber = subscriber;
    executor::spawn(async move {
        loop {
            match subscriber.next().await {
                Some(_) => {
                    let _ = items_tx.try_send(service.items());
                }
                None => break,
            }
        }
    })
};
```

- [ ] **Step 4: Run the task tests**

Run: `cargo test -p koushi-core threads_list::tests::open_subscription_loads_initial_page_before_emitting_opened`

Expected: PASS.

Run: `cargo test -p koushi-core --lib`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/koushi-core/src/threads_list.rs
git commit -m "Load initial thread inbox page"
```

---

### Task 2: Route Viewport Observations by TimelineKey

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/harnessMain.tsx`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/commands/navigation.rs`

**Interfaces:**
- Consumes: existing `TimelineKey` from every `TimelineView`.
- Produces: `observeViewport(timelineKey, roomId, firstVisibleEventId, lastVisibleEventId, atBottom)` on the TS side and `build_observe_timeline_viewport_command(request_id, account_key, timeline_key, ...)` on the Rust bridge side.

- [ ] **Step 1: Write the failing React test**

In `apps/desktop/src/components/TimelineView.test.tsx`, add:

```tsx
it("reports viewport observations with the thread timeline key", async () => {
  vi.useFakeTimers();
  const threadKey = threadTimelineKey(
    "@me:example.invalid",
    "!room:example.invalid",
    "$root:example.invalid"
  );
  const observeViewport = vi.fn().mockResolvedValue(undefined);
  const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
    InitialItems: {
      request_id: null,
      key: threadKey,
      generation: 1,
      items: [message("$reply:example.invalid", "Thread reply")]
    }
  });

  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    const itemId = this.getAttribute("data-item-id");
    const top = itemId ? 20 : 0;
    const height = itemId ? 40 : 240;
    return {
      x: 0,
      y: top,
      top,
      left: 0,
      right: 480,
      width: 480,
      height,
      bottom: top + height,
      toJSON: () => ({})
    } as DOMRect;
  });

  render(
    <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={baseTransport({ observeViewport })}
        onReply={vi.fn()}
      />
    </TimelineStoreContext.Provider>
  );

  await vi.runOnlyPendingTimersAsync();

  expect(observeViewport).toHaveBeenCalledWith(
    threadKey,
    "!room:example.invalid",
    "$reply:example.invalid",
    "$reply:example.invalid",
    expect.any(Boolean)
  );
  vi.useRealTimers();
});
```

If `threadTimelineKey` is not imported in the test file, import it from `../domain/coreEvents`.

- [ ] **Step 2: Run the React test and verify it fails**

Run: `npm --prefix apps/desktop test -- src/components/TimelineView.test.tsx --testNamePattern "reports viewport observations with the thread timeline key"`

Expected: FAIL because `observeViewport` currently receives room-id-only arguments or is not called for thread timelines.

- [ ] **Step 3: Update the TS transport interface**

In `apps/desktop/src/components/TimelineView.tsx`, change:

```ts
observeViewport?(
  roomId: string,
  firstVisibleEventId: string | null,
  lastVisibleEventId: string | null,
  atBottom: boolean
): Promise<void>;
```

to:

```ts
observeViewport?(
  timelineKey: TimelineKey,
  roomId: string,
  firstVisibleEventId: string | null,
  lastVisibleEventId: string | null,
  atBottom: boolean
): Promise<void>;
```

- [ ] **Step 4: Update `TimelineView` viewport reporting**

In `reportViewportObservation`, replace the room-only guard:

```ts
if (!transport.observeViewport || roomTimelineRoomId !== roomId) {
  return;
}
```

with:

```ts
if (!transport.observeViewport) {
  return;
}
```

Then restrict read-marker side effects to the room timeline:

```ts
const isRoomTimeline = "Room" in timelineKey.kind && roomTimelineRoomId === roomId;
if (atBottom && latestReadableEventId && isRoomTimeline) {
  sendReadSignalsForEvent(latestReadableEventId);
}
```

Update the signature key:

```ts
const signature = [
  JSON.stringify(timelineKey),
  roomId,
  visible.firstVisibleEventId ?? "",
  visible.lastVisibleEventId ?? "",
  atBottom ? "bottom" : "not-bottom"
].join("\u0000");
```

Update the transport call:

```ts
void transport
  .observeViewport(
    timelineKey,
    roomId,
    visible.firstVisibleEventId,
    visible.lastVisibleEventId,
    atBottom
  )
  .catch(() => undefined);
```

- [ ] **Step 5: Update App and API adapters**

In `apps/desktop/src/App.tsx`, change:

```ts
async observeViewport(
  roomId: string,
  firstVisibleEventId: string | null,
  lastVisibleEventId: string | null,
  atBottom: boolean
) {
  await invoke("observe_timeline_viewport", {
    roomId,
    firstVisibleEventId,
    lastVisibleEventId,
    atBottom
  });
}
```

to:

```ts
async observeViewport(
  timelineKey: TimelineKey,
  roomId: string,
  firstVisibleEventId: string | null,
  lastVisibleEventId: string | null,
  atBottom: boolean
) {
  await invoke("observe_timeline_viewport", {
    roomId,
    timelineKey,
    firstVisibleEventId,
    lastVisibleEventId,
    atBottom
  });
}
```

Make the same signature change in:

```ts
apps/desktop/src/backend/client.ts
apps/desktop/src/backend/browserFakeApi.ts
apps/desktop/src/test/harnessMain.tsx
```

For no-op fake implementations, accept `timelineKey` and ignore it:

```ts
async observeViewport(
  _timelineKey: TimelineKey,
  _roomId: string,
  _firstVisibleEventId: string | null,
  _lastVisibleEventId: string | null,
  _atBottom: boolean
): Promise<void> {
  return undefined;
}
```

- [ ] **Step 6: Write the failing Tauri command test**

In `apps/desktop/src-tauri/src/commands/mod.rs`, add or update a test near `observe_timeline_viewport_command_routes_viewport_facts_only`:

```rust
#[test]
fn observe_timeline_viewport_command_can_route_thread_key() {
    let account_key = AccountKey("@me:example.invalid".to_owned());
    let command = build_observe_timeline_viewport_command(
        fake_request_id(1),
        account_key.clone(),
        TimelineKey {
            account_key: AccountKey("@stale:example.invalid".to_owned()),
            kind: TimelineKind::Thread {
                room_id: "!room:example.invalid".to_owned(),
                root_event_id: "$root:example.invalid".to_owned(),
            },
        },
        Some("$first:example.invalid".to_owned()),
        Some("$last:example.invalid".to_owned()),
        true,
    );

    let CoreCommand::Timeline(TimelineCommand::ObserveViewport {
        key,
        observation,
        ..
    }) = command
    else {
        panic!("expected observe viewport timeline command");
    };

    assert_eq!(key.account_key, account_key);
    assert!(matches!(
        key.kind,
        TimelineKind::Thread {
            ref room_id,
            ref root_event_id
        } if room_id == "!room:example.invalid"
            && root_event_id == "$root:example.invalid"
    ));
    assert_eq!(
        observation.first_visible_event_id.as_deref(),
        Some("$first:example.invalid")
    );
    assert_eq!(
        observation.last_visible_event_id.as_deref(),
        Some("$last:example.invalid")
    );
    assert!(observation.at_bottom);
}
```

- [ ] **Step 7: Run the Tauri command test and verify it fails**

Run: `cargo test -p koushi-desktop observe_timeline_viewport_command_can_route_thread_key`

Expected: FAIL because `build_observe_timeline_viewport_command` currently accepts `room_id`, not `TimelineKey`.

- [ ] **Step 8: Update the Tauri command builder**

In `apps/desktop/src-tauri/src/commands/mod.rs`, change:

```rust
pub(crate) fn build_observe_timeline_viewport_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    at_bottom: bool,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::ObserveViewport {
        request_id,
        key: build_timeline_key(account_key, room_id),
        observation: TimelineViewportObservation {
            first_visible_event_id,
            last_visible_event_id,
            at_bottom,
        },
    })
}
```

to:

```rust
pub(crate) fn build_observe_timeline_viewport_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    timeline_key: TimelineKey,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    at_bottom: bool,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::ObserveViewport {
        request_id,
        key: TimelineKey {
            account_key,
            kind: timeline_key.kind,
        },
        observation: TimelineViewportObservation {
            first_visible_event_id,
            last_visible_event_id,
            at_bottom,
        },
    })
}
```

- [ ] **Step 9: Update the Tauri command entrypoint**

In `apps/desktop/src-tauri/src/commands/navigation.rs`, change the command signature to:

```rust
pub async fn observe_timeline_viewport(
    room_id: String,
    timeline_key: Option<TimelineKey>,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    at_bottom: bool,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let key = timeline_key.unwrap_or_else(|| TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Room { room_id },
    });
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_observe_timeline_viewport_command(
            request_id,
            account_key,
            key,
            first_visible_event_id,
            last_visible_event_id,
            at_bottom,
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(())
}
```

Add imports if needed:

```rust
use koushi_core::{TimelineKey, TimelineKind};
```

- [ ] **Step 10: Run the task tests**

Run:

```bash
npm --prefix apps/desktop test -- src/components/TimelineView.test.tsx --testNamePattern "reports viewport observations with the thread timeline key"
cargo test -p koushi-desktop observe_timeline_viewport_command_can_route_thread_key
npm --prefix apps/desktop run typecheck
```

Expected: all PASS.

- [ ] **Step 11: Commit**

```bash
git add apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx apps/desktop/src/App.tsx apps/desktop/src/backend/client.ts apps/desktop/src/backend/browserFakeApi.ts apps/desktop/src/test/harnessMain.tsx apps/desktop/src-tauri/src/commands/mod.rs apps/desktop/src-tauri/src/commands/navigation.rs
git commit -m "Route viewport observations by timeline key"
```

---

### Task 3: Clear Thread Attention When the Open Thread Is Read

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`

**Interfaces:**
- Consumes: `TimelineActorMessage::ObserveViewport`, `TimelineViewportObservation.at_bottom`, `ThreadAttentionCounters`.
- Produces: `AppAction::ThreadAttentionUpdated` with zero counts for the currently tracked thread.

- [ ] **Step 1: Write the failing pure-function test**

In `crates/koushi-core/src/timeline.rs`, add near `thread_attention_action_counts_remote_live_thread_messages_only`:

```rust
#[test]
fn thread_attention_clear_action_resets_counts_at_thread_live_edge() {
    let key = TimelineKey {
        account_key: AccountKey("@me:test".to_owned()),
        kind: TimelineKind::Thread {
            room_id: "!room:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
        },
    };
    let mut counts = ThreadAttentionCounters {
        notification_count: 2,
        highlight_count: 1,
        live_event_marker_count: 2,
    };

    let action = thread_attention_clear_action_from_viewport(
        &mut counts,
        &key,
        &TimelineViewportObservation {
            first_visible_event_id: Some("$a:test".to_owned()),
            last_visible_event_id: Some("$b:test".to_owned()),
            at_bottom: true,
        },
    );

    assert_eq!(counts, ThreadAttentionCounters::default());
    assert_eq!(
        action,
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!room:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0,
        })
    );
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run: `cargo test -p koushi-core thread_attention_clear_action_resets_counts_at_thread_live_edge`

Expected: FAIL because `thread_attention_clear_action_from_viewport` is not defined.

- [ ] **Step 3: Implement the clear helper**

Add near `thread_attention_action_from_timeline_diffs`:

```rust
fn thread_attention_clear_action_from_viewport(
    counts: &mut ThreadAttentionCounters,
    key: &TimelineKey,
    observation: &TimelineViewportObservation,
) -> Option<AppAction> {
    if !observation.at_bottom {
        return None;
    }
    if *counts == ThreadAttentionCounters::default() {
        return None;
    }
    let TimelineKind::Thread {
        room_id,
        root_event_id,
    } = &key.kind
    else {
        return None;
    };

    *counts = ThreadAttentionCounters::default();
    Some(AppAction::ThreadAttentionUpdated {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
        notification_count: 0,
        highlight_count: 0,
        live_event_marker_count: 0,
    })
}
```

- [ ] **Step 4: Call the helper from `ObserveViewport`**

In `TimelineActor::handle_msg`, update the observe branch from:

```rust
TimelineActorMessage::ObserveViewport { observation } => {
    self.viewport_observation = observation;
    self.maybe_fetch_visible_reply_details();
    self.emit_navigation_if_changed();
}
```

to:

```rust
TimelineActorMessage::ObserveViewport { observation } => {
    self.viewport_observation = observation;
    self.maybe_fetch_visible_reply_details();
    if let Some(action) = thread_attention_clear_action_from_viewport(
        &mut self.thread_attention_counts,
        &self.key,
        &self.viewport_observation,
    ) {
        let _ = self.action_tx.try_send(vec![action]);
    }
    self.emit_navigation_if_changed();
}
```

- [ ] **Step 5: Run the task tests**

Run:

```bash
cargo test -p koushi-core thread_attention_clear_action_resets_counts_at_thread_live_edge
cargo test -p koushi-core --lib
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/koushi-core/src/timeline.rs
git commit -m "Clear thread attention at live edge"
```

---

### Task 4: Full Verification

**Files:**
- No new files.

**Interfaces:**
- Consumes: all previous task outputs.
- Produces: verified branch state ready for manual app testing.

- [ ] **Step 1: Run formatting and Rust tests**

```bash
cargo fmt --check
cargo test -p koushi-core --lib
cargo test -p koushi-desktop observe_timeline_viewport
```

Expected: all PASS.

- [ ] **Step 2: Run desktop frontend checks**

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
```

Expected: all PASS.

- [ ] **Step 3: Run targeted e2e smoke**

```bash
cd apps/desktop
npx playwright test e2e/basic-operations.spec.ts -g "add reaction picker"
```

Expected: PASS.

- [ ] **Step 4: Manual verification checklist**

Launch the app and verify:

```text
1. Open a room with known threaded messages.
2. Click Threads in the left room navigation.
3. The right panel shows thread rows instead of "No threads".
4. Click a row.
5. The individual thread timeline opens in the right panel.
6. A visible reply quote whose original message was missing updates after the thread panel settles.
7. If the thread is at live edge, the left Threads badge does not keep showing unread for the currently open thread.
```

- [ ] **Step 5: Commit any final test-only or verification fixes**

If no code changed during verification, do not create an empty commit. If verification required fixes:

```bash
git status --short
git add crates/koushi-core/src/threads_list.rs crates/koushi-core/src/timeline.rs apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx apps/desktop/src/App.tsx apps/desktop/src/backend/client.ts apps/desktop/src/backend/browserFakeApi.ts apps/desktop/src/test/harnessMain.tsx apps/desktop/src-tauri/src/commands/mod.rs apps/desktop/src-tauri/src/commands/navigation.rs
git commit -m "Stabilize thread inbox verification"
```

---

## Self-Review

- Spec coverage: Task 1 fixes false "No threads"; Task 2 enables thread-keyed right-panel timeline behavior and reply quote detail fetches; Task 3 clears the misleading current-thread badge; Task 4 verifies the full flow.
- Placeholder scan: no banned placeholder tokens or unspecified edge handling remains.
- Type consistency: TS side uses `TimelineKey`; Rust bridge normalizes account key while preserving `TimelineKind`; thread attention reset reuses existing `AppAction::ThreadAttentionUpdated`.
