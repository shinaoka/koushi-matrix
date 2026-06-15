# Message Interactions Phase A1 Design

## Goal

Implement the first Rust-owned slice of issue #19: reply quote projection and
pinned-event state. This slice creates the state-machine and headless contract
needed before any GUI work for quoted replies or pin/unpin controls.

The umbrella rule applies: Phase A is Rust/core/headless first. React must
render projected state and dispatch typed commands only.

## Scope

Phase A1 includes:

- `TimelineItem.reply_quote`, resolved by Rust for events that reply to another
  event.
- Room-level pinned-event state, projected from `m.pinned_events`.
- Typed commands `PinEvent { room_id, event_id }` and
  `UnpinEvent { room_id, event_id }`.
- A headless core QA proof with private-data-free tokens:
  `reply_quote=ok`, `pin_event=ok`, `unpin_event=ok`, and
  `pinned_state=ok`.

Phase A1 does not include:

- URL/link preview fetch, cache, network policy, or rendering.
- Forward-message semantics.
- Permalink parsing/building.
- View-source or copy-message actions.
- Phase B visual components. Those consume this contract later.

## Current State

The repo already owns reply sending and reply relation state:

- `TimelineCommand::SendReply` sends a Matrix reply through Rust.
- `ComposerMode::Reply` and `PendingComposerSendKind::Reply` keep reply
  composer state in the reducer.
- `TimelineItem.in_reply_to_event_id` exposes the target event id.
- Headless core QA already proves true reply relation with `reply=ok`.

The missing piece is the renderable quote source. Today React can see that an
item replies to an event, but it cannot render a Rust-resolved quote block with
sender/body/redacted/missing state.

Pinned events have SDK support through the Matrix room APIs, but the app has no
first-party state, DTO, command, or QA token for pins.

## Reply Quote Model

Add a Rust-owned DTO on `TimelineItem`:

```rust
pub struct ReplyQuote {
    pub event_id: String,
    pub sender: Option<String>,
    pub body_preview: Option<String>,
    pub state: ReplyQuoteState,
}

pub enum ReplyQuoteState {
    Ready,
    Redacted,
    Missing,
    Unsupported,
}
```

The TypeScript wire shape mirrors this as an optional `reply_quote` field.

Rules:

- Rust sets `reply_quote = None` when the item is not a reply.
- Rust sets `Missing` when the target cannot be resolved from available
  timeline context.
- Rust sets `Redacted` when the target is known but redacted.
- Rust sets `Unsupported` when the target exists but has no renderable message
  body or media summary.
- `body_preview` is a short display preview, not full source HTML.
- DTO debug output and QA tokens must not print message bodies or Matrix ids.

Phase A1 may resolve quote sources from currently available timeline items and
SDK reply details. It does not need to paginate or fetch arbitrary historical
events to repair missing quotes. A future slice can add explicit backfill if
needed.

## Pinned Events Model

Add Rust-owned per-room interaction state to `AppState`:

```rust
pub struct AppState {
    // existing fields...
    pub room_interactions: BTreeMap<String, RoomInteractionState>,
}

pub struct RoomInteractionState {
    pub pinned_events: Vec<PinnedEvent>,
    pub pin_operation: PinOperationState,
}

pub struct PinnedEvent {
    pub event_id: String,
    pub sender: Option<String>,
    pub body_preview: Option<String>,
    pub redacted: bool,
}

pub enum PinOperationState {
    Idle,
    Pending { request_id: RequestId, room_id: String, event_id: String, op: PinOp },
    Failed { room_id: String, event_id: String, op: PinOp, recoverable: bool },
}
```

The map is keyed by room id. It is cleared on logout/account switch and entries
are replaced by room-state projections, not by React actions. The Tauri
frontend snapshot mirrors this as `state.room_interactions` so Phase B can
render a pinned banner/list from Rust-owned data.

Rules:

- Pinned event order follows the `m.pinned_events` content order.
- Unknown pinned event ids may appear as previewless `PinnedEvent` entries.
- Pin/unpin commands are request-correlated and guarded by ready session,
  selected/known room, non-empty event id, and no conflicting pin operation for
  that room.
- Success is driven by the resulting room state projection, not by React-local
  optimism.
- Failure records a recoverable operation error without exposing raw SDK
  errors, event ids, room ids, or message bodies in QA output.

## Commands And Effects

Add typed core commands:

- `RoomCommand::PinEvent { request_id, room_id, event_id }`
- `RoomCommand::UnpinEvent { request_id, room_id, event_id }`

The actor calls the SDK room pin/unpin APIs. It then waits for or triggers the
room state projection that updates `RoomInteractionState.pinned_events`.

Tauri commands:

- `pin_event(room_id, event_id)`
- `unpin_event(room_id, event_id)`

Frontend transport later calls these commands, but Phase A1 only needs the IPC
contract and Rust tests.

## Headless QA

Extend the local headless core QA with a message-interactions stage.

Scenario:

1. Create a disposable room with two accounts.
2. Account A sends a source message.
3. Account B sends a reply to that source.
4. Account A observes the reply item with `reply_quote.state = Ready`.
5. Account A pins the source event and observes Rust-owned pinned state.
6. Account A unpins the source event and observes the pinned list empty.

Private-data-free tokens:

- `reply_quote=ok`
- `pin_event=ok`
- `pinned_state=ok`
- `unpin_event=ok`

The lane must not print room ids, event ids, user ids, message bodies, raw SDK
errors, or permalink strings.

## Phase B Contract

Phase B can begin only after Phase A1 lands. The GUI consumes:

- `TimelineItem.reply_quote` for quote block rendering.
- `RoomInteractionState.pinned_events` for pinned banner/list rendering.
- `pin_event` / `unpin_event` for visible controls.

Browser-headless tests should seed Rust-shaped snapshots/CoreEvents, click
Pin/Unpin controls, assert typed IPC dispatch, and then push Rust-shaped state
updates. React must not synthesize quote bodies or pinned state.

## Verification

Phase A1 verification:

- Rust state/reducer tests for pin operation guards and request correlation.
- Core command debug tests redacting event ids, room ids, and message bodies.
- Tauri command builder and invoke contract tests for `pin_event` and
  `unpin_event`.
- CoreEvent/frontend snapshot serialization contract tests for `reply_quote`
  and pinned state.
- Headless core local QA tokens listed above.
- Docs and `AGENTS.md` updated with the reply quote and pinned-event lessons.

Phase B verification comes later:

- `basic-operations.spec.ts` style GUI contract for quoted reply rendering and
  pin/unpin controls.
- i18n labels and design-token styling.
- No Matrix identifiers in browser-headless output.

## Non-Goals

- No link preview fetch model in Phase A1. Link previews need separate network,
  cache, security, and privacy rules.
- No permalink builder or parser in Phase A1.
- No forward-message command in Phase A1.
- No native GUI smoke for Phase A1; local headless core QA is the primary gate.

## Open Follow-Ups Within Issue #19

After Phase A1 and its Phase B GUI slice land, remaining #19 slices are:

1. Permalink builder plus copy/open actions.
2. Forward-message command and GUI flow.
3. Link preview fetch/projection model and preview cards.
4. View-source action backed by a redacted/safe event-source DTO.
