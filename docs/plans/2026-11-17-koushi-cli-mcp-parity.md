# CLI generated from the MCP tool table (#1944)

Canon: REPOSITORY_RULES Security Rules; engineering rules Secrets and Private
Data and Test Placement; overview Local CLI and External MCP Bridge; test
suites; agent verification and QA lanes (Local CLI).

The canon amendment in this change (overview "Local CLI", the CLI stdout rule
in REPOSITORY_RULES, and the CLI paragraph of the engineering rules) is
reviewed before implementation, because it widens an authorization surface:
the CLI gains every write tool.

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
  64 KiB, requires a JSON object, and excludes every tool option. It replaces
  `--query-stdin`.
- `koushi --help` keeps the #1943 sections; COMMANDS lists every tool with the
  first sentence of its description. `koushi <tool> --help` prints NAME,
  DESCRIPTION (the full tool description), OPTIONS (one per property: flag,
  value form, required, bounds, property description), and for a tool with
  `read_only: false` a WRITES paragraph: a timeout or Ctrl-C after submission
  may already have applied the write; retry with the same `client_request_id`.
  Text properties (`body`, `query`, `key`) carry a note recommending
  `--args-stdin` for private text.

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
listed in `docs/architecture/test-suites.md`) with feature modules:

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
