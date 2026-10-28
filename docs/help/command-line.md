# Command-line access

[User guide](README.md)

The bundled `koushi` CLI reads and searches through the running Koushi app.
It uses the same local bridge and verified account sessions as MCP. It does
not sign in independently. This first version supports account/room lists,
message history, individual messages, and search. Sending and attachments
remain outside this CLI's scope.

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

You may add the executable's directory to your own PATH. Installation does not
edit your shell configuration. The examples below assume `koushi` is on PATH.

## Connect and approve

1. Start Koushi and sign in to the account you want to use. Complete verification.
2. Open **App Settings → AI assistants** and enable the local bridge.
3. Run `koushi accounts --json`. Copy the intended `account_id` from the result.
4. Run an account command. If needed, approve **koushi-cli** read access in Koushi.

The account ID is always required, even with only one account. Switching the
visible app tab never changes a CLI target. Use the same settings page to
remove access. CLI approval is separate from the named MCP clients' grants.
Missing permissions wait for the existing in-app approval; a script cannot
approve itself, and piping input does not skip this step. An already-approved
script needs no new dialog until access is revoked.

Names are self-declared: another process running as your OS user can claim
`koushi-cli`. The boundary is your OS account, local transport, and private
connection token. Grants cover an account and capability, not a room allowlist.
Account enumeration exposes account IDs before approval, as with MCP.

## Commands

The account, room, and event values below are synthetic placeholders. Replace
them with the values returned by your own account's results.

```bash
koushi accounts
koushi rooms --account 'account:@member1:example.invalid' --limit 20 --json
koushi messages --account 'account:@member1:example.invalid' --room '!room:example.invalid' --limit 30
koushi message --account 'account:@member1:example.invalid' --room '!room:example.invalid' --event '$event:example.invalid'
koushi search --account 'account:@member1:example.invalid' --query-stdin < private-query.txt
```

Search accepts UTF-8 stdin to EOF, at most 16 KiB, and refuses terminal input.
Use a private input source such as a secret manager's pipe or a file you have
chosen to protect. Do not put confidential queries into shell literals, argv,
or environment variables; shell history and process listings can expose them.
The CLI creates no query file or history. A trailing line ending is removed.
Use `--room` to restrict a search to one room.

`rooms` returns `result.next_cursor`; pass it with `--cursor` for another page.
`messages` returns `result.next`; pass it with `--from` to read older history.
Limits are 1–200 for rooms and 1–100 for messages/search. `rooms --unread-only`
filters rooms with unread activity. Search returns one bounded result set,
without a continuation cursor. Results reflect the history Koushi can decrypt
and index, so an empty result does not prove there were no messages.

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
result cache. Stderr contains only a fixed failure code, never the query,
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
