//! Every MCP tool is reachable from the CLI, with help rendered from its
//! definition, so a tool added to the table without CLI coverage fails here.
use crate::support::cli;
use koushi_mcp::cli::{ARGV_SAFE_PROPERTIES, argv_option};
use koushi_mcp::tools::{ToolDefinition, definitions};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Options the CLI owns itself; no tool property may generate one of them.
const CLI_OPTIONS: [&str; 6] = [
    "--json",
    "--endpoint",
    "--timeout",
    "--args-stdin",
    "--help",
    "--version",
];

fn option(property: &str) -> String {
    format!("--{}", property.replace('_', "-"))
}

fn properties(tool: &ToolDefinition) -> Vec<(&String, &Value)> {
    tool.input_schema["properties"]
        .as_object()
        .expect("properties object")
        .iter()
        .collect()
}

fn help(args: &[&str]) -> String {
    let output = cli().args(args).output().expect("help process");
    assert!(output.status.success(), "{args:?}");
    assert!(output.stderr.is_empty(), "{args:?}");
    String::from_utf8(output.stdout).expect("help utf8")
}

#[test]
fn the_overview_help_lists_every_tool_as_a_command() {
    let overview = help(&["--help"]);
    for tool in definitions() {
        assert!(
            overview.contains(&format!("\n  {}\n", tool.name)),
            "{} missing from COMMANDS",
            tool.name
        );
    }
}

#[test]
fn every_tool_help_has_the_agent_sections_and_states_its_effects() {
    for tool in definitions() {
        let text = help(&[tool.name, "--help"]);
        crate::help::assert_sections(&text, tool.name);
        for fact in ["Stdout:", "Side effects:", "Exit status:"] {
            assert!(text.contains(fact), "{} {fact}", tool.name);
        }
        assert!(
            text.contains(&format!("koushi {} --args-stdin --json", tool.name)),
            "{} stdin example",
            tool.name
        );
        assert!(
            text.contains(&format!("\"tool\":\"{}\"", tool.name)),
            "{} output envelope",
            tool.name
        );
        let flowed = crate::help::flowed(&text);
        for phrase in ["waits up to 40 s", "A denial also exits 5"] {
            assert!(flowed.contains(phrase), "{} {phrase}", tool.name);
        }
        // A generated example must not put a local path in argv: the stdin
        // example reads it from a file instead.
        assert!(
            !text.contains("/absolute/path"),
            "{} example names a path on the command line",
            tool.name
        );
    }
}

#[test]
fn the_overview_states_each_commands_stdout_and_side_effects() {
    let overview = help(&["--help"]);
    for tool in definitions() {
        let start = overview
            .find(&format!("\n  {}\n", tool.name))
            .expect("command entry");
        let entry = &overview[start + 1..];
        // Entries are separated by a blank line.
        let entry = &entry[..entry.find("\n\n").unwrap_or(entry.len())];
        for fact in ["Stdout:", "Side effects:"] {
            assert!(entry.contains(fact), "{} {fact}", tool.name);
        }
    }
}

#[test]
fn every_tool_has_help_naming_each_argument() {
    for tool in definitions() {
        let text = help(&[tool.name, "--help"]);
        // Help wraps the description, so compare word sequences.
        let words = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            words(&text).contains(&words(tool.description)),
            "{} description",
            tool.name
        );
        for (name, property) in properties(&tool) {
            if !argv_option(name, property) {
                // No option: help names the property and the stdin path.
                assert!(!text.contains(&option(name)), "{}.{name}", tool.name);
                assert!(
                    text.lines()
                        .any(|line| line.trim_start().starts_with(&format!("{name}  ("))),
                    "{}.{name}",
                    tool.name
                );
            } else {
                assert!(
                    text.lines().any(|line| line
                        .trim_start()
                        .starts_with(&format!("{} ", option(name)))
                        && line.contains("  (")),
                    "{}.{name}",
                    tool.name
                );
            }
        }
        assert!(text.contains("--args-stdin"), "{}", tool.name);
    }
}

#[test]
fn read_help_names_what_a_read_still_writes() {
    // get_attachment downloads into the media cache; "Side effects: none"
    // would be false.
    let text = help(&["get_attachment", "--help"]);
    assert!(
        crate::help::flowed(&text)
            .contains("downloads the attachment into Koushi's media cache and returns that path"),
        "get_attachment side effects"
    );
}

#[test]
fn write_help_states_the_retry_rule_its_schema_supports() {
    for tool in definitions() {
        let text = help(&[tool.name, "--help"]);
        let retry_id = tool.input_schema["properties"]
            .get("client_request_id")
            .is_some();
        assert_eq!(
            text.contains("\nWRITES\n"),
            !tool.read_only,
            "{}",
            tool.name
        );
        assert_eq!(
            text.contains("retry with the same client_request_id"),
            retry_id,
            "{}",
            tool.name
        );
        if retry_id {
            assert!(!tool.read_only, "{}", tool.name);
        }
    }
}

#[test]
fn no_property_generates_a_cli_option() {
    for tool in definitions() {
        for (name, _) in properties(&tool) {
            assert!(
                !CLI_OPTIONS.contains(&option(name).as_str()),
                "{}.{name}",
                tool.name
            );
        }
    }
}

/// Whether a property can carry message text, a query, or a path: a string,
/// a list of strings, or a list of one-property objects.
fn carries_text(property: &Value) -> bool {
    match property["type"].as_str() {
        Some("string") => true,
        Some("array") => {
            property["items"]["type"] == "string"
                || property["items"]["properties"]
                    .as_object()
                    .is_some_and(|fields| fields.len() == 1)
        }
        _ => false,
    }
}

#[test]
fn every_text_property_is_classified_argv_safe_or_stdin_only() {
    let mut argv = BTreeSet::new();
    let mut stdin_only = BTreeSet::new();
    for tool in definitions() {
        for (name, property) in properties(&tool) {
            if carries_text(property) {
                let set = if argv_option(name, property) {
                    &mut argv
                } else {
                    &mut stdin_only
                };
                set.insert(name.clone());
            } else {
                // A number or a boolean is a bound or a flag, never content.
                assert!(argv_option(name, property), "{} {name}", tool.name);
            }
        }
    }
    // Nothing is silently unclassified: the allowlist is exactly the set of
    // text properties that take an option, so a stale or missing entry fails.
    assert_eq!(
        argv,
        ARGV_SAFE_PROPERTIES
            .iter()
            .map(|name| name.to_string())
            .collect(),
        "argv-safe allowlist and the option-bearing text properties disagree"
    );
    // The content arguments are the stdin-only half, by name.
    for name in ["attachments", "body", "key", "query"] {
        assert!(stdin_only.contains(name), "{name} has an option");
        assert!(!argv.contains(name), "{name} is on the argv-safe allowlist");
    }
    // One path only: nothing is in both halves.
    assert!(argv.is_disjoint(&stdin_only));
}

#[test]
fn a_new_string_property_fails_closed_to_stdin() {
    // A property nobody classified must not gain `--caption <private text>`.
    for name in ["caption", "reason", "text", "topic", "new_body", "path"] {
        for property in [
            json!({"type": "string"}),
            json!({"type": "array", "items": {"type": "string"}}),
        ] {
            assert!(!argv_option(name, &property), "{name} took an option");
        }
    }
    // While the identifiers already on the allowlist still do, and an array of
    // one-property objects still does not: its value is a local file path.
    assert!(argv_option("room_id", &json!({"type": "string"})));
    assert!(argv_option("limit", &json!({"type": "integer"})));
    assert!(argv_option("thumbnail", &json!({"type": "boolean"})));
    assert!(!argv_option(
        "attachments",
        &json!({"type": "array", "items": {"type": "object",
            "properties": {"path": {"type": "string"}}}})
    ));
}

/// The generated command surface, pinned: one line per tool with its options
/// and its stdin-only arguments. A schema rename or type change shows up here as
/// a CLI break before it reaches a script.
const SURFACE: &str = include_str!("surface.golden.txt");

/// One line per tool: options in schema order (`*` = required), then the
/// arguments that have no option.
fn surface() -> String {
    let mut text = String::new();
    for tool in definitions() {
        let required: Vec<&str> = tool.input_schema["required"]
            .as_array()
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut options = Vec::new();
        let mut stdin_only = Vec::new();
        for (name, property) in properties(&tool) {
            if argv_option(name, property) {
                let mark = if required.contains(&name.as_str()) {
                    "*"
                } else {
                    ""
                };
                options.push(format!("{}{mark}", option(name)));
            } else {
                let mark = if required.contains(&name.as_str()) {
                    "*"
                } else {
                    ""
                };
                stdin_only.push(format!("{name}{mark}"));
            }
        }
        text.push_str(&format!(
            "{} {} | stdin-only: {}\n",
            tool.name,
            options.join(" "),
            if stdin_only.is_empty() {
                "(none)".to_owned()
            } else {
                stdin_only.join(" ")
            }
        ));
    }
    text
}

#[test]
fn the_command_surface_matches_its_golden_file() {
    // A Windows checkout may give the file CRLF endings; the surface is lines.
    assert_eq!(
        surface(),
        SURFACE.replace("\r\n", "\n"),
        "crates/koushi-mcp/tests/cli/surface.golden.txt is stale; run \
         `cargo test -p koushi-mcp --test cli write_command_surface_golden -- --ignored`"
    );
}

#[test]
#[ignore = "writes crates/koushi-mcp/tests/cli/surface.golden.txt; run explicitly after a tool-table change"]
fn write_command_surface_golden() {
    // Runtime lookup: `env!` would key the test build on the checkout path.
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo test sets CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(&manifest_dir).join("tests/cli/surface.golden.txt");
    std::fs::write(path, surface()).expect("write golden");
}
