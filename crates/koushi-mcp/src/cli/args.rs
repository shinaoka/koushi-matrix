use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::Failure;

pub(super) const HELP: &str = "\
koushi - read-only CLI for a running Koushi app

Usage:
  koushi accounts [options]
  koushi rooms --account <account_id> [--limit <n>] [--cursor <cursor>] [--unread-only] [options]
  koushi messages --account <account_id> --room <room_id> [--limit <n>] [--from <cursor>] [options]
  koushi message --account <account_id> --room <room_id> --event <event_id> [options]
  koushi search --account <account_id> --query-stdin [--room <room_id>] [--limit <n>] [options]

Options:
  --json               Compact JSON (default: indented JSON), schema_version 1
  --endpoint <path>    Private endpoint.json (default: platform app-data location)
  --timeout <seconds>  Whole-command deadline, including stdin/consent (1-120; default 75)
  -h, --help           Show help without connecting
  -V, --version        Show CLI version

Enable the bridge in App Settings > AI assistants and approve koushi-cli in
Koushi. Missing permission waits for in-app approval; there is no auto-approval.
Every account operation requires an explicit account; no GUI tab is inferred.
Search reads UTF-8 stdin to EOF (at most 16 KiB); terminal input is refused.
Use a private input source; do not put confidential query text in shell literals.
Results are confidential: scrollback, redirected output, CI logs, and downstream
programs may retain plaintext. The CLI creates no result cache/history.
Grants are account/capability scoped; same-OS-user processes can claim this
client name. This CLI does not isolate processes or provide room allowlists.
History is limited to what Koushi can decrypt/search; pagination is explicit.
Exit: 0 success/broken pipe, 1 internal/output, 2 input/usage, 3 unavailable,
4 deadline, 5 authorization/consent, 6 operation, 130 cancelled.
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
    if words.len() == 1 {
        match words[0].as_str() {
            "--help" | "-h" => return Ok(Action::Help),
            "--version" | "-V" => return Ok(Action::Version),
            _ => {}
        }
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
