//! Help is printed without connecting to the app.
use crate::support::cli;

#[test]
fn help_is_available_without_the_app() {
    let output = cli().arg("--help").output().expect("help process");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help utf8");
    assert!(help.contains("--args-stdin"));
    assert!(help.contains("scrollback"));
    assert!(output.stderr.is_empty());
}

/// The sections an agent-facing help needs, in this order.
pub const SECTIONS: [&str; 11] = [
    "NAME",
    "DESCRIPTION",
    "INSTRUCTIONS",
    "SYNOPSIS",
    "COMMANDS",
    "OPTIONS",
    "STDIN",
    "EXIT STATUS",
    "ENVIRONMENT",
    "EXAMPLES",
    "OUTPUT",
];

/// Asserts every section is a whole-line heading, in `SECTIONS` order.
pub fn assert_sections(help: &str, context: &str) {
    let text = format!("\n{help}");
    let mut last = 0;
    for section in SECTIONS {
        let at = text
            .find(&format!("\n{section}\n"))
            .unwrap_or_else(|| panic!("{context}: missing {section}"));
        assert!(at >= last, "{context}: {section} out of order");
        last = at;
    }
}

#[test]
fn help_documents_each_section_an_agent_needs() {
    let output = cli().arg("--help").output().expect("help process");
    let help = String::from_utf8(output.stdout).expect("help utf8");
    assert_sections(&help, "overview");
    // Pagination continues only through these documented result fields.
    assert!(help.contains("result.next_cursor"));
    assert!(help.contains("result.next"));
    // An agent given only the help met an empty page after a non-null
    // cursor; help says which value ends paging.
    assert!(help.contains("until it is null"));
    assert!(help.contains("Only null ends the history"));
}

#[test]
fn help_after_a_command_prints_help_without_connecting() {
    for args in [
        vec!["search_messages", "--help"],
        vec![
            "list_rooms",
            "--account-id",
            "account:@member1:example.invalid",
            "-h",
        ],
        vec!["help"],
        vec!["help", "send_message"],
        vec!["no_such_tool", "--help"],
    ] {
        let output = cli().args(&args).output().expect("help process");
        assert!(output.status.success(), "{args:?}");
        let help = String::from_utf8(output.stdout).expect("help utf8");
        assert!(help.contains("SYNOPSIS"), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn the_word_cap_admits_a_full_room_list_and_is_named_in_help() {
    // 64 room ids (read_recent_activity's maxItems) as repeated options plus
    // every other option still fit; help names the cap.
    let help = cli().arg("--help").output().expect("help process");
    let text = String::from_utf8(help.stdout).expect("help utf8");
    assert!(text.contains("at most 160 words"));
    for (words, status) in [(160, Some(0)), (161, Some(2))] {
        let mut args = vec!["--help".to_owned()];
        args.extend((1..words).map(|index| format!("!room{index}:example.invalid")));
        let output = cli().args(&args).output().expect("cap process");
        assert_eq!(output.status.code(), status, "{words} words");
    }
}

#[test]
fn the_overview_examples_are_one_session_with_private_text_on_stdin() {
    let output = cli().arg("--help").output().expect("help process");
    let help = String::from_utf8(output.stdout).expect("help utf8");
    let examples = &help
        [help.find("\nEXAMPLES\n").expect("examples")..help.find("\nOUTPUT\n").expect("output")];
    // Each step feeds the next: ids come from earlier results, and private
    // text reaches --args-stdin from a file, never from argv.
    for step in [
        "account=$(koushi list_accounts",
        "room=$(koushi list_rooms --account-id \"$account\"",
        "koushi read_room_messages --account-id \"$account\" --room-id \"$room\"",
        "--rawfile body",
        "koushi send_message --args-stdin",
        "koushi react_to_message --args-stdin",
    ] {
        assert!(examples.contains(step), "missing step {step}");
    }
    // A room is chosen by its name, not by its position in the list.
    assert!(examples.contains("select(.name == $name)"));
    assert!(!examples.contains("rooms[0]"));
}
