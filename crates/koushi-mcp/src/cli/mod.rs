//! Terminal adapter generated from the MCP tool table: one command per tool,
//! one `tools/call` per process. Private payloads are emitted only as
//! requested stdout results, never as errors/diagnostics or automatic
//! persistent data.
mod args;
mod help;
mod output;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::wire::{ClientInfo, ErrorCode};
use args::item_property;

/// Properties whose value can only be an identifier, an opaque cursor, a
/// caller-chosen id, or a list of ids. Everything else — message text, a
/// search query, a reaction key, an attachment's local path — has no option
/// and enters only through `--args-stdin`, because argv reaches shell history
/// and process listings. Allowlist, not denylist: a property added to the tool
/// table later is stdin-only until someone classifies it here.
pub const ARGV_SAFE_PROPERTIES: [&str; 10] = [
    "account_id",
    "client_request_id",
    "cursor",
    "event_id",
    "from",
    "reply_to_event_id",
    "room_id",
    "room_ids",
    "space_id",
    "thread_root_event_id",
];

/// Whether a schema property gets a command-line option. Numbers and booleans
/// are bounds and flags, so they always do; a string or list does only when it
/// is on the argv-safe allowlist and the parser can fill it.
pub fn argv_option(name: &str, property: &Value) -> bool {
    match property["type"].as_str() {
        // A bound or a flag carries no content.
        Some("integer" | "boolean") => true,
        Some("string") => ARGV_SAFE_PROPERTIES.contains(&name),
        Some("array") => {
            let fillable =
                property["items"]["type"] == "string" || item_property(property).is_some();
            fillable && ARGV_SAFE_PROPERTIES.contains(&name)
        }
        _ => false,
    }
}

/// `--args-stdin` accepts any argument object that fits one bridge frame with
/// its request envelope (method, tool name, id) around it. The reserve turns an
/// object that is just under the frame into a usage error before connecting,
/// instead of `unsupported` after.
pub const MAX_ARGUMENT_BYTES: usize = crate::wire::MAX_FRAME_BYTES - 1024;
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
        Ok(args::Action::Help(text)) => return output::write_stdout(text.as_bytes()),
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

async fn execute(invocation: args::Invocation) -> Result<(), Failure> {
    let arguments = match invocation.arguments {
        Some(arguments) => arguments,
        None => {
            let arguments = read_argument_object().await?;
            args::check(&invocation.tool, &arguments)?;
            arguments
        }
    };
    let tool = invocation.tool.name;
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
        .call("tools/call", json!({"name":tool,"arguments":arguments}))
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
    let mut envelope = json!({"schema_version":1,"tool":tool,"result":result});
    if let Some(account) = arguments.get("account_id") {
        envelope["account_id"] = account.clone();
    }
    let bytes = output::render(&envelope, invocation.json)?;
    output::write_result(&bytes).await
}

/// `--args-stdin`: the whole argument object as UTF-8 JSON from non-terminal
/// stdin, bounded by the bridge frame. Nothing read is ever echoed.
async fn read_argument_object() -> Result<Value, Failure> {
    if std::io::stdin().is_terminal() {
        return Err(Failure::USAGE);
    }
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take((MAX_ARGUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Failure {
            code: "input_error",
            status: 2,
        })?;
    if bytes.len() > MAX_ARGUMENT_BYTES {
        return Err(Failure::USAGE);
    }
    match serde_json::from_slice(&bytes) {
        Ok(object @ Value::Object(_)) => Ok(object),
        _ => Err(Failure::USAGE),
    }
}
