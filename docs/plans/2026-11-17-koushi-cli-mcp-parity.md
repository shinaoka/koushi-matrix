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
| `"type": "string"` / `"integer"` | `--room-id <v>`, `--limit <n>`: one value |
| `"type": "boolean"` | `--unread-only`: flag, sets `true` |
| array of strings (`room_ids`) | `--room-ids <v>`, repeated per element |
| array of one-property objects (`attachments: [{path}]`) | `--attachments <path>`, repeated, fills `path` |
| `body`, `query`, `key` (private text) | no option: `--args-stdin` only |
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
- `--args-stdin` reads UTF-8 JSON from non-terminal stdin to EOF, at most
  the bridge frame bound (`wire::MAX_FRAME_BYTES`, 1 MiB), so any body MCP can
  send fits; it requires a JSON object and excludes every tool option. It
  replaces `--query-stdin`. A request whose encoded frame still exceeds the
  bound fails in `ipc::Client::call` as today.
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
  A tool with a private text property (`body`, `query`, `key`) gets no option
  for it; its help says that argument comes only through `--args-stdin`,
  because the CLI is for agents and argv reaches shell history and process
  listings, and shows the argument object to pipe. The closed list of private
  property names lives beside the parser; `tool_table.rs` asserts that every
  property it names exists in some tool, so a renamed property fails CI.

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
help. The user kept private text stdin-only. The integrated diff gets its own
independent review before landing.

## Evidence

- RED first: the rewritten and new `tests/cli/` cases failed against the
  verb parser (17 of 23), then passed with the generated parser.
- `cargo test -p koushi-mcp -p koushi-mcp-host`: all pass (`cli` 23, host
  `bridge` 58 including `a_cli_read_grant_admits_no_write_tool`, which fails
  when the write grants are injected). `cargo clippy -p koushi-mcp -p
  koushi-mcp-host --all-targets -- -D warnings` and `cargo fmt --check` on both
  crates are clean.
- Test-structure, suite-inventory, agents-docs, headless-QA-lane, secret-scan,
  SDK-submodule and user-help checks pass.
- `node scripts/cli-local-qa.mjs` against a disposable local Tuwunel prints
  `cli_accounts=ok`, `cli_encrypted_read=ok`, `cli_encrypted_search=ok`.
- Not run here: Windows build and runtime, macOS packaging, and the CI matrix.
