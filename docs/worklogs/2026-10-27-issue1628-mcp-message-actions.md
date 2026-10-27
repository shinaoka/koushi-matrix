# #1628 — MCP message actions and `get_message`

## Contract

The mandatory contract is [issue #1628](https://github.com/shinaoka/koushi-matrix/issues/1628).
The plan that records the sequence, ownership and deliberate limits is
[2026-10-27-issue1628-mcp-message-actions](../plans/2026-10-27-issue1628-mcp-message-actions.md).

Canon consulted before code: `REPOSITORY_RULES.md` (ownership, root-cause, canon-first,
security, QA gates), `docs/architecture/overview.md` "External MCP Bridge",
`docs/agents/state-ownership.md` "MCP assistants", `docs/policies/engineering-rules.md`
(async/runtime, secrets, test placement), and the #1524 and #1482 plans.

Canon amended in this change: `docs/architecture/overview.md` (message actions are
direct operations, not timeline mutations; schema 3; who owns what) and
`docs/agents/state-ownership.md` (Core owns the semantics, the host owns only
validation, the grant, and failure mapping). No reducer-visible state was added, so
`docs/architecture/state-machine.md` is unchanged; the operation lifecycle is recorded in
the plan instead.

## What changed

- **Core** (`crates/koushi-core`): `DirectMessageRequest` / `DirectMessageResponse` /
  `DirectMessageFailure` in `direct_operations.rs`; `timeline/direct_message.rs` (one
  bounded read plus exactly one 256-relation page, set-semantics reactions, own-author
  edit/redact, thread reply through the SDK room send queue with a private completion
  channel); `AccountMessage::DirectMessageAction` / `CoreCommandEnvelope::DirectMessageAction`
  with a per-account cap of 4 and one lane per reaction target;
  `CoreConnection::direct_message_action`. Display policy is reused, not reimplemented:
  `TimelineItemContent::from_event`, `check_validity_of_replacement_events`,
  `edited_document_content_for_edit_target`, `Room::make_edit_event`, and
  `outbound_send::make_offline_thread_content`.
- **MCP** (`crates/koushi-mcp`, `crates/koushi-mcp-host`): six tool schemas with typed
  outputs (`get_message` has an `outputSchema`), argument validation before any approval
  (reaction key 1-128 bytes without control characters, `client_request_id` charset),
  dispatch, the failure table with stable reason tokens (`not_author`, `not_a_message`,
  `redacted`, `not_thread_root`, `relations_unavailable`), and a bounded process-local
  `client_request_id` ledger (`actions.rs`) for `edit_message` and `send_thread_reply`.
- **Grant** (`grants.rs`, `consent.rs`): `Need::MessageActions`, `connections.json`
  schema 3, schema 1 and 2 still readable with a missing flag meaning `false`,
  `list_accounts.permissions.message_actions`, approval copy and the settings row.
- **GUI** (`apps/desktop`): `McpNeed` and the permission mirror gain `message_actions`,
  the approval dialog renders its can/cannot list, the settings page lists the
  permission, and the `send` copy names thread replies. Catalog entries in `en`, `ja`,
  `ko`, `zh-Hans`, `zh-Hant`.
- **QA**: scenario `mcp_message_actions` in `crates/koushi-qa`, registered in the
  scenario registry and the lane lists, with the contract in `docs/agents/qa-lanes.md`.
- **Docs**: `docs/help/ai-assistants.md` (tool table, permissions, schema-3 example,
  what calls never change, limits, reason tokens).

## Verification

- `cargo test -p koushi-core -p koushi-mcp -p koushi-mcp-host`: 1781 core tests
  (including 10 new direct-message tests), 20 tool tests, 59 host unit tests, and 3 new
  bridge transport tests in `crates/koushi-mcp-host/tests/message_actions.rs`.
- `cargo clippy -p koushi-core -p koushi-mcp -p koushi-mcp-host --all-targets` and
  `cargo fmt --check` clean.
- Frontend: `tsc --noEmit` and 70 focused vitest tests (approval dialog, settings page,
  catalog parity).
- `node scripts/check-agents-docs.mjs` and `node scripts/check-headless-qa-lanes.mjs`
  (44 scenarios) green.
- Local-server QA: `mcp_message_actions` `=ok` on a disposable local Tuwunel and
  again on a disposable local Synapse (`--core`, `--server=tuwunel` and
  `--server=synapse`), asserting the observer account's server-side view of the
  reaction, edited body, redaction, and thread relation.

## Deliberate limits

Own messages only for edit and delete; no attachments in edits or thread replies; no
forwarding, receipts, read-state mutation, or typing; `get_message` reads one relations
page and reports `relations_truncated`; a `client_request_id` is remembered for ten
minutes per bridge run, so a retry after a restart may act again.

## Review record

Two independent pre-implementation reviews (gpt-6.1-sol) changed the design before code:
thread replies must not go through `TimelineCommand::SendText` (pending projection, actor
refresh, `SendCompleted` to the WebView), the dedupe ledger must be real (the SDK queue
mints its own transaction id), and the grant must be re-read at submission. The
post-implementation review (same model) raised 17 findings; the ones this phase
fixes are listed here:

- reactions now count distinct senders per key, like the timeline projection, and
  a truncated or undecryptable relations page refuses the set decision instead of
  acting on a partial set (react and unreact both);
- an undecryptable message reads as a successful event with no body instead of
  `not_a_message`, and its envelope keeps the sender;
- every result struct has a redacted `Debug` (no bodies, reaction keys, or ids);
- a thread reply's own `m.in_reply_to` fallback is reported as
  `reply_to_event_id`;
- the host no longer pre-reads the target (Core owns every verdict), so the
  grant check sits immediately before the operation;
- a settled `client_request_id` replay re-checks the grant and session before
  answering, an in-flight claim is never evicted, and an ambiguous `internal`
  failure keeps the claim instead of inviting a duplicate;
- the edit result reports the caller's `client_request_id` as `transaction_id`
  and the tool description says so (a direct edit has no queued SDK transaction);
- an operation-wide 45 s deadline covers admission, the reaction lane wait,
  resolution, and submission;
- the approval copy says reactions apply to any message the assistant can see,
  not only the user's own (all five catalogs).

The local-server QA run then found two ordering bugs that the unit tests could not:
a mutation reading its target from the local event cache before the redaction echo
arrived (it now reads the target authoritatively in one bounded request), and a
relations page treated as truncated whenever the server returned a continuation
token even though it had returned everything (a page shorter than the limit is now
complete; only a full page is uncertain).

Accepted limits are recorded in the plan: a sent reply's local echo is the same
domain projection as `send_message`; the revocation window is the Core call
itself until a Core-side authorization port exists; an ambiguous failure keeps
the retry key claimed for ten minutes; the QA scenario uses an unencrypted room.
