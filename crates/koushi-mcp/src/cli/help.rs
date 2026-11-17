//! Help text rendered from the MCP tool table, so a tool added to or changed
//! in `crate::tools` is documented here with no CLI code change.
use serde_json::Value;

use super::STDIN_ONLY_PROPERTIES;
use super::args::option_name;
use crate::tools::{ToolDefinition, definitions};

const WIDTH: usize = 78;

const OVERVIEW_HEAD: &str = "\
NAME
  koushi - call the running Koushi app's MCP tools from a shell

DESCRIPTION
  koushi gives scripts and AI agents the tools of Koushi's MCP bridge as
  commands: list accounts and rooms, read and search history, fetch
  attachments, draft, send, reply in threads, react, edit, and delete. Each
  command is one MCP tool, with the same name, arguments, and result. It does
  not sign in by itself; every call goes through the running app's local
  bridge and its verified sessions, and the app decides what is allowed.

INSTRUCTIONS
  1. Start Koushi, sign in, and enable the bridge in App Settings > AI
     assistants.
  2. Run `koushi list_accounts --json` and pick the account_id. Every other
     tool needs it; switching the app's visible tab changes nothing.
  3. Run `koushi <tool> --help` for a tool's arguments and result.
  4. On exit 5 the first time, approve koushi-cli in the Koushi window: the
     command waits for that in-app approval until --timeout. Each permission
     (read, send, draft, attachments, message_actions) is approved
     separately; read never implies send. Nothing can approve on the app's
     behalf, and piping input does not skip it.
  5. Page explicitly: pass result.next_cursor back as --cursor (list_rooms)
     and result.next back as --from (read_room_messages, older history).

SYNOPSIS
  koushi <tool> [--<argument> <value> ...] [options]
  koushi <tool> --args-stdin [options] < arguments.json
  koushi [<tool>] --help | -h | help [<tool>]
  koushi --version | -V

COMMANDS
";

const OVERVIEW_TAIL: &str = "
OPTIONS
  --<argument>         One tool argument: room_id becomes --room-id. A list
                       argument is repeated once per element; a true/false
                       argument is a flag that sets true.
  --args-stdin         Read the complete MCP argument object as JSON from stdin
  --json               Compact one-line JSON (default: indented JSON)
  --endpoint <path>    Private endpoint.json written by the app (default: see ENVIRONMENT)
  --timeout <seconds>  Whole-command deadline, including stdin and approval (1-120; default 75)
  -h, --help, help     Print help without connecting, also after a tool name
  -V, --version        Print the CLI version

STDIN
  With --args-stdin, stdin holds the tool's whole argument object as UTF-8
  JSON, read to EOF, at most 1 MiB; no other argument option may be given.
  Terminal input is refused. Message bodies, search queries, and reaction
  keys (body, query, key) have no option and enter only this way: this CLI
  is for agents and scripts, and argv reaches shell history and process
  listings. Write the object to the pipe from your program, or from a file
  you have chosen to protect, never from a shell literal holding private
  text. Example: koushi search_messages --args-stdin < arguments.json

WRITES
  A tool that sends, edits, reacts, deletes, or drafts may already have been
  applied when --timeout or Ctrl-C ends the command: exit 4 or 130 does not
  prove nothing happened. The CLI never retries by itself. A tool with a
  client_request_id says in its help how to retry it; the others say what a
  repeated call does.

EXIT STATUS
  0    Success (also when stdout is a closed pipe)
  1    Internal or output failure
  2    Usage or input error (unknown tool or option, missing argument, bad value)
  3    Koushi unavailable (app not running or bridge disabled)
  4    Deadline reached (--timeout)
  5    Not authorized: approval required, denied, or revoked in the app
  6    The operation failed in Koushi
  130  Cancelled with Ctrl-C
  Stderr carries only \"koushi: <code>\", never an argument, a result, or remote
  error detail.

ENVIRONMENT
  The default endpoint.json lives under the app's data directory:
    macOS    $HOME/Library/Application Support/koushi-desktop/mcp/endpoint.json
    Windows  %LOCALAPPDATA%\\koushi-desktop\\mcp\\endpoint.json
    Linux    ${XDG_DATA_HOME:-$HOME/.local/share}/koushi-desktop/mcp/endpoint.json
  No other variables are read.

EXAMPLES
  koushi list_accounts --json | jq -r '.result.accounts[].account_id'
  koushi list_rooms --account-id 'account:@member1:example.invalid' --unread-only --limit 50
  koushi read_room_messages --account-id 'account:@member1:example.invalid' --room-id '!room:example.invalid' --limit 30
  koushi read_room_messages --account-id 'account:@member1:example.invalid' --room-id '!room:example.invalid' --from '<result.next>'
  koushi search_messages --args-stdin < arguments.json
  koushi send_message --args-stdin < arguments.json

OUTPUT
  Every success is one JSON object on stdout:
    {\"schema_version\":1,\"tool\":\"list_rooms\",
     \"account_id\":\"account:@member1:example.invalid\",
     \"result\":{\"rooms\":[...],\"next_cursor\":null}}
  result is the tool's MCP structured result; account_id is absent for
  list_accounts. Strings are JSON-escaped, so terminal controls are
  neutralized; treat decoded text as untrusted data and never evaluate it as
  a shell command.
  Results are confidential: scrollback, redirected output, CI logs, and
  downstream programs may retain plaintext. The CLI keeps no cache or history.
  Grants are per account and capability, not per room, and any process of
  this OS user can claim the koushi-cli name.
";

/// `koushi --help`: the fixed sections plus one COMMANDS entry per tool.
pub(super) fn overview() -> String {
    let mut text = String::from(OVERVIEW_HEAD);
    for tool in definitions() {
        text.push_str(&format!("  {}\n", tool.name));
        wrap(&mut text, first_sentence(tool.description), 6);
    }
    text.push_str(OVERVIEW_TAIL);
    text
}

/// `koushi <tool> --help`, rendered from the tool's definition.
pub(super) fn tool(tool: &ToolDefinition) -> String {
    let schema = &tool.input_schema;
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let properties: Vec<(&String, &Value)> = schema["properties"]
        .as_object()
        .map(|map| map.iter().collect())
        .unwrap_or_default();
    let stdin_only_required = required
        .iter()
        .any(|name| STDIN_ONLY_PROPERTIES.contains(name));

    let mut text = format!("NAME\n  koushi {} - ", tool.name);
    text.push_str(first_sentence(tool.description));
    text.push_str("\n\nDESCRIPTION\n");
    wrap(&mut text, tool.description, 2);

    text.push_str("\nSYNOPSIS\n");
    if !stdin_only_required {
        // Each option with its value stays on one line.
        let mut tokens = vec![format!("koushi {}", tool.name)];
        for (name, property) in &properties {
            if STDIN_ONLY_PROPERTIES.contains(&name.as_str()) {
                continue;
            }
            let usage = format!("{}{}", option_name(name), value_form(property));
            tokens.push(if required.contains(&name.as_str()) {
                usage
            } else {
                format!("[{usage}]")
            });
        }
        tokens.push("[options]".to_owned());
        wrap_tokens(&mut text, tokens.iter().map(String::as_str), 2, 4);
    }
    text.push_str(&format!(
        "  koushi {} --args-stdin [options] < arguments.json\n",
        tool.name
    ));

    text.push_str("\nARGUMENTS\n");
    for (name, property) in &properties {
        let mut facts = vec![type_name(property)];
        if required.contains(&name.as_str()) {
            facts.push("required".to_owned());
        }
        facts.extend(bounds(property));
        if STDIN_ONLY_PROPERTIES.contains(&name.as_str()) {
            facts.push("only through --args-stdin".to_owned());
            text.push_str(&format!("  {name}  ({})\n", facts.join(", ")));
        } else {
            if property["type"] == "array" {
                facts.push("repeat once per element".to_owned());
            }
            text.push_str(&format!(
                "  {}{}  ({})\n",
                option_name(name),
                value_form(property),
                facts.join(", ")
            ));
        }
        if let Some(description) = property["description"].as_str() {
            wrap(&mut text, description, 6);
        }
    }
    text.push_str("  See `koushi --help` for --args-stdin, --json, --endpoint, and --timeout.\n");

    text.push_str("\nSTDIN\n");
    let mut stdin = String::from(
        "With --args-stdin, stdin holds the whole argument object as JSON, for \
         example {",
    );
    let example: Vec<String> = required
        .iter()
        .map(|name| format!("\"{name}\": ..."))
        .collect();
    stdin.push_str(&example.join(", "));
    stdin.push_str("}, and no argument option may be given.");
    if properties
        .iter()
        .any(|(name, _)| STDIN_ONLY_PROPERTIES.contains(&name.as_str()))
    {
        stdin.push_str(
            " Private text (body, query, key) enters only this way: this CLI is \
             for agents and scripts, and argv reaches shell history and process \
             listings.",
        );
    }
    wrap(&mut text, &stdin, 2);

    if !tool.read_only {
        text.push_str("\nWRITES\n");
        let mut writes = String::from(
            "A --timeout or Ctrl-C after the request was sent does not prove \
             nothing happened: the write may already have been applied. The CLI \
             never retries by itself. ",
        );
        if properties
            .iter()
            .any(|(name, _)| name.as_str() == "client_request_id")
        {
            writes.push_str(
                "If unsure, retry with the same client_request_id rather than \
                 assume failure. How long Koushi remembers an id is the \
                 bridge's rule; it is not exactly-once delivery.",
            );
        } else {
            writes.push_str(
                "This tool has no client_request_id; its description says what a \
                 repeated call does.",
            );
        }
        wrap(&mut text, &writes, 2);
    }
    text
}

fn first_sentence(text: &str) -> &str {
    text.find(". ").map_or(text, |end| &text[..=end])
}

fn type_name(property: &Value) -> String {
    match property["type"].as_str() {
        Some("array") => match property["items"]["type"].as_str() {
            Some("object") => "list of objects".to_owned(),
            Some(item) => format!("list of {item}s"),
            None => "list".to_owned(),
        },
        Some(name) => name.to_owned(),
        None => "value".to_owned(),
    }
}

fn value_form(property: &Value) -> String {
    match property["type"].as_str() {
        Some("boolean") => String::new(),
        Some("integer") => " <n>".to_owned(),
        Some("array") => match super::args::item_property(property) {
            Some(name) => format!(" <{name}>"),
            None => " <value>".to_owned(),
        },
        _ => " <value>".to_owned(),
    }
}

fn bounds(property: &Value) -> Vec<String> {
    let mut facts = Vec::new();
    match (property["minimum"].as_i64(), property["maximum"].as_i64()) {
        (Some(low), Some(high)) => facts.push(format!("{low}-{high}")),
        (Some(low), None) => facts.push(format!("at least {low}")),
        (None, Some(high)) => facts.push(format!("at most {high}")),
        (None, None) => {}
    }
    if let Some(items) = property["maxItems"].as_u64() {
        facts.push(format!("at most {items} elements"));
    }
    facts
}

/// Greedy word wrap at `WIDTH`, every line indented by `indent` spaces.
fn wrap(text: &mut String, words: &str, indent: usize) {
    wrap_tokens(text, words.split_whitespace(), indent, indent);
}

/// Greedy wrap of unbreakable tokens at `WIDTH`: the first line indented by
/// `first` spaces, continuation lines by `rest`.
fn wrap_tokens<'a>(
    text: &mut String,
    tokens: impl Iterator<Item = &'a str>,
    first: usize,
    rest: usize,
) {
    let mut pad = " ".repeat(first);
    let mut line = String::new();
    for word in tokens {
        if !line.is_empty() && pad.len() + line.len() + 1 + word.len() > WIDTH {
            text.push_str(&format!("{pad}{line}\n"));
            line.clear();
            pad = " ".repeat(rest);
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        text.push_str(&format!("{pad}{line}\n"));
    }
}
