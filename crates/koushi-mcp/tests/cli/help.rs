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

#[test]
fn help_documents_each_section_an_agent_needs() {
    let output = cli().arg("--help").output().expect("help process");
    let help = String::from_utf8(output.stdout).expect("help utf8");
    for section in [
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
    ] {
        // A heading is a whole line, including the first line.
        assert!(
            format!("\n{help}").contains(&format!("\n{section}\n")),
            "missing {section}"
        );
    }
    // Pagination continues only through these documented result fields.
    assert!(help.contains("result.next_cursor"));
    assert!(help.contains("result.next"));
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
