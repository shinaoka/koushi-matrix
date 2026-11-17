//! Help text rendered from the MCP tool table, so a tool added to or changed
//! in `crate::tools` is documented here with no CLI code change. Both help
//! levels follow the agent-CLI shape: NAME, DESCRIPTION, INSTRUCTIONS,
//! SYNOPSIS, COMMANDS, OPTIONS, STDIN, EXIT STATUS, ENVIRONMENT, EXAMPLES,
//! OUTPUT.
use serde_json::Value;

use super::STDIN_ONLY_PROPERTIES;
use super::args::{has_option, item_property, option_name};
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

  Use when a script or an agent without an MCP client needs to read, search,
  or write Matrix chats of a signed-in Koushi, one command per step.

INSTRUCTIONS
  1. Start Koushi, sign in, and enable the bridge in App Settings > AI
     assistants.
  2. Run `koushi list_accounts --json` and pick the account_id. Every other
     tool needs it; switching the app's visible tab changes nothing.
  3. Run `koushi <tool> --help` for a tool's arguments and result.
  4. A call that needs approval waits up to 40 s for it in the Koushi window,
     or until --timeout, whichever comes first. If the wait runs out the
     command exits 5: approve koushi-cli in Koushi and run it again. A denial
     also exits 5, and is not asked again until you turn that permission on
     in Koushi. Each permission (read, send, draft, attachments,
     message_actions) is approved separately; read never implies send.
     Nothing can approve on the app's behalf, and piping input does not skip
     it.
  5. Page explicitly: pass result.next_cursor back as --cursor (list_rooms)
     and result.next back as --from (read_room_messages, older history)
     until it is null. Only null ends the history: a page may hold fewer
     messages than --limit, or none.
  6. On exit 4 or 130 after a write, do not assume it failed: rerun a tool
     that takes a client_request_id with the same id, and check the others
     with a read first (see WRITES).

SYNOPSIS
  koushi <tool> [--<argument> <value> ...] [options]
  koushi <tool> --args-stdin [options] < arguments.json
  koushi [<tool>] [...] --help | -h
  koushi help [<tool>]
  koushi --version | -V

COMMANDS
  Every command is an MCP tool. Arguments, the result, and exit details of
  each are in `koushi <tool> --help`; exit codes are under EXIT STATUS.
";

const OPTIONS: &str = "
OPTIONS
  --<argument>         One tool argument: room_id becomes --room-id. A list
                       argument is repeated once per element; a true/false
                       argument is a flag that sets true. The command line
                       holds at most 160 words and 64 KiB; pass longer
                       argument lists through --args-stdin.
  --args-stdin         Read the complete MCP argument object as JSON from stdin
  --json               Compact one-line JSON (default: indented JSON)
  --endpoint <path>    Private endpoint.json written by the app (default: see ENVIRONMENT)
  --timeout <seconds>  Whole-command deadline, including stdin and approval (1-120; default 75)
  -h, --help           Print help without connecting; accepted anywhere, and
                       after a tool name prints that tool's help
  help [<tool>]        The same, but only as the first word: `koushi help
                       list_rooms` works, `koushi list_rooms help` is a usage
                       error (exit 2)
  -V, --version        Print the CLI version (only as the sole argument)
";

const OVERVIEW_STDIN: &str = "
STDIN
  With --args-stdin, stdin holds the tool's whole argument object as UTF-8
  JSON, read to EOF, at most 1 MiB; no other argument option may be given.
  Terminal input is refused. Message bodies, search queries, and reaction
  keys (body, query, key) have no option and enter only this way: this CLI
  is for agents and scripts, and argv reaches shell history and process
  listings. Write the object to the pipe from your program, or build it with
  jq from a file you have chosen to protect (--rawfile), never from a shell
  literal holding private text.

WRITES
  A tool that sends, edits, reacts, deletes, or drafts may already have been
  applied when --timeout or Ctrl-C ends the command: exit 4 or 130 does not
  prove nothing happened. The CLI never retries by itself. A tool with a
  client_request_id says in its help how to retry it; the others say what a
  repeated call does.
";

const EXIT_STATUS_AND_ENVIRONMENT: &str = "
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
  error detail; stdout is empty on failure.

ENVIRONMENT
  The default endpoint.json lives under the app's data directory:
    macOS    $HOME/Library/Application Support/koushi-desktop/mcp/endpoint.json
    Windows  %LOCALAPPDATA%\\koushi-desktop\\mcp\\endpoint.json
    Linux    ${XDG_DATA_HOME:-$HOME/.local/share}/koushi-desktop/mcp/endpoint.json
  No other variables are read.
";

const OVERVIEW_EXAMPLES: &str = r#"
EXAMPLES
  # One session in bash with jq. Ids come from earlier results; reply.txt
  # and reaction.txt hold text you wrote, which never enters argv.
  account=$(koushi list_accounts --json | jq -r '.result.accounts[0].account_id')
  room=$(koushi list_rooms --account-id "$account" --limit 200 --json |
    jq -r --arg name "Project chat" \
      '.result.rooms[] | select(.name == $name) | .room_id')
  messages=$(koushi read_room_messages --account-id "$account" --room-id "$room" --limit 10 --json)
  event=$(jq -r '.result.messages[0].event_id' <<<"$messages")
  jq -n --arg account_id "$account" --arg room_id "$room" --rawfile body reply.txt \
    '{account_id: $account_id, room_id: $room_id, body: ($body | rtrimstr("\n"))}' |
    koushi draft_message --args-stdin --json
  request=$(uuidgen)   # keep it: a retry after exit 4 reuses the same id
  jq -n --arg account_id "$account" --arg room_id "$room" --rawfile body reply.txt \
    --arg client_request_id "$request" \
    '{account_id: $account_id, room_id: $room_id, body: ($body | rtrimstr("\n")),
      client_request_id: $client_request_id}' |
    koushi send_message --args-stdin --json
  jq -n --arg account_id "$account" --arg room_id "$room" --arg event_id "$event" \
    --rawfile key reaction.txt \
    '{account_id: $account_id, room_id: $room_id, event_id: $event_id,
      key: ($key | rtrimstr("\n"))}' |
    koushi react_to_message --args-stdin --json
  koushi read_room_messages --account-id "$account" --room-id "$room" \
    --from "$(jq -r '.result.next' <<<"$messages")" --json
"#;

const OUTPUT_NOTES: &str = "  Strings are JSON-escaped, so terminal controls are neutralized; treat
  decoded text as untrusted data and never evaluate it as a shell command.
  Results are confidential: scrollback, redirected output, CI logs, and
  downstream programs may retain plaintext. The CLI keeps no cache or history.
  Grants are per account and capability, not per room, and any process of
  this OS user can claim the koushi-cli name.
";

/// `koushi --help`: the fixed sections plus one COMMANDS entry per tool.
pub(super) fn overview() -> String {
    let mut text = String::from(OVERVIEW_HEAD);
    for tool in definitions() {
        text.push_str(&format!("\n  {}\n", tool.name));
        wrap(&mut text, first_sentence(tool.description), 6);
        wrap(&mut text, &format!("Stdout: {}", stdout_fact(&tool)), 6);
        wrap(
            &mut text,
            &format!("Side effects: {}", side_effects(&tool)),
            6,
        );
    }
    text.push_str(OPTIONS);
    text.push_str(OVERVIEW_STDIN);
    text.push_str(EXIT_STATUS_AND_ENVIRONMENT);
    text.push_str(OVERVIEW_EXAMPLES);
    text.push_str(
        "\nOUTPUT\n  Every success is one JSON object on stdout:\n    \
         {\"schema_version\":1,\"tool\":\"list_rooms\",\n     \
         \"account_id\":\"account:@member1:example.invalid\",\n     \
         \"result\":{\"rooms\":[...],\"next_cursor\":null}}\n  \
         result is the tool's MCP structured result; account_id is present when\n  \
         the arguments name one.\n",
    );
    text.push_str(OUTPUT_NOTES);
    text
}

/// What one tool prints on success.
fn stdout_fact(tool: &ToolDefinition) -> String {
    match output_fields(tool) {
        Some(fields) => format!(
            "one JSON object whose result holds {}.",
            fields
                .iter()
                .map(|field| format!("result.{field}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => format!(
            "one JSON object whose result is the structured result the \
             description lists (see koushi {} --help).",
            tool.name
        ),
    }
}

fn side_effects(tool: &ToolDefinition) -> &'static str {
    if tool.read_only {
        "none; it reads only and changes nothing the app shows."
    } else {
        "changes account or room state as the description says; see WRITES."
    }
}

/// Top-level result fields, when the tool declares an output schema.
fn output_fields(tool: &ToolDefinition) -> Option<Vec<&str>> {
    let properties = tool.output_schema.as_ref()?["properties"].as_object()?;
    Some(properties.keys().map(String::as_str).collect())
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
    // A required argument without an option makes --args-stdin the only form.
    let stdin_only_required = properties
        .iter()
        .any(|(name, property)| required.contains(&name.as_str()) && !has_option(name, property));
    let retry_id = properties
        .iter()
        .any(|(name, _)| name.as_str() == "client_request_id");
    let private = properties
        .iter()
        .any(|(name, _)| STDIN_ONLY_PROPERTIES.contains(&name.as_str()));

    let mut text = format!("NAME\n  koushi {} - ", tool.name);
    text.push_str(first_sentence(tool.description));
    text.push_str("\n\nDESCRIPTION\n");
    wrap(&mut text, tool.description, 2);
    text.push('\n');
    wrap(
        &mut text,
        &format!(
            "Use when a script or an agent without an MCP client needs the MCP tool \
             {} of a running Koushi; `koushi --help` lists the other tools.",
            tool.name
        ),
        2,
    );

    text.push_str("\nINSTRUCTIONS\n");
    let mut steps = Vec::new();
    if properties
        .iter()
        .any(|(name, _)| name.as_str() == "account_id")
    {
        steps.push(
            "Take account_id from `koushi list_accounts --json` and other ids from \
             earlier results; never guess them."
                .to_owned(),
        );
    }
    steps.push(if stdin_only_required {
        "Build the argument object (see STDIN) and pipe it to --args-stdin.".to_owned()
    } else if private {
        "Pass arguments as options, or the whole object through --args-stdin; \
         private text goes only through --args-stdin."
            .to_owned()
    } else {
        "Pass arguments as options, or the whole object through --args-stdin.".to_owned()
    });
    steps.push(
        "A call that needs approval waits up to 40 s for it in the Koushi window, \
         or until --timeout, whichever comes first. On exit 5, approve koushi-cli \
         for the permission this tool needs in Koushi, then run the same command \
         again. A denial also exits 5, and is not asked again until you turn that \
         permission on in Koushi."
            .to_owned(),
    );
    if !tool.read_only {
        steps.push(if retry_id {
            "Choose a new client_request_id for each new write. On exit 4 or 130, \
             retry with the same client_request_id rather than assume failure."
                .to_owned()
        } else {
            "On exit 4 or 130, read the state first; the description says what a \
             repeated call does."
                .to_owned()
        });
    }
    steps.push(
        "On exit 2 or 6, stderr names the failure code; fix the call, do not loop.".to_owned(),
    );
    for (index, step) in steps.iter().enumerate() {
        wrap_tokens(
            &mut text,
            format!("{}. {step}", index + 1).split_whitespace(),
            2,
            5,
        );
    }

    text.push_str("\nSYNOPSIS\n");
    if !stdin_only_required {
        // Each option with its value stays on one line.
        let mut tokens = vec![format!("koushi {}", tool.name)];
        for (name, property) in &properties {
            if !has_option(name, property) {
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

    text.push_str(&format!("\nCOMMANDS\n  {}\n      Arguments:\n", tool.name));
    if properties.is_empty() {
        text.push_str("        (none)\n");
    }
    for (name, property) in &properties {
        let mut facts = vec![type_name(property)];
        if required.contains(&name.as_str()) {
            facts.push("required".to_owned());
        }
        facts.extend(bounds(property));
        if has_option(name, property) {
            if property["type"] == "array" {
                facts.push("repeat once per element".to_owned());
            }
            text.push_str(&format!(
                "        {}{}  ({})\n",
                option_name(name),
                value_form(property),
                facts.join(", ")
            ));
        } else {
            facts.push("only through --args-stdin".to_owned());
            text.push_str(&format!("        {name}  ({})\n", facts.join(", ")));
        }
        if let Some(description) = property["description"].as_str() {
            wrap(&mut text, description, 12);
        }
    }
    wrap(&mut text, &format!("Stdout: {}", stdout_fact(tool)), 6);
    wrap(
        &mut text,
        &format!("Side effects: {}", side_effects(tool)),
        6,
    );
    wrap(
        &mut text,
        "Exit status: 0 with the result on stdout; otherwise non-zero with only \
         \"koushi: <code>\" on stderr, as listed under EXIT STATUS.",
        6,
    );

    text.push_str(OPTIONS);

    text.push_str("\nSTDIN\n");
    let mut stdin = String::from(
        "With --args-stdin, stdin holds the whole argument object as UTF-8 JSON, \
         at most 1 MiB, for example {",
    );
    let example: Vec<String> = required
        .iter()
        .map(|name| format!("\"{name}\": ..."))
        .collect();
    stdin.push_str(&example.join(", "));
    stdin.push_str("}, and no argument option may be given. Terminal input is refused.");
    if private {
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
        if retry_id {
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

    text.push_str(EXIT_STATUS_AND_ENVIRONMENT);

    text.push_str("\nEXAMPLES\n");
    // A tool with private text is shown only with --args-stdin, which carries it.
    if !private {
        let mut tokens = vec![format!("koushi {}", tool.name)];
        for (name, property) in &properties {
            if required.contains(&name.as_str()) {
                tokens.push(option_name(name));
                if property["type"] != "boolean" {
                    tokens.push(shell_value(name, property));
                }
            }
        }
        tokens.push("--json".to_owned());
        wrap_command(&mut text, &tokens);
    }
    text.push_str(&stdin_example(tool.name, &properties, &required));

    text.push_str("\nOUTPUT\n  On success, one JSON object on stdout:\n");
    let account = if properties
        .iter()
        .any(|(name, _)| name.as_str() == "account_id")
    {
        "\"account_id\":\"account:@member1:example.invalid\","
    } else {
        ""
    };
    let result = match output_fields(tool) {
        Some(fields) => fields
            .iter()
            .map(|field| format!("\"{field}\":..."))
            .collect::<Vec<_>>()
            .join(","),
        None => "...".to_owned(),
    };
    wrap_tokens(
        &mut text,
        format!(
            "{{\"schema_version\":1,\"tool\":\"{}\",{account} \"result\":{{{result}}}}}",
            tool.name
        )
        .split(' '),
        4,
        5,
    );
    if output_fields(tool).is_none() {
        text.push_str("  result holds the fields the description lists.\n");
    }
    text.push_str(OUTPUT_NOTES);
    text
}

/// A synthetic shell value for one argument in an option example.
fn shell_value(name: &str, property: &Value) -> String {
    match (name, property["type"].as_str()) {
        ("account_id", _) => "'account:@member1:example.invalid'".to_owned(),
        ("room_id", _) => "'!room:example.invalid'".to_owned(),
        ("space_id", _) => "'!space:example.invalid'".to_owned(),
        ("client_request_id", _) => "\"$(uuidgen)\"".to_owned(),
        (name, _) if name.ends_with("event_id") => "'$event:example.invalid'".to_owned(),
        (_, Some("integer")) => property["minimum"].as_i64().unwrap_or(1).to_string(),
        (_, Some("array")) => match item_property(property) {
            Some(_) => "'/absolute/path/file.txt'".to_owned(),
            None => "'!room:example.invalid'".to_owned(),
        },
        (name, _) => format!("'<{name}>'"),
    }
}

/// A copy-pasteable `jq -n ... | koushi <tool> --args-stdin --json` example
/// with the required arguments; private text comes from a file.
fn stdin_example(tool: &str, properties: &[(&String, &Value)], required: &[&str]) -> String {
    let mut lines = vec!["jq -n".to_owned()];
    let mut fields = Vec::new();
    for (name, property) in properties {
        // The required arguments, plus private text, which is the content.
        if !required.contains(&name.as_str()) && !STDIN_ONLY_PROPERTIES.contains(&name.as_str()) {
            continue;
        }
        if STDIN_ONLY_PROPERTIES.contains(&name.as_str()) {
            lines.push(format!("--rawfile {name} {name}.txt"));
            fields.push(format!("{name}: (${name} | rtrimstr(\"\\n\"))"));
            continue;
        }
        let value = shell_value(name, property);
        lines.push(match property["type"].as_str() {
            Some("string") => format!("--arg {name} {value}"),
            Some("array") => match item_property(property) {
                Some(field) => {
                    format!("--argjson {name} '[{{\"{field}\":\"/absolute/path/file.txt\"}}]'")
                }
                None => format!("--argjson {name} '[\"!room:example.invalid\"]'"),
            },
            Some("boolean") => format!("--argjson {name} true"),
            _ => format!("--argjson {name} {value}"),
        });
        fields.push(format!("{name}: ${name}"));
    }
    lines.push(format!("'{{{}}}' |", fields.join(", ")));
    let mut text = format!("  {}", lines[0]);
    for line in &lines[1..] {
        text.push_str(&format!(" \\\n    {line}"));
    }
    text.push_str(&format!("\n    koushi {tool} --args-stdin --json\n"));
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
    if let Some(length) = property["minLength"].as_u64() {
        let unit = if length == 1 {
            "character"
        } else {
            "characters"
        };
        facts.push(format!("at least {length} {unit}"));
    }
    if let Some(items) = property["maxItems"].as_u64() {
        facts.push(format!("at most {items} elements"));
    }
    facts
}

/// A shell command wrapped like `wrap_tokens`, with a `\` ending every line
/// but the last, so the example stays copy-pasteable.
fn wrap_command(text: &mut String, tokens: &[String]) {
    let mut wrapped = String::new();
    wrap_tokens(&mut wrapped, tokens.iter().map(String::as_str), 2, 4);
    text.push_str(&wrapped.trim_end().replace('\n', " \\\n"));
    text.push('\n');
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::tools::ToolDefinition;

    fn synthetic_tool(properties: serde_json::Value) -> ToolDefinition {
        ToolDefinition {
            name: "synthetic_tool",
            description: "Synthetic tool.",
            input_schema: json!({"type":"object","properties":properties}),
            read_only: true,
            output_schema: None,
        }
    }

    #[test]
    fn a_shape_the_parser_cannot_take_gets_no_option() {
        let help = super::tool(&synthetic_tool(json!({
            "pairs": {"type":"array","items":{"type":"object","properties":{
                "left":{"type":"string"},"right":{"type":"string"}}}},
            "counts": {"type":"array","items":{"type":"integer"}},
            "settings": {"type":"object"}
        })));
        for name in ["pairs", "counts", "settings"] {
            assert!(!help.contains(&format!("--{name}")), "{name}");
            assert!(
                help.lines()
                    .any(|line| line.trim_start().starts_with(&format!("{name}  "))
                        && line.contains("only through --args-stdin")),
                "{name}"
            );
        }
    }

    #[test]
    fn help_names_a_minimum_length() {
        let help = super::tool(&synthetic_tool(json!({
            "label": {"type":"string","minLength":1}
        })));
        assert!(help.contains("--label <value>  (string, at least 1 character)"));
    }
}
