# DM Conversation Activity Ordering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Rust projection rank DMs by real conversational activity and remove every React comparison that mixes Matrix timestamps with SDK recency stamps.

**Architecture:** The SDK adapter projects a typed `ConversationActivity` independently from preview-oriented `latest_event` and opaque `recency_stamp`. `koushi-state` owns the sole Active comparator and emits final room-list order; TypeScript browser fixtures consume the projected order instead of implementing product sorting.

**Tech Stack:** Rust, matrix-rust-sdk latest-events APIs, serde DTOs, TypeScript/React, Cargo tests, Vitest.

---

### Task 1: Canon and RED state contract

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `crates/koushi-state/src/state/room.rs`
- Modify: `crates/koushi-state/src/state/navigation.rs`
- Modify: `crates/koushi-state/src/state/mod.rs`
- Modify: `crates/koushi-state/src/lib.rs`
- Test: `crates/koushi-state/tests/navigation_state.rs`

- [ ] Add RED fixtures for a recent join-only DM, an older messaged DM, two no-conversation DMs, and equal activity timestamps. Assert messaged DMs precede join-only DMs and no-conversation fallback is display-label then room-id stable.
- [ ] Run `cargo test -p koushi-state --test navigation_state room_list_activity -- --nocapture`; expect the join-only fixture to sort first under the old `latest_event.timestamp_ms` comparator.
- [ ] Add serializable types with redacted/coarse Debug:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationActivitySource {
    Message,
    EncryptedMessage,
    ThreadReply,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConversationActivity {
    pub timestamp_ms: u64,
    pub source: ConversationActivitySource,
}
```

- [ ] Replace `last_activity_ms` with `recency_stamp: Option<u64>` and add `conversation_activity: Option<ConversationActivity>` on `RoomSummary`. Preserve serde defaults for older snapshots.
- [ ] Replace `room_active_sort_timestamp` with one comparator that orders `Some(activity)` before `None`, descending timestamp, then lowercased display label, then room ID. Never inspect `latest_event` or `recency_stamp`.
- [ ] Amend the Room List Filter section of `state-machine.md` with the typed source, ignored event classes, stable fallback, and single-owner rule.
- [ ] Run `cargo test -p koushi-state --test navigation_state`; expect PASS.
- [ ] Commit with `git commit -am 'fix(#251): define conversational activity ordering'`.

### Task 2: SDK event classification and projection

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Test: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-core/src/room.rs`
- Test: `crates/koushi-core/tests/runtime_room_list_sync.rs`

- [ ] Add RED SDK unit fixtures covering ordinary `m.room.message`, undecryptable encrypted message, thread reply, current-user join membership, room state, reaction, edit/replacement, redaction, receipt/typing/presence exclusions, and local sent message.
- [ ] Run the focused SDK tests with `cargo test -p koushi-sdk --lib conversation_activity`; expect missing classification failures.
- [ ] Add SDK-owned mirror types `MatrixConversationActivity` and `MatrixConversationActivitySource` to `MatrixRoomListRoom`; rename raw `last_activity_ms` to `recency_stamp`.
- [ ] In `matrix_room_latest_event_summary`, classify the remote event from raw event type/relation before preview conversion. Count original message-like events, undecryptable encrypted messages, and `m.thread` replies; exclude replacements, annotations, redactions, membership/state, ephemeral signals, and non-message events.
- [ ] For `LocalHasBeenSent`, classify message content and thread relation from the local event content rather than assuming every local event is conversational.
- [ ] Map SDK conversation activity into `koushi_state::ConversationActivity` in the RoomActor normalization path without using preview text.
- [ ] Add restored snapshot and incremental update coverage proving both paths yield the same type and order.
- [ ] Run `cargo test -p koushi-sdk --lib conversation_activity` and `cargo test -p koushi-core --test runtime_room_list_sync`; expect PASS.
- [ ] Commit with `git commit -am 'fix(#251): classify SDK conversational activity'`.

### Task 3: Remove frontend product sorting

**Files:**
- Modify: `apps/desktop/src/domain/types.ts`
- Modify: `apps/desktop/src/components/Shell.tsx`
- Modify: `apps/desktop/src/backend/roomListProjection.ts`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/test/appHarnessMain.tsx`
- Test: `apps/desktop/src/components/Shell.test.tsx`
- Test: `apps/desktop/src/domain/desktopModel.test.ts`

- [ ] Add a RED Shell test whose `snapshot.sidebar.global_dms` order conflicts with room timestamps and assert DOM order follows the sidebar projection exactly.
- [ ] Run `npm --prefix apps/desktop run test -- --run src/components/Shell.test.tsx`; expect the old Shell comparator to reorder rows.
- [ ] Add TypeScript mirrors:

```ts
export type ConversationActivitySource = "message" | "encryptedMessage" | "threadReply";
export interface ConversationActivity {
  timestamp_ms: number;
  source: ConversationActivitySource;
}
```

- [ ] Delete `roomActivityTimestamp` and all `.sort()` calls over Rust-projected sidebar room arrays in `Shell.tsx`.
- [ ] Restrict `computeBrowserRoomListProjection` to the browser fake boundary. Mirror the Rust comparator using only `conversation_activity` for synthetic reducer behavior; add a structure guard that production components do not import it.
- [ ] Update every fake/harness RoomSummary with `recency_stamp` and `conversation_activity` defaults.
- [ ] Run the Shell/desktop-model tests and `npm --prefix apps/desktop run typecheck`; expect PASS.
- [ ] Commit with `git commit -am 'fix(#251): render Rust-owned DM order'`.

### Task 4: DTO, diagnostics, and wire contract

**Files:**
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `crates/koushi-diagnostics/src/lib.rs` or the existing room-list diagnostic producer selected by CodeGraph
- Test: `apps/desktop/src-tauri/src/commands/mod.rs`
- Test: `crates/koushi-state/tests/navigation_state.rs`

- [ ] Add serialization tests proving `conversation_activity` and opaque `recency_stamp` survive snapshot/delta transport and legacy JSON defaults to `None`.
- [ ] Add kind/count-only diagnostics that expose source kind and whether a room has a conversation without IDs, labels, preview, or raw timestamps.
- [ ] Regenerate the checked-in event contract using the repository generator identified by `npm --prefix apps/desktop run test:ipc-contract` output.
- [ ] Run `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml core_event_wire_format_matches_checked_in_contract_artifact`, `npm --prefix apps/desktop run test:ipc-contract`, and `npm --prefix apps/desktop run typecheck`; expect PASS.
- [ ] Commit with `git commit -am 'test(#251): lock activity ordering wire contract'`.

### Task 5: #251 focused completion gate

**Files:**
- Update issue/PR evidence only after gates pass.

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test -p koushi-state --test navigation_state`.
- [ ] Run `cargo test -p koushi-sdk --lib conversation_activity`.
- [ ] Run `cargo test -p koushi-core --test runtime_room_list_sync`.
- [ ] Run `npm --prefix apps/desktop run test -- --run src/components/Shell.test.tsx src/domain/desktopModel.test.ts`.
- [ ] Run `npm --prefix apps/desktop run typecheck` and `git diff --check`.
- [ ] Record the passing commands in the batch worklog; do not close #251 until the final PR is merged.
