# Scheduled Send Reliable Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep a locally scheduled message until Matrix confirms it, even when its timeline is not open.

**Architecture:** A random Matrix ID makes every persisted scheduled ID unique across restarts. `AppActor` marks a due local item as transiently dispatching and sends an account-level command carrying the origin session key. `AccountActor` sends it without a UI timeline actor, using a deterministic Matrix transaction ID. Success removes the item; failure clears the transient marker and sets a bounded retry time.

**Tech Stack:** Rust, Tokio actors, matrix-sdk room sends, koushi-state reducer tests.

## Global Constraints

- Preserve server-delayed-events behavior.
- Do not expose the transient dispatch marker in persisted or frontend scheduled-send data.
- Retry with a stable Matrix transaction ID so uncertain network outcomes cannot duplicate the message.
- Reject a dispatch when its origin session is no longer the active account.

---

### Task 1: Prove the data-loss regression

**Files:**
- Modify: `crates/koushi-core/tests/runtime_scheduled_send.rs`

**Interfaces:**
- Consumes: `CoreRuntime`, `AppCommand::ScheduleSend`, `wait_for_state_for`.
- Produces: a test that requires an unavailable local delivery attempt to retain and reschedule its item.

- [x] **Step 1: Write the failing test**

```rust
let snapshot = wait_for_state_for(&mut conn, Duration::from_secs(3), |state| {
    state.scheduled_sends.items.values().any(|item| {
        item.body == "retry instead of drop" && item.send_at_ms > send_at_ms
    })
})
.await;
assert_eq!(snapshot.scheduled_sends.items.len(), 1);
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p koushi-core --test runtime_scheduled_send local_fallback_scheduled_send_is_retained_when_delivery_cannot_start`

Expected: FAIL because the current timer removes the item before it can reach a timeline actor.

- [x] **Step 3: Commit**

```bash
git add crates/koushi-core/tests/runtime_scheduled_send.rs
git commit -m "test: cover local scheduled send retry"
```

### Task 2: Dispatch directly through the SDK and preserve failures

**Files:**
- Modify: `crates/koushi-state/src/state/timeline.rs`
- Modify: `crates/koushi-state/src/action.rs`
- Modify: `crates/koushi-state/src/reducer/mod.rs`
- Modify: `crates/koushi-state/src/reducer/timeline.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/src/timeline.rs`

**Interfaces:**
- Consumes: `ScheduledSendItem`, `AppAction`, `AccountMessage`, `Room::send()`.
- Produces: `ScheduledSendDispatchStarted` and `ScheduledSendDispatchFailed` actions plus `AccountMessage::DispatchLocalScheduledSend`.

- [x] **Step 1: Add transient dispatch state and reducer actions**

```rust
#[serde(skip)]
pub is_dispatching: bool,
```

The due-item selector excludes dispatching items. A start action sets the marker; a failure action clears it and updates `send_at_ms` to the supplied retry timestamp.

- [x] **Step 2: Route local due items to AccountActor**

```rust
AccountMessage::DispatchLocalScheduledSend {
    request_id,
    origin_session_key,
    scheduled_id: item.scheduled_id,
    room_id: item.room_id,
    body: item.body,
}
```

Start the dispatch reducer action before forwarding. If the actor channel is closed, reset the item to its retry time.

- [x] **Step 3: Send the message without a timeline actor**

```rust
room.send(content)
    .with_transaction_id(OwnedTransactionId::from(scheduled_send_transaction_id(&scheduled_id)))
    .await
```

On `Ok`, emit `ScheduledSendDispatched`; on error or missing session/room, emit `ScheduledSendDispatchFailed` and the matching core failure event.

- [x] **Step 4: Run the focused regression test**

Run: `cargo test -p koushi-core --test runtime_scheduled_send local_fallback_scheduled_send_is_retained_when_delivery_cannot_start`

Expected: PASS.

### Task 3: Verify the scheduled-send suite

**Files:**
- Verify: `crates/koushi-core/tests/runtime_scheduled_send.rs`

- [x] **Step 1: Run the complete focused suite**

Run: `cargo test -p koushi-core --test runtime_scheduled_send`

Expected: all scheduled-send tests pass.

- [x] **Step 2: Inspect the final diff**

Run: `git diff --check && git diff --stat`

Expected: no whitespace errors and only the files named above.
