# #1905 — MCP `draft_message` can replace its own untouched draft

## Contract

[Issue #1905](https://github.com/shinaoka/koushi-matrix/issues/1905):
`draft_message` answered `conflict` whenever the target composer held any draft,
including the one the same assistant staged a moment earlier, so an assistant
could not revise its own draft. `get_draft` (#1809) already reports that state as
`assistant`.

Fix (frozen by the task instructions): add an opt-in `replace: true` argument.
The call replaces the target's draft only when all of these hold, and otherwise
answers `conflict` and changes nothing:

- the target still holds exactly the draft an assistant staged there — the
  `AssistantDraftMarker` names the stored `ComposerDraftRevision`, which is what
  `get_draft` reports as `assistant`;
- no renderer draft or send command is in flight for that exact target (no
  command permit in the composer lease registry) and the open composer has no
  pending submission;
- the `draft` permission is granted, as for any `draft_message` call.

`user_edited` drafts — hand-edited text, or text whose **Drafted by** label the
user hid — are never replaced. Without `replace`, behavior is unchanged. The
replacement goes through the Rust draft lifecycle as a new
`ComposerDraftRevision`, so an open composer follows it live, keeps the assistant
marker, and moves neither focus nor navigation.

Canon consulted before code: `REPOSITORY_RULES.md` (canon-first, root cause,
state-machine discipline, ownership, security, test placement),
`docs/architecture/overview.md` "External MCP Bridge" ("Assistant drafts are the
one composer write", "No GUI coupling"), `docs/architecture/state-machine.md`
composer draft machine, the
[#1524 plan](../plans/2026-10-18-mcp-ui-isolation.md), and
`docs/policies/engineering-rules.md` (async/runtime, test placement).

Canon amended in this change: `docs/architecture/overview.md` (the `replace`
widening of the assistant-draft exception),
`docs/architecture/state-machine.md` (`AssistantDraftStaged` carries `replace`;
one new self-transition), and `docs/help/ai-assistants.md` (tool table, the
`draft_message` section, the `conflict` row).

## Files changed

- `crates/koushi-state/src/action.rs` — `AssistantDraftStaged` gains
  `replace: bool`.
- `crates/koushi-state/src/state/timeline.rs` —
  `ComposerDraftStore::assistant_draft_is_untouched` (marker names the stored
  revision) and `stage_assistant_draft(..., replace)`, which writes a non-empty
  target only when that guard holds.
- `crates/koushi-state/src/reducer/{mod,timeline}.rs` — the reducer passes
  `replace` through and hydrates an open composer that already shows the older
  revision, not only an empty one.
- `crates/koushi-core/src/direct_operations.rs`, `runtime.rs` — the request
  carries `replace`; `handle_assistant_draft` keeps the existing room, permit,
  and pending-send guards, admits a non-empty target only when it is replaceable,
  and answers success from the stored revision moving forward instead of from
  the target staying non-empty (a replaced draft is never empty, so the old test
  could not tell a refusal from a write).
- `crates/koushi-mcp/src/tools.rs` — the `replace` boolean in the input schema
  and the description; `crates/koushi-mcp-host/src/handlers.rs` — parse it
  (`unsupported` when it is not a boolean, for a host reached without the
  launcher's schema validation); `registration.rs` — the tool summary line.

## Verification

RED first, on the bridge suite:
`cargo test -p koushi-mcp-host --profile ci --test bridge -- draft_message`
failed with `conflict` for the replacement case.

GREEN after the change:

- `crates/koushi-mcp-host/tests/bridge/draft_message.rs` — a staged draft is
  replaced and reads back as the new body with status `assistant` at a newer
  revision; a user-edited draft is never replaced and its text stays; a composer
  the user cleared accepts the new draft instead of resurrecting the old text;
  the existing "second draft without `replace` conflicts" test still passes.
- `crates/koushi-core/src/runtime/tests/assistant_drafts.rs` — the actor
  replaces an untouched draft at a new revision, refuses a user draft, refuses
  while a renderer command permit is held (and accepts once released), and
  refuses while the open composer has a pending send.
- `crates/koushi-state/tests/state/assistant_drafts.rs` — the reducer hydrates
  the open main and thread composers at a new revision without changing
  navigation or the composer mode, and never writes over edited or
  label-dismissed text.

Live GUI confirmation (an open composer following the replacement) is manual
inspection, not correctness evidence; the headless reducer test pins that the
replacement hydrates the open composer without touching navigation.
