# Connect an AI assistant (MCP)

Koushi can act as a local [Model Context Protocol](https://modelcontextprotocol.io)
server for Claude Code, Codex, pi, DeepSeek Harness, and any other stdio MCP
client. The assistant runs on the same computer and talks to the Koushi app you
already have open, so it can search your messages and send replies without ever
holding your encryption keys. Koushi asks you before an assistant can use each
account and each permission.

[Back to the user guide](README.md)

## What the assistant can do

| Tool | What it does | Permission |
| --- | --- | --- |
| `list_accounts` | Lists the signed-in accounts and whether each is ready. | none |
| `list_rooms` | Lists rooms and direct messages with names, encryption state, unread counts, and the latest event's sender and time. | read |
| `read_room_messages` | Reads a room's recent messages, newest first, with sender, time, and text. | read |
| `search_messages` | Searches decrypted history and returns room, event id, sender, timestamp, and a snippet. | read |
| `get_attachment` | Downloads one attachment into Koushi's media cache and returns its path. | attachments |
| `send_message` | Sends a message, optionally as a reply, or sends local files with an optional caption. | send; also attachments when sending files |
| `wait_for_events` | Waits for sampled room changes in the account and returns the changed rooms. | read |

Every tool needs an `account_id`. The assistant gets it from `list_accounts`;
Koushi never guesses which account you meant. `list_accounts` needs no
approval, so any connected assistant can see each account's id, which contains
its Matrix user ID, and whether it is signed in. Names, rooms, and messages need
your approval.

`list_rooms` answers the "what is unread" question: `unread_messages`,
`unread_notifications`, `unread_mentions`, the thread equivalents, and
`marked_unread` come from the room list, and `last_event` carries the newest
event's sender, time, and preview. Pass `unread_only: true` to get only rooms
with something waiting. `read_room_messages` then reads a room whether or not it
is open, and `search_messages` returns a timestamp for every hit, so an assistant
can date a conversation instead of guessing.

## Before you start

- Koushi is running and signed in. If Koushi is closed, tools answer
  `koushi_unavailable` and recover when you reopen it.
- **App Settings → AI assistants → Let AI assistants connect** is on. It is on
  by default; turning it off disconnects every assistant at once.
- The `koushi-mcp` launcher ships with Koushi. The settings page shows its
  path inside the registration text, so you never need to find it yourself.
  - On macOS, run Koushi from the Applications folder. A copy opened from the
    disk image or a temporary location cannot be used, and the page says so.
  - The Linux AppImage copies the launcher to
    `~/.local/share/koushi-desktop/mcp/bin/koushi-mcp`, because the AppImage's
    own path changes every time it runs.
  - A build from source without the launcher shows "does not include the
    koushi-mcp launcher". Build it in a checkout of this repository with
    `cargo build --release -p koushi-mcp`; the binary is
    `target/release/koushi-mcp`.
- The assistant runs as the same operating-system user as Koushi. The bridge
  socket and its token are readable only by that user.

## Step 1: register the assistant

Open **App Settings → AI assistants** and pick your assistant under **Connect
an assistant**.

| Assistant | How to register |
| --- | --- |
| Claude Code | Select **Register**, check the command, and select **Run**. Koushi runs `claude mcp add --scope user koushi -- <launcher> --client-name claude-code`. |
| Codex | **Register** runs `codex mcp add koushi -- <launcher> --client-name codex`. |
| pi (1.0 or later) | **Register** runs `pi mcp add koushi -- <launcher> --client-name pi`. |
| Claude Desktop (macOS, Windows) | Select **Create Claude Desktop extension**. Koushi saves `Koushi.mcpb` in your Downloads folder and opens it; finish in Claude Desktop's install review. If nothing opens, install it from Claude Desktop's **Settings → Extensions**, or drag the file into the Claude window. |
| DeepSeek Harness | Copy the YAML entry (a `@deepseek-ai/dsh-mcp-client` plugin with `transport: stdio`) into `~/.dsh/profiles/<profile>/cordis.patch.yml`. Install the client plugin in that profile first. |
| Other | Copy the `mcpServers` JSON entry into the client's MCP configuration. |

**Register** runs the assistant's own registration command, so the assistant
saves the registration in its own settings; Koushi writes no other tool's
files. Every tab also offers **Copy**, so you can run the command in a terminal
yourself. If Koushi cannot find the command (the assistant is not installed, or
it is installed somewhere your login shell does not put on `PATH`), copy it
and run it in a terminal.

The Claude Desktop extension contains its own copy of the launcher. After you
update Koushi, create the extension again; the settings page tells you when a
launcher from an older Koushi tries to connect. Registrations made with
**Register** point at the launcher inside Koushi, so they keep working after an
update.

`--client-name` is the name Koushi shows when the assistant asks for access.
Keep it the same, or Koushi treats the assistant as a new one. Every command also
accepts an explicit endpoint file with `--endpoint /path/to/endpoint.json`; the
default location is discovered automatically.

Then ask the assistant something, for example "List my Koushi accounts and
rooms." `claude mcp list`, `codex mcp list`, or `pi mcp list` show that the
registration exists; only a tool call shows that Koushi is reachable.

## Register another MCP client

Any client that can start a stdio server works. The **Other** text is the
launcher's absolute path plus a client name:

```json
{
  "mcpServers": {
    "koushi": {
      "command": "/Applications/Koushi.app/Contents/MacOS/koushi-mcp",
      "args": ["--client-name", "my-client"]
    }
  }
}
```

Client names may use letters, digits, `.`, `_`, and `-`, up to 64 characters.
Clients launched from the Dock often do not inherit your shell `PATH`, so keep
the full path. If the client runs under a different home directory, pass
`--endpoint` explicitly.

## Clients that can only reach a URL

Some assistants accept nothing but an `https://` MCP URL: hosted connectors such
as ChatGPT's, and chat surfaces that never run a process on your machine. They
cannot start the launcher and cannot open Koushi's socket. Bridge the gap with
a tunnel instead of exposing the socket:

```text
remote assistant -> OpenAI-hosted tunnel endpoint -> tunnel-client (your Mac)
                 -> koushi-mcp --client-name <name> -> Koushi
```

1. Create a tunnel in [Platform tunnel settings](https://platform.openai.com/settings/organization/tunnels)
   and note the `tunnel_id`. Associate it with the ChatGPT workspace and the
   Platform organization the assistant uses; a tunnel tied to only one of them
   does not appear in the connector.
2. Install `tunnel-client` from that page or from
   [openai/tunnel-client releases](https://github.com/openai/tunnel-client/releases/latest),
   with a runtime API key that has Tunnels **Read + Use**.
3. Point a profile at the launcher and keep it running while the assistant
   discovers and calls the tools:

   ```bash
   tunnel-client init --sample sample_mcp_stdio_local --profile koushi \
     --tunnel-id tunnel_0123456789abcdef0123456789abcdef \
     --mcp-command "/Applications/Koushi.app/Contents/MacOS/koushi-mcp --client-name my-client"
   tunnel-client doctor --profile koushi --explain
   tunnel-client run --profile koushi
   ```

4. Register the assistant against the tunnel. In ChatGPT that is
   **Plugins → Add custom MCP server → Connection: Tunnel**. A client that
   only takes a URL and a header needs the endpoint the tunnel exposes; use
   header authentication, because connectors that only support OAuth servers
   cannot reach Koushi.
5. Ask the assistant something. Koushi asks you to approve the client name you
   passed to `--client-name`, as for any other assistant.

The tunnel opens no inbound port, but everything it exposes stays reachable
while `tunnel-client run` is alive: stop it when you are done. Each request has
a 120-second deadline and a 10 MiB payload limit, and the assistant's vendor can
call the tools for as long as the connector is authorized. Not every hosted
connector accepts a tunnel endpoint; one that only takes a public `https://`
URL cannot reach Koushi, and no local workaround makes it reachable.

## Step 2: approve it in Koushi

The first time an assistant needs something, Koushi shows a dialog and the
assistant's call waits for your answer (up to 40 seconds):

- **Per account.** The first call that touches an account asks about that
  account only. Other accounts stay hidden until the assistant uses them and you
  approve them too.
- **Per permission.** Reading, sending, and attachments are asked separately,
  when first needed. An assistant you allowed to read asks again the first time
  it sends. The attachments permission covers both downloading attachments and
  sending files from your computer, so an assistant allowed to send text asks
  again the first time it sends a file.
- **Allow** keeps that permission for that account. After you allow sending,
  later messages are sent without asking again.
- **Deny** refuses that permission. Koushi remembers the denial until you turn
  the permission on in settings, remove the assistant, turn the bridge off, or
  restart Koushi, so a retrying assistant cannot ask again and again.
- Closing the dialog without answering leaves the request open for five
  minutes. **App Settings → AI assistants → Waiting for your answer** lists
  open requests so you can still answer them. If the assistant gave up
  waiting, it gets `consent_required`; once you allow, its next try works.

For a send, the dialog shows the message the assistant is waiting to send, as
context. If the assistant stopped waiting, that message is not sent; your answer
still applies to its later messages.

The name in the dialog is the one the assistant passed with `--client-name`.
Koushi cannot verify it: any program running as you could use the same name.
Allow a request only when you just set up or asked that assistant.

### Change or remove access

**App Settings → AI assistants → Connected assistants** lists every assistant
that has asked, with a **Read**, **Send**, and **Attachments** switch for each
account. A permission you denied shows **Denied**; turning its switch on allows
it. **Remove access** deletes everything for that assistant. Changes apply on
the assistant's next call.

### Where permissions are stored

Koushi keeps them in `connections.json` in its data directory:

- macOS: `~/Library/Application Support/koushi-desktop/mcp/connections.json`
- Windows: `%LOCALAPPDATA%\koushi-desktop\mcp\connections.json`
- Linux: `~/.local/share/koushi-desktop/mcp/connections.json`

You normally never edit it. The file is re-read on every tool call, so a hand
edit applies immediately:

```json
{
  "schema": 2,
  "connections": [
    {
      "client_name": "claude-code",
      "accounts": [
        { "account_id": "account:@you:example.org", "read": true, "send": false, "attachments": false }
      ]
    }
  ]
}
```

Files written by earlier versions (`accounts` as a list of ids with
`allow_read`, `allow_send`, and `allow_attachments`) still work and are upgraded
the next time Koushi saves a change. If Koushi cannot read the file, no assistant
has access and the settings page says so; Koushi does not overwrite a file it
cannot read, so fix or remove it.

## Step 3: use it

Ask in normal language, for example:

- "List my Koushi accounts and rooms."
- "Search my messages for the invoice from last month."
- "Summarise what was said in the Design room today."
- "Reply to that message saying I will send the file tomorrow."
- "Download the attachment on that message."
- "Send report.pdf from my Documents folder to the Design room."

Sending is a normal message from your account. Koushi does not mark it as
written by an assistant. Sending and replying work even when the target room
is not open in Koushi. They do not switch your selected room or change a message
you are drafting.

## New-message notification

Koushi cannot wake an idle assistant, so the assistant polls `wait_for_events`.
While Koushi is running, an internal poller samples each joined room about every
two seconds and records one entry for every room whose latest message changed
since the previous sample. `wait_for_events` returns immediately when something
is waiting, otherwise it blocks up to `timeout_s` (default 25, maximum 50
seconds).

This is a change feed, not a message log:

- Several messages posted between two samples become one entry, for the newest
  of them. Invites are not reported.
- The first sample after Koushi or the bridge starts only sets a baseline.
- Koushi keeps at most 500 entries in memory, across all accounts. Reading a
  room does not remove its entry; the unread counts in an entry are
  informational.
- An empty result means no change was sampled, not that nobody posted.
- Every result carries `"coverage": "sampled_room_changes"` in its structured
  content, so a client does not have to infer this.

The cursor is opaque and valid only for the account it came from and for the
current Koushi run. Koushi rejects a cursor it cannot continue from with the
error `resync_required` and one of these reasons, instead of quietly starting
over:

| Reason | Meaning |
| --- | --- |
| `malformed_cursor` | The cursor was not issued by `wait_for_events` (including the `seq:` cursors of earlier versions). |
| `previous_run` | The cursor is from before Koushi restarted or the bridge was turned off and on. |
| `wrong_account` | The cursor was issued for another account. |
| `future_cursor` | The cursor is ahead of the newest entry. |
| `expired_cursor` | Entries after the cursor were dropped from the 500-entry window. |

To recover, an assistant:

1. Calls `wait_for_events` without a cursor. This starts at the newest position
   and does not return earlier changes.
2. Calls `read_room_messages` for the rooms it cares about to see what it may
   have missed, following `next` for older pages as far as the room's history
   allows.
3. Skips messages it already handled, keyed by account, room, and event id.

A message posted between steps 1 and 2 can appear both in the history and as a
later change; de-duplicating by event id handles it. `list_rooms` with
`unread_only` is not a catch-up: a room you already read in Koushi drops out of
it.

## What assistant calls never change

Tool calls run inside Koushi's own runtime, but they are not GUI actions:

- `search_messages` does not open the search pane, replace its query or
  results, or cancel a search you are typing. Each call runs its own search
  with the room filter you passed and returns only to that caller.
- `get_attachment` reads the message from the account, not from an open
  timeline, so it works for rooms you have never opened. Files land in Koushi's
  media cache.
- `send_message` sends without touching a draft or the composer. The message
  appears in an open room through normal sync; the assistant's request and its
  errors stay with the assistant.

## Checking a connection

`initialize` and `tools/list` are answered by the launcher itself: they succeed
whether or not Koushi is running, so they do not show that the app is reachable.
Only a tool call crosses the socket. Confirm a connection with the read-only
`list_accounts` before reporting success or failure.

- If the execution environment already exposes Koushi's tools, use them rather
  than starting a second launcher.
- `koushi-mcp --client-name <name>` started by hand prints nothing until a
  request arrives on stdin. Silence is the waiting state, not a crash.

## Sandboxes and approvals

Two failures look similar and mean different things:

| Symptom | Likely cause | Next step |
| --- | --- | --- |
| `koushi_unavailable` | Koushi is closed, its socket is gone, or the endpoint file is unreadable. | Open Koushi and retry. On macOS the socket is `~/Library/Application Support/koushi-desktop/mcp/mcp.sock`. |
| `Operation not permitted` | The assistant's execution environment refused the local connection; Koushi was never asked. | Use the environment's own escalated execution for this call. |

- Request escalation through the mechanism the environment documents, for
  example `require_escalated` where it is supported. Name the exact operation and
  keep the scope to the socket access that call needs.
- Do not route the same call through another assistant or process to get around a
  refusal, and do not disable a sandbox or approval prompt wholesale to make the
  tools reachable.
- Koushi's grant, the environment's permission, and the user's request are three
  separate decisions. A working connection does not authorize sending, editing,
  or changing settings, and one successful test call is not ongoing access.

## Limitations

- `read_room_messages` decrypts each page before returning it, so a message
  this device cannot decrypt comes back with its sender and time but no text.
  Only message-like events are returned; membership and other state events are
  skipped.
- Search results cover what the local index has crawled; a very recent message
  may not be searchable yet. `read_room_messages` reads the room directly and
  does not depend on the crawl.
- Replies are supported with `reply_to_event_id`; starting a new thread is not.
- `send_message` sends files from absolute local paths that Koushi can read: at
  most 10 files, 50 MB each and 100 MB per call. Each file becomes its own
  message, and `body` captions the first one. A file cannot be sent as a reply,
  and files are sent as they are, without resizing. Koushi checks and reads every
  file before sending the first; if a later file fails to send, the error says
  how many were already sent.
- `get_attachment` returns one file per call, always the first attachment of the
  message. Koushi chooses the destination inside its media cache; the assistant
  cannot write elsewhere.
- Permissions are per account, not per room.
- On Windows, the bundled `koushi-mcp.exe` is unsigned, like the installer.

## Troubleshooting

| Message | Meaning | Fix |
| --- | --- | --- |
| "… was not found on this computer" after **Register** | Koushi could not find the assistant's command on your login shell's `PATH` or in the usual install folders. | Install the assistant, or select **Copy** and run the command in a terminal. |
| "… uses a launcher from an older Koushi" | A Claude Desktop extension (or another copy of the launcher) made with an earlier Koushi tried to connect. Registrations made with **Register** point at the launcher inside Koushi and update with it. | Create the Claude Desktop extension again. For another assistant, remove its old `koushi` entry (for example `claude mcp remove koushi -s user`) and register again. |
| `koushi_unavailable` | Koushi is not running, **Let AI assistants connect** is off, or the bridge stopped while the call waited. | Open Koushi, check **App Settings → AI assistants**, and retry. |
| `Operation not permitted` | The execution environment blocked the local socket connection before Koushi saw it. | Ask the environment for its escalated execution on that call; do not disable its sandbox. |
| `consent_required` | Nobody answered the approval in Koushi in time. | Answer the dialog in Koushi, then retry. |
| `not_permitted` | You denied this permission, or removed the assistant while it waited. | Turn the permission on in **App Settings → AI assistants**, then retry. |
| `account_unavailable` | The account is not signed in yet: signed out, locked, or still restoring. | Sign in, unlock Koushi, or wait for restore to finish, then retry. |
| `room_not_found` | The room is unknown to that account or you have left it. | List rooms again and use a room id the account is joined to. |
| `event_not_found` | The event id is unknown to the room. | Re-run `search_messages` for a current event id. |
| `busy` | This account already runs the maximum number of concurrent searches from assistants, or too many approvals are waiting. | Answer or close waiting approvals, wait for earlier calls, and retry. |
| `internal: Koushi could not save the approval` | `connections.json` could not be written or read. | See the message in **App Settings → AI assistants**; fix or remove the file. |
| `unsupported` | The assistant asked for a tool this build does not implement. | Update Koushi. |

If the launcher exits with "client name must be 1-64 characters", choose a
`--client-name` made of letters, digits, `.`, `_`, and `-`.
If the launcher reports it cannot locate the endpoint file, pass
`--endpoint` explicitly. If Koushi refuses to read its own endpoint file,
check that the file is readable only by you (`chmod 600`).


## Recent topics across rooms

Ask the assistant to call `read_recent_activity` with an explicit `account_id`.
It reads the same bounded observed **Activity → Recent** feed while Activity is
closed, including already-read messages. It does not open a room, mark messages
read, clear unread counts or send receipts. Muted/low-priority exclusions and
Activity notification filters apply. This is up to 200 observed rows, not a
complete Matrix archive; no history resolver starts for the assistant. Unknown
read state and unavailable previews are explicit.

Optional `room_ids`, inclusive `from_timestamp_ms`, exclusive `to_timestamp_ms`,
and `limit` (1–100, default 50) restrict the result. Continue with the returned
opaque cursor and identical filters/limit. Pages are immutable, at most 256 KiB,
so a large preview can yield fewer than the requested rows. Preview and label
truncation are reported. Snapshots expire after 120 seconds, restart, access or
session changes, or bounded-cache eviction. `resync_required` means start a new
read without a cursor; it cannot restore evicted history. Use
`read_room_messages` for available context and deduplicate account/room/event IDs.
An empty or exhausted feed does not establish that no messages occurred.

Connected assistants' permission tables label accounts by their complete Matrix
IDs, including homeservers, so matching display names remain distinguishable.
