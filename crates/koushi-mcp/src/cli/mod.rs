//! Read-only terminal adapter. Private payloads are emitted only as requested
//! stdout results, never as errors/diagnostics or automatic persistent data.
mod args;
mod output;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::wire::{ClientInfo, ErrorCode};

const MAX_QUERY_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * crate::wire::MAX_FRAME_BYTES;

/// Only closed codes and statuses can reach diagnostics. No remote detail or
/// user-provided value is retained here.
#[derive(Clone, Copy, Debug)]
struct Failure {
    code: &'static str,
    status: u8,
}

impl Failure {
    const USAGE: Self = Self {
        code: "usage_error",
        status: 2,
    };
    const INTERNAL: Self = Self {
        code: "internal",
        status: 1,
    };

    fn bridge(code: ErrorCode) -> Self {
        Self {
            code: code.as_str(),
            status: match code {
                ErrorCode::KoushiUnavailable => 3,
                ErrorCode::Unauthorized | ErrorCode::ConsentRequired | ErrorCode::NotPermitted => 5,
                _ => 6,
            },
        }
    }
}

/// Entry point shared only with the shipped CLI binary.
pub fn main() -> ExitCode {
    let invocation = match args::parse(std::env::args_os().skip(1)) {
        Ok(args::Action::Help) => return output::write_stdout(args::HELP.as_bytes()),
        Ok(args::Action::Version) => {
            return output::write_stdout(
                concat!("koushi ", env!("CARGO_PKG_VERSION"), "\n").as_bytes(),
            );
        }
        Ok(args::Action::Call(invocation)) => invocation,
        Err(error) => return output::fail(error),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return output::fail(Failure::INTERNAL),
    };
    let duration = invocation.timeout;
    let result = runtime.block_on(async {
        tokio::select! {
            biased;
            signal = tokio::signal::ctrl_c() => {
                match signal {
                    Ok(()) => Err(Failure { code: "cancelled", status: 130 }),
                    Err(_) => Err(Failure::INTERNAL),
                }
            }
            result = tokio::time::timeout(duration, execute(invocation)) => {
                result.unwrap_or(Err(Failure { code: "timeout", status: 4 }))
            }
        }
    });
    // Tokio stdin uses an uncancelable blocking read. Never wait for its EOF
    // during shutdown after timeout/Ctrl-C. Process exit releases that read;
    // all async work (including the socket) was dropped by block_on above.
    runtime.shutdown_timeout(Duration::from_millis(50));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => output::fail(error),
    }
}

async fn execute(mut invocation: args::Invocation) -> Result<(), Failure> {
    if invocation.tool == "search_messages" {
        if std::io::stdin().is_terminal() {
            return Err(Failure::USAGE);
        }
        let mut bytes = Vec::new();
        tokio::io::stdin()
            .take((MAX_QUERY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Failure {
                code: "input_error",
                status: 2,
            })?;
        if bytes.len() > MAX_QUERY_BYTES {
            return Err(Failure::USAGE);
        }
        let query = String::from_utf8(bytes).map_err(|_| Failure::USAGE)?;
        let query = query.trim_end_matches(['\r', '\n']);
        if query.trim().is_empty() {
            return Err(Failure::USAGE);
        }
        invocation.arguments["query"] = json!(query);
    }
    crate::tools::validate_arguments(invocation.tool, &invocation.arguments)
        .map_err(|_| Failure::USAGE)?;
    let endpoint = invocation
        .endpoint
        .or_else(crate::endpoint::default_path)
        .ok_or_else(|| Failure::bridge(ErrorCode::KoushiUnavailable))?;
    let mut client = crate::ipc::connect(
        &endpoint,
        ClientInfo {
            name: "koushi-cli".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .await
    .map_err(|error| Failure::bridge(error.code))?;
    let response = client
        .call(
            "tools/call",
            json!({"name":invocation.tool,"arguments":invocation.arguments}),
        )
        .await
        .map_err(|error| Failure::bridge(error.code))?;
    let result = response.get("structuredContent").ok_or(Failure::INTERNAL)?;
    match response.get("isError").and_then(Value::as_bool) {
        Some(false) => {}
        Some(true) => {
            let code = result
                .pointer("/error/code")
                .cloned()
                .ok_or(Failure::INTERNAL)?;
            let code: ErrorCode = serde_json::from_value(code).map_err(|_| Failure::INTERNAL)?;
            return Err(Failure::bridge(code));
        }
        None => return Err(Failure::INTERNAL),
    }
    let mut envelope = json!({"schema_version":1,"tool":invocation.tool,"result":result});
    if let Some(account) = invocation.arguments.get("account_id") {
        envelope["account_id"] = account.clone();
    }
    let bytes = output::render(&envelope, invocation.json)?;
    output::write_result(&bytes).await
}
