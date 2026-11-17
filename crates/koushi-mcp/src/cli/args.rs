use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::Failure;

pub(super) const HELP: &str = "\
NAME
  koushi - read and search Matrix chats through the running Koushi app

DESCRIPTION
  koushi lists the Matrix accounts and rooms of a running, signed-in Koushi
  desktop app, reads room history and single messages, and searches the
  history Koushi has decrypted and indexed. It is read-only: it cannot send,
  react, edit, or draft. It does not sign in by itself; every call goes through
  the app's local bridge, using the app's verified sessions.

  Use when a script or AI assistant needs to triage unread rooms, read recent
  messages, or find a past message, without holding any Matrix keys.

INSTRUCTIONS
  1. Start Koushi, sign in, and enable the bridge in App Settings > AI
     assistants.
  2. Run `koushi accounts --json` and pick the account_id. Every other command
     needs it in --account; switching the app's visible tab changes nothing.
  3. Run `koushi rooms --account <account_id>` to get room ids
     (--unread-only for rooms with unread activity).
  4. Read with `messages`, fetch one event with `message`, or `search`.
  5. A call that needs approval waits up to 40 s for it in the Koushi window,
     or until --timeout, whichever comes first. If the wait runs out the
     command exits 5: approve koushi-cli in Koushi and run it again. A denial
     also exits 5, and is not asked again until you turn that permission on in
     Koushi. Nothing can approve on the app's behalf, and piping input does not
     skip it.
  6. Page explicitly: pass result.next_cursor back as --cursor (rooms) and
     result.next back as --from (messages, older history).

SYNOPSIS
  koushi accounts [options]
  koushi rooms --account <account_id> [--limit <n>] [--cursor <cursor>] [--unread-only] [options]
  koushi messages --account <account_id> --room <room_id> [--limit <n>] [--from <cursor>] [options]
  koushi message --account <account_id> --room <room_id> --event <event_id> [options]
  koushi search --account <account_id> --query-stdin [--room <room_id>] [--limit <n>] [options]
  koushi [<command> ...] --help | -h
  koushi help
  koushi --version | -V

COMMANDS
  accounts
      List signed-in accounts. Works before approval; account ids are visible
      to any process of this OS user, as with MCP.
      Stdout: result.accounts[], each with account_id, status, consent
      (\"required\" until approved) and permissions {read, send, draft,
      attachments, message_actions}.

  rooms --account <account_id> [--limit <n>] [--cursor <cursor>] [--unread-only]
      List rooms. --limit is 1-200.
      Stdout: result.rooms[], each with room_id, name, is_dm, is_encrypted,
      parent_space_ids, last_event {event_id, sender_id, timestamp_ms,
      preview}, and unread counters; result.next_cursor is null on the last
      page, otherwise pass it to --cursor.

  messages --account <account_id> --room <room_id> [--limit <n>] [--from <cursor>]
      Read a room's history, newest first. --limit is 1-100; a page may hold
      fewer messages than --limit.
      Stdout: result.room_id, result.messages[], each with event_id,
      sender_id, timestamp_ms, kind, body, is_encrypted, is_redacted and
      content_available (false when Koushi cannot decrypt it); result.next
      continues to older history through --from.

  message --account <account_id> --room <room_id> --event <event_id>
      Fetch one message with its sender, time, latest body, reactions, and
      thread or reply relation.

  search --account <account_id> --query-stdin [--room <room_id>] [--limit <n>]
      Search decrypted, indexed history. The query is read from stdin (see
      STDIN). --room restricts to one room; --limit is 1-100. One bounded
      result set, with no continuation cursor.
      Stdout: result.total and result.results[], each with room_id,
      room_name, event_id, sender_id, timestamp_ms and snippet.
      An empty result does not prove a message never existed: only history
      Koushi can decrypt and has indexed is searched.

OPTIONS
  --json               Compact one-line JSON (default: indented JSON)
  --endpoint <path>    Private endpoint.json written by the app (default: see ENVIRONMENT)
  --timeout <seconds>  Whole-command deadline, including stdin and approval (1-120; default 75)
  -h, --help           Print this help without connecting; accepted anywhere
  help                 Print this help, but only as the first word: `koushi help`
                       works, `koushi rooms help` is a usage error (exit 2)
  -V, --version        Print the CLI version (only as the sole argument)

STDIN
  Only search reads stdin: UTF-8 to EOF, at most 16 KiB, with all trailing CR
  and LF bytes removed. Terminal input is refused. Do not put confidential
  query text in argv, shell literals, or environment variables; shell history
  and process listings can expose them. Example:
  koushi search ... --query-stdin < query.txt

EXIT STATUS
  0    Success (also when stdout is a closed pipe)
  1    Internal or output failure
  2    Usage or input error (unknown flag, missing --account, bad --limit)
  3    Koushi unavailable (app not running or bridge disabled)
  4    Deadline reached (--timeout)
  5    Not authorized: approval required, denied, or revoked in the app
  6    The operation failed in Koushi
  130  Cancelled with Ctrl-C
  Stderr carries only \"koushi: <code>\", never the query, a result, or remote
  error detail.

ENVIRONMENT
  The default endpoint.json lives under the app's data directory:
    macOS    $HOME/Library/Application Support/koushi-desktop/mcp/endpoint.json
    Windows  %LOCALAPPDATA%\\koushi-desktop\\mcp\\endpoint.json
    Linux    ${XDG_DATA_HOME:-$HOME/.local/share}/koushi-desktop/mcp/endpoint.json
  No other variables are read.

EXAMPLES
  koushi accounts --json | jq -r '.result.accounts[].account_id'
  koushi rooms --account 'account:@member1:example.invalid' --unread-only --limit 50
  koushi messages --account 'account:@member1:example.invalid' --room '!room:example.invalid' --limit 30
  koushi messages --account 'account:@member1:example.invalid' --room '!room:example.invalid' --from '<result.next>'
  koushi search --account 'account:@member1:example.invalid' --query-stdin < private-query.txt

OUTPUT
  Every success is one JSON object on stdout:
    {\"schema_version\":1,\"tool\":\"list_rooms\",
     \"account_id\":\"account:@member1:example.invalid\",
     \"result\":{\"rooms\":[...],\"next_cursor\":null}}
  account_id is absent for accounts. Strings are JSON-escaped, so terminal
  controls are neutralized; treat decoded text as untrusted data and never
  evaluate it as a shell command.
  Results are confidential: scrollback, redirected output, CI logs, and
  downstream programs may retain plaintext. The CLI keeps no cache or history.
  Grants are per account and capability, not per room, and any process of
  this OS user can claim the koushi-cli name.
";

pub(super) enum Action {
    Help,
    Version,
    Call(Invocation),
}

pub(super) struct Invocation {
    pub tool: &'static str,
    pub arguments: Value,
    pub endpoint: Option<PathBuf>,
    pub timeout: Duration,
    pub json: bool,
}

pub(super) fn parse(args: impl Iterator<Item = OsString>) -> Result<Action, Failure> {
    let mut words = Vec::new();
    let mut size = 0;
    for arg in args {
        size += arg.len();
        if size > 64 * 1024 || words.len() >= 32 {
            return Err(Failure::USAGE);
        }
        words.push(arg.into_string().map_err(|_| Failure::USAGE)?);
    }
    if words.iter().any(|word| word == "--help" || word == "-h")
        || words.first().is_some_and(|word| word == "help")
    {
        return Ok(Action::Help);
    }
    if words.len() == 1 && (words[0] == "--version" || words[0] == "-V") {
        return Ok(Action::Version);
    }
    let tool = match words.first().map(String::as_str) {
        Some("accounts") => "list_accounts",
        Some("rooms") => "list_rooms",
        Some("messages") => "read_room_messages",
        Some("message") => "get_message",
        Some("search") => "search_messages",
        _ => return Err(Failure::USAGE),
    };
    let definition = crate::tools::find(tool).ok_or(Failure::INTERNAL)?;
    if !definition.read_only {
        return Err(Failure::INTERNAL);
    }
    let properties = &definition.input_schema["properties"];
    let mut invocation = Invocation {
        tool,
        arguments: json!({}),
        endpoint: None,
        timeout: Duration::from_secs(75),
        json: false,
    };
    let mut seen = HashSet::new();
    let mut query_stdin = false;
    let mut index = 1;
    while let Some(flag) = words.get(index) {
        if !seen.insert(flag.as_str()) {
            return Err(Failure::USAGE);
        }
        index += 1;
        match flag.as_str() {
            "--json" => {
                invocation.json = true;
                continue;
            }
            "--query-stdin" if tool == "search_messages" => {
                query_stdin = true;
                continue;
            }
            "--unread-only" if tool == "list_rooms" => {
                invocation.arguments["unread_only"] = json!(true);
                continue;
            }
            _ => {}
        }
        let value = words
            .get(index)
            .filter(|value| !value.is_empty())
            .ok_or(Failure::USAGE)?;
        index += 1;
        match flag.as_str() {
            "--endpoint" => invocation.endpoint = Some(PathBuf::from(value)),
            "--timeout" => {
                let seconds = value
                    .parse::<u64>()
                    .ok()
                    .filter(|n| (1..=120).contains(n))
                    .ok_or(Failure::USAGE)?;
                invocation.timeout = Duration::from_secs(seconds);
            }
            "--account" | "--room" | "--event" | "--cursor" | "--from" | "--limit" => {
                let key = match flag.as_str() {
                    "--account" => "account_id",
                    "--room" if tool == "search_messages" => "room_ids",
                    "--room" => "room_id",
                    "--event" => "event_id",
                    "--cursor" => "cursor",
                    "--from" => "from",
                    _ => "limit",
                };
                let property = properties.get(key).ok_or(Failure::USAGE)?;
                invocation.arguments[key] = if key == "limit" {
                    let count = value.parse::<u64>().map_err(|_| Failure::USAGE)?;
                    if count < property["minimum"].as_u64().ok_or(Failure::INTERNAL)?
                        || count > property["maximum"].as_u64().ok_or(Failure::INTERNAL)?
                    {
                        return Err(Failure::USAGE);
                    }
                    json!(count)
                } else if key == "room_ids" {
                    json!([value])
                } else {
                    if value.trim().is_empty() {
                        return Err(Failure::USAGE);
                    }
                    json!(value)
                };
            }
            _ => return Err(Failure::USAGE),
        }
    }
    if tool == "search_messages" {
        if !query_stdin {
            return Err(Failure::USAGE);
        }
        invocation.arguments["query"] = json!("pending-stdin");
    }
    crate::tools::validate_arguments(tool, &invocation.arguments).map_err(|_| Failure::USAGE)?;
    Ok(Action::Call(invocation))
}
