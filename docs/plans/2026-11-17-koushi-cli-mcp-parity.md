# CLI generated from the MCP tool table (#1944)

Canon: REPOSITORY_RULES Security Rules; engineering rules Secrets and Private
Data and Test Placement; overview Local CLI and External MCP Bridge; test
suites; agent verification and QA lanes (Local CLI).

The canon amendment in this change (overview "Local CLI", the CLI stdout rule
in REPOSITORY_RULES, and the CLI paragraph of the engineering rules) is
reviewed before implementation, because it widens an authorization surface:
the CLI gains every write tool. Decided by the user (2026-11-17): private
text (`body`, `query`, `key`) enters only through `--args-stdin`, keeping
#1651's stdin-only condition, because the CLI is for agents.

## Command and option mapping

`crates/koushi-mcp/src/cli/args.rs` replaces the five hand-written verbs with a
parser driven by `tools::definitions()`:

```text
koushi <tool> [<tool options>] [--json] [--endpoint <path>] [--timeout <s>]
koushi <tool> --args-stdin       [--json] [--endpoint <path>] [--timeout <s>]
koushi [<tool>] --help | -h | help
koushi --version | -V
```

| Schema property | CLI form |
| --- | --- |
| `"type": "integer"` / `"boolean"` | `--limit <n>`: one value; `--unread-only`: flag, sets `true` |
| `"type": "string"` on the allowlist (ids, cursors, `client_request_id`) | `--room-id <v>`: one value |
| array of strings on the allowlist (`room_ids`) | `--room-ids <v>`, repeated per element |
| any other string, or an array of one-property objects (`attachments: [{path}]`) | no option: `--args-stdin` only |
| any other shape | `--args-stdin` only; help says so |

- Names: `snake_case` → `--kebab-case`. A unit test asserts no generated
  option collides with a CLI option (`--json`, `--endpoint`, `--timeout`,
  `--args-stdin`, `--help`, `--version`); `timeout_s` becomes `--timeout-s`.
- A repeated scalar option, an unknown option, a missing value, or a missing
  `required` property is `usage_error` (exit 2), with no value echoed.
- Types are checked by `tools::validate_arguments`; it checks types only, so
  the CLI also checks `minimum`, `maximum`, `minLength`, and `maxItems` from
  the schema before connecting. This keeps today's `--limit 201` usage error.
  It is stricter than MCP for `list_space_rooms.max_depth`, which the host
  clamps.
- `--args-stdin` reads UTF-8 JSON from non-terminal stdin to EOF, at most the
  bridge frame bound (`wire::MAX_FRAME_BYTES`, 1 MiB) minus 1 KiB for the
  request envelope around the object, so an object at the bound still fits one
  frame and an oversized one is a usage error (exit 2) before connecting
  rather than `unsupported` (exit 6) after. It requires a JSON object and
  excludes every tool option. It replaces `--query-stdin`.
- `koushi --help` keeps the #1943 sections; COMMANDS lists every tool with the
  first sentence of its description. `koushi <tool> --help` prints NAME,
  DESCRIPTION (the full tool description), SYNOPSIS, ARGUMENTS (one per property: flag,
  value form, required, bounds, property description; a property without a
  description gets the type and required line only), and for a tool with
  `read_only: false` a WRITES paragraph: a timeout or Ctrl-C after submission
  may already have applied the write. When the schema declares
  `client_request_id` (`send_message`, `edit_message`, `send_thread_reply`),
  the paragraph says to retry with the same id. Otherwise (`react_to_message`,
  `unreact_to_message`, `delete_message`, `draft_message`) it says the tool
  has no retry id and points to its description, which states what a repeat
  does (`added: false`, `removed: 0`, `already_redacted`, `conflict`). Help
  also names bounds only the host checks: a reaction key of at most 128 UTF-8
  bytes and a draft body of at most 16 KiB.
  An argument that carries content gets no option; its help says it comes only
  through `--args-stdin`, because the CLI is for agents and argv reaches shell
  history and process listings, and shows the argument object to pipe. The
  classification is an allowlist of identifier-valued property names
  (`cli::ARGV_SAFE_PROPERTIES`) beside the parser, so it fails closed: `body`,
  `query`, `key`, `attachments` (a local path), and any property added later
  are stdin-only until someone classifies them. `tool_table.rs` asserts that
  every text property of every tool lands on one side or the other, that the
  allowlist has no stale entry, that a new name has no option, and pins the
  whole generated surface in `surface.golden.txt`.

## Execution, consent and `client_request_id`

`cli/mod.rs::execute` is unchanged in shape: one `tools/call` per process
under the whole-command deadline and the Ctrl-C select, with the existing
envelope and failure mapping. The CLI never creates, changes, or reuses a
`client_request_id` and never retries. Consent is the host's: `koushi-cli`
gets exactly the grants MCP clients get per call; no host code changes. A
write that times out exits 4 with `koushi: timeout`, as today.

Open point for review: the issue states that reusing a `client_request_id`
"never applies a write twice". The canon limits that: the message-action
ledger forgets entries after 600 s, at 128 per account, and on restart, and
`send_message` maps the id to the Core transaction id `mcp-<id>`, whose
deduplication after completion is not verified here. Help and canon therefore
defer to the host's scope rather than promise exactly-once delivery.

## Tests (red first)

The flat `tests/cli_process.rs` becomes the suite `tests/cli/` (one binary,
listed in `docs/architecture/test-suites.md` and in the `layouts` of
`scripts/rust-test-suites.test.mjs`, so the suite is enforced) with feature
modules:

- `help.rs`: existing help tests, adapted to tool names.
- `tool_table.rs`: for every `tools::definitions()` entry, `koushi --help`
  lists it and `koushi <tool> --help` exits 0 without connecting and names
  every property's option; a tool without CLI coverage fails.
- `reads.rs`: existing read, escaping, output-bound, pipe, deadline, and
  Ctrl-C tests on the new names; `--args-stdin` reaches the fake app as the
  exact argument object.
- `writes.rs` (fake app): `consent_required` exits 5 with empty stdout;
  two runs with the same `client_request_id` send it unchanged and a `busy`
  reply exits 6; a deadline after the request reached the fake exits 4 with
  only `koushi: timeout`; a missing `client_request_id` is a usage error
  before connecting.
- `crates/koushi-mcp-host/tests/bridge/cli_bridge.rs`: a `koushi-cli` read
  grant does not admit `draft_message` or `send_message` (fail-closed
  `consent_required`).

## Other changes

`scripts/cli-local-qa.mjs` and `docs/agents/qa-lanes.md` use the tool names;
`docs/help/command-line.md`, `README.md`, and
`docs/releases/desktop-release.md` drop "read-only" and document writes,
grants, `--args-stdin`, and the ambiguous-write rule.

Gates: `cargo test -p koushi-mcp`, `cargo test -p koushi-mcp-host --test
bridge`, `cargo clippy -p koushi-mcp -p koushi-mcp-host --all-targets`,
`cargo fmt --check`, `node scripts/check-rust-test-structure.mjs`,
`node scripts/user-help.mjs --check`, `node scripts/check-agents-docs.mjs`.

## Review

Design review (independent, different model family, of the first canon
draft): proceed with changes, no critical findings. Applied: write help
conditioned on a declared `client_request_id`; the suite added to the
enforced inventory; the stdin bound aligned with the bridge frame; a help
fallback for properties without a description; host-only bounds named in
help. The user kept private text stdin-only.

Integrated-diff review (independent, different model family, of the diff at
11f31aa7): merge with fixes; no critical findings, no authorization bypass,
no private-text leak. Important: the 64-word argv cap let a repeated list
option carry only about 30 elements, below `read_recent_activity`'s
`maxItems` of 64. Minor: help bounds omitted `minLength`. Noted: help offered
an option for argument shapes the parser rejects. All three are fixed in
f390fad8: the cap is 160 words, named in help, with boundary tests; help
names `minLength`; one rule decides whether a property gets an option, for
parser and help alike.

## Evidence

- RED first: the rewritten and new `tests/cli/` cases failed against the
  verb parser (17 of 23), then passed with the generated parser.
- `cargo test -p koushi-mcp -p koushi-mcp-host`: all pass (`cli` 30, host
  `bridge` 58 including `a_cli_read_grant_admits_no_write_tool`, which fails
  when the write grants are injected). `cargo clippy -p koushi-mcp -p
  koushi-mcp-host --all-targets -- -D warnings` and `cargo fmt --check` on both
  crates are clean.
- Test-structure, suite-inventory, agents-docs, headless-QA-lane, secret-scan,
  SDK-submodule and user-help checks pass.
- `node scripts/cli-local-qa.mjs` against a disposable local Tuwunel prints
  `cli_accounts=ok`, `cli_encrypted_read=ok`, `cli_encrypted_search=ok`.
- Help shape: audited against the agent-CLI help contract. The overview and
  every tool help carry NAME, DESCRIPTION with a use-when line, INSTRUCTIONS,
  SYNOPSIS, COMMANDS (arguments, stdout, side effects, exit status), OPTIONS,
  STDIN, WRITES for write tools, EXIT STATUS, ENVIRONMENT, EXAMPLES and
  OUTPUT, in that order; `tests/cli/help.rs` and `tests/cli/tool_table.rs`
  pin the order of all but WRITES for the overview and every tool, and
  require WRITES in every write tool's help.
- Acceptance test: a fresh agent of another model family (Mistral Large 4,
  non-interactive, bash tool only, no context files, skills or MCP) got the
  endpoint path, the built binary on `PATH`, and the instruction to learn the
  CLI only from `koushi --help` and `koushi <tool> --help`. Against a
  disposable local Tuwunel with the `headless-core-qa` bridge host and a
  schema-3 `koushi-cli` grant, it listed the accounts, found "Synthetic CLI
  QA", read its messages, drafted a reply, sent it with a fresh
  `client_request_id`, and reacted to the fixture message, each verified with
  a read (`get_message` reactions, `get_draft`), with no other input. Private
  text went through `--args-stdin` from files it wrote, following the
  examples. Where it stumbled: after a non-null `result.next` it read an
  empty page and searched to confirm nothing was missing, and the overview
  example chose `rooms[0]` of unread rooms. Help now says only a null cursor
  ends paging and selects the room by name (tests first). Core drops
  non-message events from a page and returns the server's end token as
  `next` (`koushi-core/src/timeline/direct_room_messages.rs`), so a page can
  be empty while `next` is non-null. Its other remarks
  concern MCP tool descriptions shared with MCP clients, not CLI help, and
  are left as follow-ups: tools without an output schema list no result
  fields in help, and `draft_message` reports `status: "staged"` while
  `get_draft` reports `assistant` for the same draft.
- Merging #1954 (help text matched to the parser and approval wait): the
  generated help says `help` is recognized only as the first word (a test
  pins `koushi list_rooms help` as exit 2), that approval waits up to 40 s or
  until `--timeout`, and that a denial is not asked again until the
  permission is turned on in Koushi; the overview and every tool help are
  tested for both. Its stdin line-ending fix has no counterpart: the CLI's
  only stdin is the `--args-stdin` JSON object.
- Not run here: Windows build and runtime, macOS packaging, and the CI matrix.
