//! Every MCP tool is reachable from the CLI, with help rendered from its
//! definition, so a tool added to the table without CLI coverage fails here.
use crate::support::cli;
use koushi_mcp::tools::{ToolDefinition, definitions};
use serde_json::Value;

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
        for (name, _) in properties(&tool) {
            if koushi_mcp::cli::STDIN_ONLY_PROPERTIES.contains(&name.as_str()) {
                // Private text has no option; help names the stdin path.
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
fn no_property_generates_a_cli_option_and_every_private_name_is_a_property() {
    for tool in definitions() {
        for (name, _) in properties(&tool) {
            assert!(
                !CLI_OPTIONS.contains(&option(name).as_str()),
                "{}.{name}",
                tool.name
            );
        }
    }
    for private in koushi_mcp::cli::STDIN_ONLY_PROPERTIES {
        assert!(
            definitions()
                .iter()
                .any(|tool| tool.input_schema["properties"].get(private).is_some()),
            "{private} names no tool property"
        );
    }
}
