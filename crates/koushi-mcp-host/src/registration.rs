//! Registration of Koushi with MCP clients.
//!
//! Koushi never edits another tool's files. The settings page shows these
//! snippets with the bundled launcher's path filled in; for clients with an
//! official registration CLI it can also run that CLI on request, with the
//! argument vector built here, and for Claude Desktop it can build an MCPB
//! extension. The text is not product prose: it is a command, a config
//! fragment, or a manifest, identical in every locale.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationClient {
    ClaudeCode,
    Codex,
    Pi,
    ClaudeDesktop,
    DeepseekHarness,
    Other,
}

/// What the settings page can do besides offering the text to copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationAction {
    /// Run the client's own registration CLI ([`command_argv`]).
    RunCommand,
    /// Build a Claude Desktop extension ([`claude_desktop_manifest`]).
    CreateExtension,
}

/// The platform the launcher path belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    /// The MCPB `compatibility.platforms` value.
    fn mcpb_name(self) -> &'static str {
        match self {
            Self::MacOs => "darwin",
            Self::Windows => "win32",
            Self::Linux => "linux",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnippetKind {
    /// Run in a terminal.
    Command,
    /// Add to a YAML configuration file.
    Yaml,
    /// Add to a JSON configuration file.
    Json,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Registration {
    pub client: RegistrationClient,
    /// The name the assistant presents, and the name the user approves.
    pub client_name: String,
    pub kind: SnippetKind,
    pub text: String,
    /// Where a configuration fragment goes, when it is a file.
    pub config_hint: Option<String>,
    pub action: Option<RegistrationAction>,
}

/// The client name each client presents to Koushi.
pub fn client_name(client: RegistrationClient) -> &'static str {
    match client {
        RegistrationClient::ClaudeCode => "claude-code",
        RegistrationClient::Codex => "codex",
        RegistrationClient::Pi => "pi",
        RegistrationClient::ClaudeDesktop => "claude-desktop",
        RegistrationClient::DeepseekHarness => "deepseek-harness",
        RegistrationClient::Other => "my-client",
    }
}

/// The program and arguments of a client's official registration command, or
/// `None` for a client without one. The settings page shows exactly this
/// vector as text, and the desktop runs exactly this vector, never a string
/// the webview supplied.
pub fn command_argv(
    client: RegistrationClient,
    launcher: &str,
) -> Option<(&'static str, Vec<String>)> {
    let (program, prefix): (&'static str, &[&str]) = match client {
        RegistrationClient::ClaudeCode => ("claude", &["mcp", "add", "--scope", "user", "koushi"]),
        RegistrationClient::Codex => ("codex", &["mcp", "add", "koushi"]),
        RegistrationClient::Pi => ("pi", &["mcp", "add", "koushi"]),
        RegistrationClient::ClaudeDesktop
        | RegistrationClient::DeepseekHarness
        | RegistrationClient::Other => return None,
    };
    let mut args: Vec<String> = prefix.iter().map(|arg| (*arg).to_owned()).collect();
    args.extend([
        "--".to_owned(),
        launcher.to_owned(),
        "--client-name".to_owned(),
        client_name(client).to_owned(),
    ]);
    Some((program, args))
}

/// The snippets for one launcher path, quoted for `platform`.
pub fn registrations(launcher: &str, platform: Platform) -> Vec<Registration> {
    let windows = platform == Platform::Windows;
    let command = |client| {
        let (program, args) = command_argv(client, launcher).expect("a command client");
        let words: Vec<String> = std::iter::once(program.to_owned())
            .chain(args.iter().map(|arg| shell_word(arg, windows)))
            .collect();
        Registration {
            client,
            client_name: client_name(client).to_owned(),
            kind: SnippetKind::Command,
            text: words.join(" "),
            config_hint: None,
            action: Some(RegistrationAction::RunCommand),
        }
    };
    let mut all = vec![
        command(RegistrationClient::ClaudeCode),
        command(RegistrationClient::Codex),
        command(RegistrationClient::Pi),
    ];
    // Claude Desktop runs on macOS and Windows only.
    if platform != Platform::Linux {
        all.push(Registration {
            client: RegistrationClient::ClaudeDesktop,
            client_name: client_name(RegistrationClient::ClaudeDesktop).to_owned(),
            kind: SnippetKind::Json,
            text: mcp_servers_json(launcher, client_name(RegistrationClient::ClaudeDesktop)),
            config_hint: Some("claude_desktop_config.json".to_owned()),
            action: Some(RegistrationAction::CreateExtension),
        });
    }
    all.extend([
        Registration {
            client: RegistrationClient::DeepseekHarness,
            client_name: "deepseek-harness".to_owned(),
            kind: SnippetKind::Yaml,
            text: format!(
                "- id: mcp-koushi\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    serverName: koushi\n    transport: stdio\n    command: {}\n    args: ['--client-name', 'deepseek-harness']\n",
                yaml_quote(launcher)
            ),
            config_hint: Some("~/.dsh/profiles/<profile>/cordis.patch.yml".to_owned()),
            action: None,
        },
        Registration {
            client: RegistrationClient::Other,
            client_name: client_name(RegistrationClient::Other).to_owned(),
            kind: SnippetKind::Json,
            text: mcp_servers_json(launcher, client_name(RegistrationClient::Other)),
            config_hint: None,
            action: None,
        },
    ]);
    all
}

/// One shell word: plain when it needs no quoting, else quoted for the shell.
fn shell_word(text: &str, windows: bool) -> String {
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./=:@+".contains(&byte));
    if plain {
        text.to_owned()
    } else if windows {
        format!("\"{text}\"")
    } else {
        posix_quote(text)
    }
}

/// The tools the bridge offers, as the MCPB manifest lists them.
const TOOLS: [(&str, &str); 14] = [
    ("list_accounts", "List the signed-in Koushi accounts"),
    (
        "list_rooms",
        "List rooms and direct messages with unread counts",
    ),
    ("read_room_messages", "Read a room's recent messages"),
    (
        "search_messages",
        "Search message history, including encrypted rooms",
    ),
    ("send_message", "Send a message or a reply"),
    (
        "get_attachment",
        "Download an attachment into Koushi's media cache",
    ),
    ("wait_for_events", "Wait for new activity"),
    (
        "read_recent_activity",
        "Read bounded recent account activity",
    ),
    ("get_message", "Read one message by its event id"),
    ("react_to_message", "React to a message"),
    ("unreact_to_message", "Remove your reaction from a message"),
    ("edit_message", "Edit a message you sent"),
    ("delete_message", "Delete a message you sent"),
    ("send_thread_reply", "Reply in a thread"),
];

/// The `manifest.json` of the Claude Desktop extension. The launcher is
/// bundled at `server/koushi-mcp[.exe]`, as the MCPB examples do; the
/// extension only runs on `platform`, the one the bundled binary was built for.
pub fn claude_desktop_manifest(version: &str, platform: Platform) -> Value {
    let launcher = if platform == Platform::Windows {
        "server/koushi-mcp.exe"
    } else {
        "server/koushi-mcp"
    };
    json!({
        "manifest_version": "0.3",
        "name": "koushi",
        "display_name": "Koushi",
        "version": version,
        "description": "Search, read, and reply to your Matrix chats through the running Koushi app. Koushi asks you before each account and permission.",
        "author": { "name": "Koushi", "url": "https://github.com/shinaoka/koushi-matrix" },
        "homepage": "https://shinaoka.github.io/koushi-matrix/",
        "documentation": "https://github.com/shinaoka/koushi-matrix/blob/main/docs/help/ai-assistants.md",
        "icon": "icon.png",
        "license": "MIT OR Apache-2.0",
        "server": {
            "type": "binary",
            "entry_point": launcher,
            "mcp_config": {
                "command": format!("${{__dirname}}/{launcher}"),
                "args": ["--client-name", client_name(RegistrationClient::ClaudeDesktop)],
                "env": {}
            }
        },
        "tools": TOOLS
            .iter()
            .map(|(name, description)| json!({ "name": name, "description": description }))
            .collect::<Vec<_>>(),
        "compatibility": { "platforms": [platform.mcpb_name()] }
    })
}

/// POSIX single quotes; an embedded `'` closes, escapes, and reopens.
fn posix_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A YAML single-quoted scalar; `'` is doubled.
fn yaml_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn mcp_servers_json(launcher: &str, client_name: &str) -> String {
    let value = json!({
        "mcpServers": {
            "koushi": {
                "command": launcher,
                "args": ["--client-name", client_name]
            }
        }
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

#[cfg(test)]
#[path = "registration_tests.rs"]
mod tests;
