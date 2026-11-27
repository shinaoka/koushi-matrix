use super::*;

fn find(registrations: &[Registration], client: RegistrationClient) -> &Registration {
    registrations
        .iter()
        .find(|registration| registration.client == client)
        .expect("client")
}

fn text(registrations: &[Registration], client: RegistrationClient) -> &str {
    &find(registrations, client).text
}

#[test]
fn commands_quote_a_posix_path_with_spaces_and_quotes() {
    let all = registrations("/Apps/Kou shi's/koushi-mcp", Platform::MacOs);
    assert_eq!(
        text(&all, RegistrationClient::ClaudeCode),
        "claude mcp add --scope user koushi -- '/Apps/Kou shi'\\''s/koushi-mcp' --client-name claude-code"
    );
    assert_eq!(
        text(&all, RegistrationClient::Pi),
        "pi mcp add koushi -- '/Apps/Kou shi'\\''s/koushi-mcp' --client-name pi"
    );
}

#[test]
fn windows_paths_use_double_quotes() {
    let all = registrations(
        r"C:\Users\Member 1\AppData\Local\Koushi\koushi-mcp.exe",
        Platform::Windows,
    );
    assert_eq!(
        text(&all, RegistrationClient::Codex),
        r#"codex mcp add koushi -- "C:\Users\Member 1\AppData\Local\Koushi\koushi-mcp.exe" --client-name codex"#
    );
}

#[test]
fn json_and_yaml_escape_the_path() {
    let all = registrations(r"C:\Kou'shi\koushi-mcp.exe", Platform::Windows);
    let json: serde_json::Value =
        serde_json::from_str(text(&all, RegistrationClient::Other)).expect("valid json");
    assert_eq!(
        json["mcpServers"]["koushi"]["command"],
        r"C:\Kou'shi\koushi-mcp.exe"
    );
    assert!(
        text(&all, RegistrationClient::DeepseekHarness)
            .contains(r"command: 'C:\Kou''shi\koushi-mcp.exe'")
    );
}

#[test]
fn the_shown_command_is_the_vector_that_runs() {
    let launcher = "/Applications/Koushi.app/Contents/MacOS/koushi-mcp";
    let all = registrations(launcher, Platform::MacOs);
    for client in [
        RegistrationClient::ClaudeCode,
        RegistrationClient::Codex,
        RegistrationClient::Pi,
    ] {
        let (program, args) = command_argv(client, launcher).expect("command client");
        let shown: Vec<&str> = text(&all, client).split(' ').collect();
        let expected: Vec<&str> = std::iter::once(program)
            .chain(args.iter().map(String::as_str))
            .collect();
        assert_eq!(shown, expected, "{client:?}");
        assert_eq!(
            find(&all, client).action,
            Some(RegistrationAction::RunCommand)
        );
        assert_eq!(args.last().map(String::as_str), Some(client_name(client)));
    }
    for client in [
        RegistrationClient::ClaudeDesktop,
        RegistrationClient::DeepseekHarness,
        RegistrationClient::Other,
    ] {
        assert!(command_argv(client, launcher).is_none());
    }
}

#[test]
fn a_launcher_path_starting_with_a_dash_is_still_one_argument_after_the_separator() {
    let (_, args) = command_argv(RegistrationClient::Codex, "-x/koushi-mcp").expect("codex");
    let separator = args.iter().position(|arg| arg == "--").expect("separator");
    assert_eq!(args[separator + 1], "-x/koushi-mcp");
}

#[test]
fn claude_desktop_is_offered_only_where_it_runs() {
    let mac = registrations("/opt/koushi-mcp", Platform::MacOs);
    let desktop = find(&mac, RegistrationClient::ClaudeDesktop);
    assert_eq!(desktop.action, Some(RegistrationAction::CreateExtension));
    assert_eq!(desktop.client_name, "claude-desktop");
    let windows = registrations(r"C:\Koushi\koushi-mcp.exe", Platform::Windows);
    assert!(
        windows
            .iter()
            .any(|r| r.client == RegistrationClient::ClaudeDesktop)
    );
    let linux = registrations("/usr/bin/koushi-mcp", Platform::Linux);
    assert!(
        !linux
            .iter()
            .any(|r| r.client == RegistrationClient::ClaudeDesktop)
    );
}

#[test]
fn every_client_has_a_distinct_name() {
    let all = registrations("/usr/bin/koushi-mcp", Platform::MacOs);
    let mut names: Vec<&str> = all.iter().map(|r| r.client_name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all.len());
}

#[test]
fn the_claude_desktop_manifest_runs_the_bundled_launcher() {
    let mac = claude_desktop_manifest("0.28.0", Platform::MacOs);
    assert_eq!(mac["manifest_version"], "0.3");
    assert_eq!(mac["version"], "0.28.0");
    assert_eq!(mac["server"]["type"], "binary");
    assert_eq!(mac["server"]["entry_point"], "server/koushi-mcp");
    assert_eq!(
        mac["server"]["mcp_config"]["command"],
        "${__dirname}/server/koushi-mcp"
    );
    assert_eq!(
        mac["server"]["mcp_config"]["args"],
        serde_json::json!(["--client-name", "claude-desktop"])
    );
    assert_eq!(
        mac["compatibility"]["platforms"],
        serde_json::json!(["darwin"])
    );
    assert_eq!(mac["tools"].as_array().map(Vec::len), Some(TOOLS.len()));

    let windows = claude_desktop_manifest("0.28.0", Platform::Windows);
    assert_eq!(windows["server"]["entry_point"], "server/koushi-mcp.exe");
    assert_eq!(
        windows["compatibility"]["platforms"],
        serde_json::json!(["win32"])
    );
}

#[test]
fn the_manifest_lists_exactly_the_tools_the_launcher_serves() {
    let mut served: Vec<String> = koushi_mcp::tools::definitions()
        .into_iter()
        .map(|tool| tool.name.to_owned())
        .collect();
    served.sort();
    let mut listed: Vec<String> = TOOLS.iter().map(|(name, _)| (*name).to_owned()).collect();
    listed.sort();
    assert_eq!(listed, served);
}

#[test]
fn profile_snippets_carry_the_profile_and_the_secret_path_and_no_secret() {
    let id = "prof_00112233445566778899aabbccddeeff";
    let path = "/Users/me/Library/Application Support/koushi/mcp/profiles/prof_00112233445566778899aabbccddeeff.secret";
    let snippets = profile_registrations(
        "/Applications/Koushi.app/Contents/MacOS/koushi-mcp",
        Platform::MacOs,
        id,
        path,
    );
    let claude = snippets
        .iter()
        .find(|snippet| snippet.client == RegistrationClient::ClaudeCode)
        .expect("Claude Code");
    assert_eq!(
        claude.text,
        format!(
            "claude mcp add --scope user koushi-prof_00112233 -- /Applications/Koushi.app/Contents/MacOS/koushi-mcp --client-name claude-code --profile {id} --profile-secret-file '{path}'"
        )
    );
    assert_eq!(claude.kind, SnippetKind::Command);
    let other = snippets
        .iter()
        .find(|snippet| snippet.client == RegistrationClient::Other)
        .expect("generic");
    let parsed: serde_json::Value = serde_json::from_str(&other.text).expect("json");
    let args = &parsed["mcpServers"]["koushi-prof_00112233"]["args"];
    assert_eq!(args[2], "--profile");
    assert_eq!(args[3], id);
    assert_eq!(args[5], path);
    // The same text on every platform except the quoting of the launcher path.
    for platform in [Platform::MacOs, Platform::Linux] {
        let again = profile_registrations("/opt/koushi-mcp", platform, id, "/data/p.secret");
        assert_eq!(again.len(), 4);
    }
}
