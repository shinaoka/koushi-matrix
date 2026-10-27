# MCP message actions and get_message (#1628)

The mandatory contract — six tools, their arguments and results, the
`message_actions` grant, the failure matrix, the privacy rules, and the
acceptance criteria — is in [#1628](https://github.com/shinaoka/koushi-matrix/issues/1628).
This plan records the implementation sequence, where each piece lives, and the
deliberate limits of this phase. It does not restate the contract.

Related: [#1520](2026-10-18-issue1520-mcp-bridge.md) (bridge),
[#1524](2026-10-18-mcp-ui-isolation.md) (no GUI state),
[#1536](2026-10-19-issue1536-mcp-approval.md) (grants and approval),
[#1482](2026-10-22-issue1482-timeline-mutations.md) (mutation ownership),
[#1551](../agents/history.md) (typed results).

## Why this is not additive schema work

`TimelineCommand::{EditText, Redact, SendReaction, RedactReaction}` are
mutations routed to a resident `TimelineActor`; without a subscribed timeline
they fail `NotSubscribed`, and mounting one per call would create presentation
residency and GUI state (#1524). Thread replies exist only through the composer
send path. `read_room_messages` pages a room newest-first, so a single known
event id cannot be fetched directly.

## Design decisions

1. **Direct operations, one owner.** The five mutations run as
   session-owned direct operations in `koushi-core`, next to
   `direct_operations.rs`, `timeline/direct_room_messages.rs`, and the direct
   search/send paths. They take the account session, resolve the joined room
   themselves, and return a typed outcome on a per-request reply channel. They
   never reduce an `AppAction`, never create a pending GUI row, never mount a
   `TimelineActor`, and never touch selection, scroll, unread state, or read
   markers. No `CoreEvent` is emitted for any of them: a thread reply is
   inserted straight into the SDK room send queue and its terminal is observed
   on the call's own channel, so `SendCompleted` never reaches the WebView.
2. **No second edit/reaction/relation semantics.** Every decision that exists
   today is called, not copied: which msgtype a target carries and how its
   edited document is built (`item_projection::edit_target_msgtype`,
   `edited_document_content_for_edit_target`, `msgtype_carries_editable_caption`),
   whether a reaction toggle is valid (`validate_send_reaction`,
   `validate_redact_reaction`, `classify_reaction_error`), the media payload
   preservation (`Room::make_edit_event`), the `m.thread` content shape
   (`outbound_send::make_offline_thread_content` and the composer content
   builder), and the send queue path. Those helpers are widened narrowly to
   `pub(crate)`; no new content builder, relation builder, or edit policy is
   added. Edit validity is the SDK's own replacement validation, not a
timestamps-or-sender check written here.
3. **`get_message` is one bounded read plus one bounded relations page.** It
   resolves the room from the session, requires `Joined`, reads the single event
   with a bounded timeout, and fetches exactly one relations page (at most 256
   relations). It does not use `load_or_fetch_event_with_relations`, which pages
   to exhaustion and swallows relation-fetch errors. The page reports
   `relations_truncated` when more remain; an undecryptable event is a success
   with `body: null` and `content_available: false`.
4. **Thread replies need a visible message-like root, not an own one.** The
   target must be a thread root that is not itself a thread reply; answering
   another user's thread is the point of the tool, so authorship is checked only
   for `edit_message` and `delete_message`. The reply reuses the composer's
   content builder and `outbound_send::make_offline_thread_content`, is inserted
   into the SDK room send queue directly (not through `TimelineCommand::SendText`),
   and its completion is observed on the call's own channel. Nothing is
   registered as a pending send, no resident actor is refreshed, and no
   `CoreEvent` is emitted; the GUI sees the reply through the normal sync echo.
   A `SendText` submission was rejected during review because it registers a
   pending-send projection, refreshes resident actors, and emits `SendCompleted`
   to the WebView, which the contract forbids.
5. **`client_request_id` dedupes in the MCP host, because the SDK queue
   cannot.** `send_queue().send()` mints its own transaction id
   (`Queue::push` creates a fresh one), so passing the caller's id as a label
   does not prevent a second event. A bounded, in-memory ledger in
   `koushi-mcp-host` maps (client, account, target, kind, `client_request_id`)
   to the in-flight or settled tool result and answers a repeat with that result
   instead of submitting again. It holds ids and the tool's own result only, is
   bounded in entries and age, and is process-local, so a retry after a bridge
   restart may act again (the tool descriptions say so). `edit_message` and
   `send_thread_reply` use it. Core's own ledgers (`accepted_submissions`,
   `sent_event_txns`, the completion coordinator) were checked and answer
   different questions.
6. **Admission is fenced, and revocation is re-read at submission.** Each
   action captures the account's runtime session identity before it asks for
   consent and re-checks it at admission, after target resolution, and at the
   submission point; a grant revoked before submission fails `not_permitted`, a
   signed-out, locked, or replaced session fails `account_unavailable`, and an event already submitted is never
   rolled back. A caller disconnect drops the
   response and the outcome stays unknown to the caller, which the tool
   description states.
7. **Reaction set semantics are serialized, not raced.** The decision (does an
   own reaction with this key exist?) and the submission run in one per-account
   lane keyed by (room, target, key), so two concurrent calls cannot both add
   one. The required own-reaction set comes from the same single bounded
   relations page; when the page cannot establish it (truncated, or an
   undecryptable reaction event), the call fails `internal` with reason
   `relations_unavailable` instead of guessing or creating a duplicate.
8. **Bounded, per-account capped.** Every direct action runs in a bounded
   session-owned task with a fixed deadline and a small per-account concurrency
   cap (`busy` on overflow).
9. **Grant schema 3.** `Need::MessageActions` and
   `AccountGrant::message_actions` are added; `connections.json` moves to
   schema 3 while schema 1 and 2 stay readable with a missing flag meaning
   `false`. `list_accounts.permissions` gains the flag, the settings row shows
   it, and the approval copy is new text in every supported locale (including an
   update to the existing `send` copy so it names thread replies).
10. **Canon and the QA contract land with the code, canon first.**
    `docs/architecture/overview.md` and `docs/agents/state-ownership.md` are
    amended before the implementation slices, and the scenario's token contract
    is added to `docs/agents/qa-lanes.md` in the same change that registers it.

## Operation lifecycle

No reducer-visible state is added, so this table is the record (it does not
go into `state-machine.md`):

| Stage | Owner | Entry | Terminal outcome | Cleanup |
| --- | --- | --- | --- | --- |
| admission | host handler | arguments validated, grant read | `not_permitted`, `consent_required`, `busy` | nothing created |
| consent | `koushi-mcp-host::consent` | grant missing | `consent_required` after 40 s, `not_permitted` on deny | pending key released |
| session fence | Core direct operation | runtime session identity captured before consent | `account_unavailable` on sign-out/lock/replacement | task dropped |
| target resolution | Core direct operation | joined room resolved, bounded event + relations read | `room_not_found`, `event_not_found`, `unsupported`, `not_permitted`, `relations_unavailable` | deadline timer released |
| submission | Core direct operation | grant re-read, fence re-checked, ledger entry created | `busy`, `not_permitted`, `internal` | ledger entry kept until TTL |
| settle | Core direct operation | SDK accepted the event | tool success with event/transaction ids | ledger records the event id |
| settle failure | Core direct operation | SDK/queue/timeout failure | `internal` with a private-safe message | ledger entry kept until TTL |
| caller disconnect | host | socket closes | no response; a submitted event is not rolled back | reply channel dropped |
| duplicate completion | Core direct operation | a late SDK completion for a settled entry | ignored | ledger entry expires |

## Implementation slices

Each slice lands tests with it; the slices are ordered so every later one
compiles against a settled earlier one.

1. **Core direct operations** (`crates/koushi-core`): one
   `DirectMessageRequest`/`DirectMessageResponse` pair on the existing ingress
   (`AccountMessage::DirectMessageAction` / `CoreCommandEnvelope::DirectMessageAction`),
   the session-owned execution with deadline, per-account cap and the
   per-reaction-key lane, `CoreConnection::direct_message_action`, the typed
   `DirectMessageFailure`, and the own-author / thread-root / redacted /
   non-message guards. The room helpers the operation calls stay on the SDK's
   public API (`Room::load_or_fetch_event`, one `Room::relations` page,
   `Room::send`, `Room::redact`, `Room::make_edit_event`, the room send queue),
   so no `koushi-sdk` helper and no second content policy is added. Headless
   tests cover the semantics and the failure matrix.
2. **MCP surface** (`crates/koushi-mcp`, `crates/koushi-mcp-host`): schemas,
   argument validation, dispatch, typed outputs (including `get_message`'s
   `outputSchema`), the failure mapping table, and the deliberate updates to the
   tool-count, typed-output, and read-only-flag tests.
3. **Grant and approval** (`grants.rs`, `consent.rs`, Tauri forwarding, React
   approval dialog and settings row, message catalog for `en`, `ja`, `ko`,
   `zh-Hans`, `zh-Hant`).
4. **Headless QA scenario** `mcp_message_actions` in
   `crates/koushi-qa/src/bin/headless_core_qa/`, registered in `registry.rs`
   and the lane lists with `scripts/check-headless-qa-lanes.mjs`, run on Tuwunel
   and Synapse.
5. **Canon and docs**: `docs/architecture/overview.md` "External MCP Bridge",
   `docs/agents/state-ownership.md` (direct message-action operations),
   `docs/architecture/state-machine.md` only if a reducer-visible state is
   introduced (none is expected), `docs/help/ai-assistants.md` (tools,
   permissions, limits), `docs/agents/plans.md` link, and this plan.

## Verification

- **RED first.** Each new guard gets a headless test that fails before the
  implementation: own-message edit/delete, already-redacted delete,
  non-message target, thread target that is itself a reply, reaction set
  semantics under two concurrent calls, media caption edit preserving the
  payload, undecryptable body, relation truncation, an invalid or unencrypted
  edit replacement, duplicate `client_request_id`, `busy`, revocation before
  submission, sign-out/lock mid-flight, cross-room and cross-account isolation,
  an already-open room and an unopened room (no WebView event, no receipt, no
  unread or selection change), and schema-1/schema-2 default-off migration.
- **Local gates.** `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test -p koushi-core -p koushi-mcp -p koushi-mcp-host` (plus the
  bridge transport tests in `crates/koushi-mcp-host/tests/message_actions.rs`),
  the MCP browser-headless tests for the approval dialog and settings row, and
  `node scripts/check-agents-docs.mjs`.
- **Local-server QA.** `mcp_message_actions` green on `--server=tuwunel` and
  `--server=synapse`, asserting the observer's server-side view of the
  reaction, edit, redaction, and thread relation, with one private-data-free
  success token. Mutations read their target authoritatively (one bounded
  request), because the local event cache can still hold the pre-redaction copy
  of an event this account just deleted.

## Accepted limits found in review

- **A sent thread reply is a domain echo, not a call event.** The SDK room send
  queue creates a local echo, so a room or thread the user has open shows the
  reply as their own sent message before the server confirms it — exactly what a
  GUI send and an MCP `send_message` already do. There is no send path without a
  local echo; the alternative was claiming the message was not sent. What the
  contract forbids is the call's own pending row, correlation, and progress
  reaching the GUI, and none of those exist here.
- **The revocation window is the Core call itself.** The grant store lives only
  in the host, so the host re-reads it immediately before
  `direct_message_action`. A revocation that lands in the milliseconds between
  that read and the SDK enqueue is not observed; the event is then not rolled
  back, as the contract already states. Closing that window needs a
  Core-visible authorization port; it is not part of this phase.
- **An ambiguous failure keeps the retry key.** A thread reply or edit that
  failed with `internal` may already have reached the homeserver, so its
  `client_request_id` stays claimed for the ledger's ten minutes and a retry
  answers `busy` instead of risking a second event.
- **The QA scenario uses an unencrypted room.** It drives the Core operations
  and proves the observer account sees the reaction, edit, redaction, and thread
  relation server-side. E2EE delivery of those operations is covered by the
  existing encrypted-room lanes for the same send/decrypt paths, not by this
  scenario.

## Deliberate limits

- Own messages only: no moderation of another user's message in this phase.
- No attachments in edits or thread replies; no scheduled sends.
- No forwarding, receipts, fully-read/unread mutation, or typing.
- No thread listing or history beyond `get_message` of one known event.
- `get_message` reads at most one relations page; a thread with a very large
  reaction/edit history reports `relations_truncated` rather than paging to
  exhaustion.
