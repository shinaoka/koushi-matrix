# README presents the CLI on par with MCP (#2002)

Since #1952 the bundled `koushi` CLI reaches every MCP tool, and #1943 gave it
agent-facing help generated from the same tool table. `docs/help/command-line.md`
describes all of that, but the README said it in one line at the very bottom
("Scripts and agents can call every MCP tool through the bundled `koushi` CLI"),
while the section that presents "Use Koushi from your AI assistant" presented
MCP alone. A user without an MCP client, or who does not want to register one,
had no way in from the front page.

## What changed

- **The section now presents both ways in**, in order: the MCP server, then the
  same tools from a shell. The prompts and the Read/Act/You-decide bullets stay
  shared, because they describe the tool surface, not the transport.
- **`### With MCP`** keeps the existing paragraph and `claude mcp add` line
  unchanged under a heading, so the new subsection reads as its peer.
- **`### Without MCP: the bundled `koushi` command`** is the new half: how to
  turn the bridge on before the first command, two example commands
  (`list_accounts --json`, then `list_rooms --account-id` with the id read from
  `result.accounts`), where the full command list comes from (`koushi --help`),
  that a tool's `--help` is generated from the MCP tool definitions and prints
  without connecting, and the failure contract. It names the one thing a reader
  could otherwise get wrong — the CLI is not a standalone client, and although
  it uses the same bridge and the same approval dialog, it is approved under its
  own name, `koushi-cli`, so an existing Claude Code grant does not cover it. It
  links `docs/help/command-line.md` for the executable's path, `--args-stdin`,
  the full exit table, and retries.
- **The trailing one-liner is gone**, moved into that subsection as the task
  asked. The front-page feature bullet now names both transports instead of
  "act as a local MCP server" alone.

## Checked against the source, not just the help page

Each claim in the new subsection was matched to the CLI itself, since
`docs/help/command-line.md` could itself be stale:

- Help is resolved during argument parsing and returns before any connection
  (`crates/koushi-mcp/src/cli/mod.rs:95`, `args.rs:65-70`), so "prints without
  connecting" holds, and `koushi <tool> --help` reaches the tool's own text
  through `crate::tools::find`.
- The exit codes live in the CLI's own table
  (`crates/koushi-mcp/src/cli/help.rs:102-119`) and the mapping in
  `Failure::bridge` (`mod.rs:80-87`). The README deliberately does not restate
  the table: a shortened list reads as the whole contract, and the two codes a
  writer needs most are the ones a summary drops. It says a failure exits
  non-zero, names 4 (deadline) and 130 (Ctrl-C) because neither proves a write
  did not happen (`docs/help/command-line.md`, "Writes and retries"), and links
  the full table.
- **The approval story is per client name, not shared with MCP.** The CLI
  connects as `koushi-cli` (`crates/koushi-mcp/src/cli/mod.rs:153`), and
  `cli_grants_do_not_inherit_from_mcp_or_cross_accounts_and_revocation_applies`
  (`crates/koushi-mcp-host/tests/bridge/cli_bridge.rs:100-125`) grants `read` to
  `codex` and asserts the CLI still gets `consent_required`. The README says so
  explicitly; an earlier draft said "the same in-app approval as MCP", which a
  reader with an existing grant would take as "already covered".
- Stderr is `koushi: <code>` and "never an argument, a result, or remote error
  detail" per the same help text; the README repeats that, not a paraphrase of
  it.
- The two example commands are the documented first steps: `list_accounts
  --json`, then an account command with the id it returned. The account id is
  quoted and labelled rather than shown as a literal, because the real value is
  a homeserver-qualified id.

## Site copy

`site/content/en.json`'s `ai.lede` said the AI section was about an MCP server
only, which the README now contradicts. English text gained one clause naming
the `koushi` command and changed "The assistant" to "Either way the assistant".
The four translations were written to match and re-stamped with
`node site/stamp-translations.mjs <locale> ai.lede --from …`, never by hand.
The lede is inserted through `t()`, which escapes, so the command name appears
as plain text there rather than in `<code>`.

## Verification

- `node scripts/check-agents-docs.mjs` → `agents docs ok` (AGENTS.md 7 topic
  files).
- `node scripts/user-help.mjs` → `User help OK: 11 pages, navigation, local
  links/anchors, and llms.txt` after `--write`, which regenerated the one
  `llms.txt` line for `docs/help/command-line.md`: its index description still
  read "Read and search through the running Koushi app", which stopped being
  true once the CLI could send, edit, react, and draft.
- `node site/check-translations.mjs` → `site translations ok`. That script only
  warns on stale hashes (`site/check-translations.mjs:10-16`, and the CI step is
  named "stale entries only warn"), so it proves the keys are present and well
  formed, not that they are current. Staleness was checked directly instead:
  `hashText(en["ai.lede"])` is `921a54249b32`, and all four locales store that
  hash.
- `node --test site/lib/*.test.mjs` → 22 pass (the CI job at `ci.yml:49`).
- `node site/build.mjs --tag v0.36.2 --screenshots … --out /tmp/site-2002` with
  stand-in PNGs: builds, and the `#ai` lede renders the new sentence in all five
  locales. The real screenshot set is produced by the Website workflow, not
  checked in.

## Review round

`independent-review` in post mode on `opencode-go/glm-5.3`, against head
`72d283c9`. Two IMPORTANT and six MINOR findings; no BLOCKER.

**Fixed:**

- IMPORTANT — "the same in-app approval as MCP" read as grant inheritance. The
  CLI is approved as its own client name, so the sentence now says that, and
  drops "the CLI is never a way around them", which only existed to prop up the
  wrong reading.
- IMPORTANT — the five-code exit summary read as the whole table while omitting
  4 and 130, the two codes that matter after a write. The enumeration is gone;
  the text names those two and links the table.
- MINOR — the examples came before the step that makes them work. The bridge
  enable now precedes the code block, and the account-id placeholder says where
  in the output it comes from (`result.accounts`).
- MINOR — the worklog claimed `check-translations.mjs` proved freshness, which
  it does not; replaced with the direct hash comparison above.

**Rejected, with the source that decides it:**

- The reviewer proposed deleting the CLI clause from the front-page feature
  bullet because it repeats the section's first sentence. The bullet list is
  this README's scanning surface, and every other bullet there duplicates detail
  from further down; removing it would restore exactly the gap #2002 reports.
- The reviewer's lean advice on the site copy was to revert `ai.lede` rather
  than leave a shell path with no steps on the page. The clause states what the
  CLI is, not that the page will walk through it, and the page's guide link
  already reaches `command-line.md` through `ai-assistants.md:510`; adding a
  second command block or five more strings would be the larger change.

Not applied: the reviewer's request to trim the "Checked against the source"
citations. The record is the point, and it is now accurate.

## Not changed

- `docs/help/ai-assistants.md` already points at `docs/help/command-line.md` for
  terminal access (`:510`), and `command-line.md` already states the bridge
  requirement, so no help page needed an edit beyond the index description.
- The README's MCP paragraph is unchanged in substance: only its heading is new.
  Its long-line break ("See [Connect an AI assistant](…) for Claude") stays as
  it was, since reformatting prose is not this issue.
