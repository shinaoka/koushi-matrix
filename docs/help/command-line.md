# Command-line access

[User guide](README.md)

The bundled `koushi` CLI gives scripts and agents every tool of Koushi's MCP
bridge as a command of the same name: `koushi list_rooms`,
`koushi read_room_messages`, `koushi send_message`, and so on. It uses the
same local bridge and verified account sessions as MCP, with the same
arguments, permissions, and results. It does not sign in independently.

## Find the executable

- macOS: `/Applications/Koushi.app/Contents/MacOS/koushi` for an app installed
  in Applications. Adjust the path if you installed it elsewhere.
- Windows: `koushi.exe` alongside `koushi-desktop.exe` in your installation
  directory. In PowerShell use `& 'path-to-installation\koushi.exe' --help`.
- Linux deb/RPM: `koushi` is installed alongside the application's executable;
  use `command -v koushi`. An AppImage contains the executable in its extracted
  `usr/bin` directory; extract with the AppImage's `--appimage-extract` option
  and run `squashfs-root/usr/bin/koushi` while Koushi is running.
- Source checkout: `cargo build -p koushi-mcp --bin koushi --locked`, then
  `target/debug/koushi` (`.exe` on Windows).

Run `koushi --help` for the list of tools, options, exit codes, and examples,
and `koushi <tool> --help` for one tool's arguments, generated from the same
definitions MCP clients see. Help prints without connecting to the app.
`-h`/`--help` works anywhere in the line, and `koushi help [<tool>]` works as
the first word; `koushi list_rooms help` is a usage error.

You may add the executable's directory to your own PATH. Installation does not
edit your shell configuration. The examples below assume `koushi` is on PATH.

## Connect and approve

1. Start Koushi and sign in to the account you want to use. Complete verification.
2. Open **App Settings → AI assistants** and enable the local bridge.
3. Run `koushi list_accounts --json`. Copy the intended `account_id` from the result.
4. Run an account command. If needed, approve **koushi-cli** access in Koushi.

The account ID is always required, even with only one account. Switching the
visible app tab never changes a CLI target. Use the same settings page to
remove access. CLI approval is separate from the named MCP clients' grants.
Missing permissions wait for the existing in-app approval for up to 40 s, or
until `--timeout` if that is shorter; when the wait runs out the command exits
5, and you approve `koushi-cli` in Koushi and run it again. A denial exits 5
too, and is not asked again until you turn that permission on in Koushi. A
script cannot approve itself, and piping input does not skip this step. An
already-approved script needs no new dialog until access is revoked. Each
permission (read, send, draft, attachments, message actions) is approved
separately, exactly as for an MCP client: read access never allows sending,
drafting, or reacting.

Names are self-declared: another process running as your OS user can claim
`koushi-cli`. The boundary is your OS account, local transport, and private
connection token. Grants cover an account and capability, not a room allowlist.
Account enumeration exposes account IDs before approval, as with MCP.

## Commands

The account, room, and event values below are synthetic placeholders. Replace
them with the values returned by your own account's results.

```bash
koushi list_accounts
koushi list_rooms --account-id 'account:@member1:example.invalid' --limit 20 --json
koushi read_room_messages --account-id 'account:@member1:example.invalid' --room-id '!room:example.invalid' --limit 30
koushi get_message --account-id 'account:@member1:example.invalid' --room-id '!room:example.invalid' --event-id '$event:example.invalid'
koushi search_messages --args-stdin < private-arguments.json
koushi send_message --args-stdin < private-arguments.json
```

Each tool argument is an option: `room_id` becomes `--room-id`. A list
argument is given once per element (`--room-ids`), and a true/false argument is
a flag that sets it. The command line holds at most 160 words and 64 KiB; pass
a longer list through `--args-stdin`.

`--args-stdin` instead reads the tool's whole argument object as JSON from
standard input, for example
`{"account_id": "...", "room_id": "...", "body": "...", "client_request_id": "..."}`.
It accepts UTF-8 to EOF, at most 1 MiB, refuses terminal input, and cannot be
combined with argument options. Arguments that carry content have no option and
enter only this way: `body` (message text), `query` (search text), `key` (a
reaction), and `attachments` (local file paths). The CLI is meant for agents and
scripts, and shell history and process listings can expose command-line
arguments, including the names of files you send. Write the object from your
program into the pipe, or from a file you have chosen to protect; never put
confidential text or a local path into shell literals, argv, or environment
variables. Every other argument is an identifier, a bound, or a flag. The CLI
creates no argument file or history. A property added to a tool in a later
version is stdin-only until the CLI classifies it, so an unfamiliar argument
means `--args-stdin`.

`list_rooms` returns `result.next_cursor`; pass it with `--cursor` for another
page. `read_room_messages` returns `result.next`; pass it with `--from` to read
older history until it is `null`. Only `null` ends the history: a page can hold
fewer messages than `--limit`, or none. Limits and other bounds are listed in
each tool's help, and the CLI refuses an out-of-range value before connecting.
`search_messages` returns one bounded result set, without a continuation
cursor. Results reflect the history Koushi can decrypt and index, so an empty
result does not prove there were no messages.

## Upgrading from the first CLI

The first CLI release (up to v0.35.x) had five hand-written commands and was
read-only. Commands are now named after the MCP tools, so those five were
renamed rather than kept as aliases; an old name is an unknown tool and exits 2.

| Up to v0.35.x | Now |
| --- | --- |
| `koushi accounts` | `koushi list_accounts` |
| `koushi rooms` | `koushi list_rooms` |
| `koushi messages` | `koushi read_room_messages` |
| `koushi message` | `koushi get_message` |
| `koushi search` | `koushi search_messages` |
| `--account <id>` | `--account-id <id>` |
| `--room <id>` | `--room-id <id>` |
| `--event <id>` | `--event-id <id>` |
| `--query-stdin` | the `query` property in `--args-stdin` |

`--limit`, `--cursor`, `--from`, `--unread-only`, `--json`, `--endpoint`, and
`--timeout` keep their names. `koushi --help` lists the current commands.

The following limits also changed:

- The command line holds 160 words instead of 32, and `--args-stdin` accepts
  1 MiB of JSON instead of 16 KiB.
- A `query` is no longer trimmed or refused locally when blank. Koushi itself
  trims it and answers with an error, so a blank query exits 6 rather than 2.
  Check the text before calling if you rely on the old exit status.
- A `query` is still never placed in argv; it goes through `--args-stdin`.

## Writes and retries

Sending, thread replies, edits, reactions, deletions, and drafts behave
exactly as for an MCP client. The CLI never retries a call by itself. When
`--timeout` or Ctrl-C ends a write after it was sent, the write may already
have been applied, so exit 4 or 130 does not prove nothing happened.

- `send_message`, `edit_message`, and `send_thread_reply` require a
  `client_request_id` that you choose. To retry, run the same call with the
  same id rather than assuming it failed. Koushi remembers ids for a limited
  time and only while it runs, so this is not exactly-once delivery.
- `react_to_message`, `unreact_to_message`, `delete_message`, and
  `draft_message` have no retry id. Their help says what a repeated call does:
  a second reaction with the same key adds nothing, removing an absent
  reaction removes nothing, deleting a deleted message sends nothing, and a
  draft into a composer that already holds one answers `conflict` and changes
  nothing, unless `replace` applies as that tool's help explains.

## Output and scripts

The default is indented JSON. `--json` emits one compact JSON object, with
`schema_version: 1`, `tool`, `account_id` for account commands, and `result`.
Bridge fields and continuation cursors remain inside `result`. For example,
a script can use `jq '.result.rooms'`. JSON escaping neutralizes terminal
controls and bidirectional formatting; parsing JSON restores the original text.
Treat restored text as untrusted data and never evaluate it as shell commands.

Results may contain decrypted messages and identifying information. Terminal
scrollback, redirected files, CI capture, and downstream programs can retain
plaintext. Choose destinations accordingly. Koushi adds no automatic CLI
result cache. Stderr contains only a fixed failure code, never an argument value,
remote error detail, or a result payload.

`--timeout <seconds>` bounds stdin, connection, approval, operation and result delivery
(default 75, range 1–120). Ctrl-C cancels and closes the connection. The bridge's
own shorter operation deadlines still apply. Each command connects afresh,
so an app restart is picked up on the next invocation. `--endpoint <path>`
selects an existing private local endpoint file; do not copy its token into
commands or modify its permissions.

| Exit | Meaning |
| --- | --- |
| 0 | Success, including a consumer closing the output pipe |
| 1 | Internal or output failure |
| 2 | Invalid arguments/input; run `koushi --help` |
| 3 | Koushi or its enabled bridge is unavailable |
| 4 | Whole-command deadline expired |
| 5 | Local authentication, consent, or permission failure |
| 6 | Account/room/event/operation failure; stderr gives the closed error code |
| 130 | Cancelled with Ctrl-C |

An unavailable, locked, signed-out, or still-verifying account fails explicitly;
the CLI never falls back to another account. No failure payload is written to
stdout before output begins. A failed output write may leave partial output.
A result whose rendered JSON exceeds the CLI's local output bound (16 MiB) fails
as an output failure (exit 1, `output_too_large`), not as an operation failure.
