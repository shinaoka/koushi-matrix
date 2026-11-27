//! `koushi-mcp` is the stdio adapter that lets Claude Code and Codex use a
//! running Koushi instance.
//!
//! It speaks MCP on stdout/stdin and the Koushi local socket on the other side.
//! It holds no Matrix credentials, opens no crypto store, and never prints
//! anything but MCP messages on stdout.

use std::process::ExitCode;
use std::sync::Arc;

use koushi_mcp::server::{AppBackend, serve};
use koushi_mcp::wire::ClientInfo;

const USAGE: &str = "\
koushi-mcp - MCP server for a running Koushi instance

Usage:
  koushi-mcp [--endpoint <path>] [--client-name <name>]
              [--profile <id> --profile-secret-file <path>]

Options:
  --endpoint <path>     Path to Koushi's endpoint.json (default: platform location)
  --client-name <name>  Name shown in Koushi's consent surface (default: koushi-mcp)
  --profile <id>        Agent profile id; makes the launcher speak protocol 2
  --profile-secret-file <path>
                        File holding the profile secret (never pass it in argv)
  -h, --help            Print this help

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

    let mut backend = AppBackend::new(
        endpoint_path,
        ClientInfo {
            name: client_name,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    );
    if let Some(profile) = profile {
        backend = backend.with_profile(profile);
    }
    let backend = Arc::new(backend);

    match serve(tokio::io::stdin(), tokio::io::stdout(), backend).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("koushi-mcp: stdio failure: {error}");
            ExitCode::FAILURE
        }
    }
}
