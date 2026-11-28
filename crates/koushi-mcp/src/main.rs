//! `koushi-mcp` is the stdio adapter that lets Claude Code and Codex use a
//! running Koushi instance.
//!
//! It speaks MCP on stdout/stdin and the Koushi local socket on the other side.
//! It holds no Matrix credentials, opens no crypto store, and never prints
//! anything but MCP messages on stdout.

use std::process::ExitCode;
use std::sync::Arc;

use koushi_mcp::channel::{ChannelFeed, ChannelRunner};
use koushi_mcp::server::{AppBackend, serve_with_channel};
use koushi_mcp::wire::ClientInfo;

const USAGE: &str = "\
koushi-mcp - MCP server for a running Koushi instance

Usage:
  koushi-mcp [--endpoint <path>] [--client-name <name>]
              [--profile <id> --profile-secret-file <path> [--claude-channel]]

Options:
  --endpoint <path>     Path to Koushi's endpoint.json (default: platform location)
  --client-name <name>  Name shown in Koushi's consent surface (default: koushi-mcp)
  --profile <id>        Agent profile id; makes the launcher speak protocol 2
  --profile-secret-file <path>
                        File holding the profile secret (never pass it in argv)
  --claude-channel      With a profile, declare the Claude Code `claude/channel`
                        capability and push wake events into the session. Start
                        Claude Code with --channels (or
                        --dangerously-load-development-channels) as well; a
                        session that did not opt in drops the events silently
  -h, --help            Print this help

Environment (used when the matching option is absent):
  KOUSHI_PROFILE, KOUSHI_PROFILE_SECRET_FILE, KOUSHI_CLAUDE_CHANNEL=1

The server needs a running Koushi. It stays alive when Koushi is down and
reports `koushi_unavailable` on tool calls until Koushi returns.
";

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("koushi-mcp: failed to start runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async_main())
}

async fn async_main() -> ExitCode {
    let mut endpoint_path = None;
    let mut client_name = "koushi-mcp".to_owned();
    let mut profile_id = None;
    let mut profile_secret_file = None;
    let mut claude_channel = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => match args.next() {
                Some(value) => endpoint_path = Some(std::path::PathBuf::from(value)),
                None => {
                    eprintln!("koushi-mcp: --endpoint requires a path");
                    return ExitCode::from(2);
                }
            },
            "--client-name" => match args.next() {
                Some(value) if koushi_mcp::wire::valid_client_name(&value) => client_name = value,
                Some(_) => {
                    eprintln!("koushi-mcp: {}", koushi_mcp::wire::CLIENT_NAME_RULE);
                    return ExitCode::from(2);
                }
                None => {
                    eprintln!("koushi-mcp: --client-name requires a value");
                    return ExitCode::from(2);
                }
            },
            "--profile" => match args.next() {
                Some(value) => profile_id = Some(value),
                None => {
                    eprintln!("koushi-mcp: --profile requires an id");
                    return ExitCode::from(2);
                }
            },
            "--profile-secret-file" => match args.next() {
                Some(value) => profile_secret_file = Some(std::path::PathBuf::from(value)),
                None => {
                    eprintln!("koushi-mcp: --profile-secret-file requires a path");
                    return ExitCode::from(2);
                }
            },
            "--claude-channel" => claude_channel = true,
            "--help" | "-h" => {
                eprintln!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("koushi-mcp: unknown argument: {other}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    let Some(endpoint_path) = endpoint_path.or_else(koushi_mcp::endpoint::default_path) else {
        eprintln!("koushi-mcp: cannot locate Koushi's endpoint file; pass --endpoint");
        return ExitCode::FAILURE;
    };

    // The environment is how a plugin picks the profile per launch, so the
    // choice is never stored in configuration that every session loads.
    let env_text = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let profile_id = profile_id.or_else(|| env_text("KOUSHI_PROFILE"));
    let profile_secret_file = profile_secret_file
        .or_else(|| env_text("KOUSHI_PROFILE_SECRET_FILE").map(std::path::PathBuf::from));
    claude_channel = claude_channel || env_text("KOUSHI_CLAUDE_CHANNEL").as_deref() == Some("1");

    let profile = match (profile_id, profile_secret_file) {
        (None, None) => None,
        (Some(id), Some(path)) => match koushi_mcp::profile_secret::read(&path) {
            Ok(secret) => Some(koushi_mcp::wire::ProfileHello { id, secret }),
            Err(error) => {
                eprintln!("koushi-mcp: {}", error.explanation());
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("koushi-mcp: --profile and --profile-secret-file go together");
            return ExitCode::from(2);
        }
    };

    let client_info = ClientInfo {
        name: client_name,
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let mut backend = AppBackend::new(endpoint_path.clone(), client_info.clone());
    // The channel needs a profile; without one `--claude-channel` has nothing
    // to subscribe with and the launcher stays an ordinary one.
    let channel_profile = profile.clone().filter(|_| claude_channel);
    if claude_channel && channel_profile.is_none() {
        eprintln!("koushi-mcp: --claude-channel needs --profile; running without the channel");
    }
    if let Some(profile) = profile {
        backend = backend.with_profile(profile);
    }
    let backend = Arc::new(backend);
    let channel = channel_profile.map(|profile| {
        Box::new(ChannelRunner {
            endpoint_path,
            client_info,
            profile,
            backend: backend.clone(),
        }) as Box<dyn ChannelFeed>
    });

    match serve_with_channel(tokio::io::stdin(), tokio::io::stdout(), backend, channel).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("koushi-mcp: stdio failure: {error}");
            ExitCode::FAILURE
        }
    }
}
