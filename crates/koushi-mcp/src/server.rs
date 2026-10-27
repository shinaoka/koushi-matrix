//! MCP stdio loop.
//!
//! Speaks JSON-RPC 2.0 over standard input and output, one message per line.
//! Nothing else may ever be written to stdout: diagnostics belong on stderr,
//! because a client that cannot parse stdout drops the server.

use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::tools;
use crate::wire::{ClientInfo, ErrorCode, WireError};

/// Latest MCP protocol revision this server implements.
const PROTOCOL_VERSION: &str = "2025-06-18";

pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// The app-facing half of the bridge. Implemented by the IPC client in
/// production and by an in-memory fake in tests.
pub trait Backend: Send + Sync {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<Value, WireError>>;
}

/// Backend that talks to the running Koushi over the local socket.
pub struct AppBackend {
    endpoint_path: std::path::PathBuf,
    client_info: ClientInfo,
    client: tokio::sync::Mutex<Option<crate::ipc::Client>>,
}

impl AppBackend {
    pub fn new(endpoint_path: impl Into<std::path::PathBuf>, client_info: ClientInfo) -> Self {
        Self {
            endpoint_path: endpoint_path.into(),
            client_info,
            client: tokio::sync::Mutex::new(None),
        }
    }
}

impl Backend for AppBackend {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<Value, WireError>> {
        Box::pin(async move {
            let mut guard = self.client.lock().await;
            if guard.is_none() {
                *guard =
                    Some(crate::ipc::connect(&self.endpoint_path, self.client_info.clone()).await?);
            }
            let client = guard.as_mut().expect("client was just connected");
            match client.call(method, params).await {
                Ok(value) => Ok(value),
                Err(error) => {
                    // Only transport failures invalidate the connection; a
                    // typed rejection from the app leaves it usable.
                    if error.code == ErrorCode::KoushiUnavailable {
                        *guard = None;
                    }
                    Err(error)
                }
            }
        })
    }
}

/// Runs the MCP loop until stdin closes.
pub async fn serve<R, W>(input: R, output: W, backend: Arc<dyn Backend>) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(input).lines();
    let mut output = output;
    let mut initialized = false;

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > crate::wire::MAX_FRAME_BYTES {
            write_message(
                &mut output,
                &error_response(Value::Null, -32600, "message too large"),
            )
            .await?;
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(message, &backend, &mut initialized).await,
            Err(_) => Some(error_response(Value::Null, -32700, "parse error")),
        };
        if let Some(response) = response {
            write_message(&mut output, &response).await?;
        }
    }
    Ok(())
}

async fn write_message<W: AsyncWrite + Unpin>(
    output: &mut W,
    value: &Value,
) -> std::io::Result<()> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    output.write_all(line.as_bytes()).await?;
    output.flush().await
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true
    })
}

async fn handle_message(
    message: Value,
    backend: &Arc<dyn Backend>,
    initialized: &mut bool,
) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    // Notifications have no id and never receive a response.
    let is_notification = id.is_none();
    if is_notification && method == "notifications/initialized" {
        *initialized = true;
        return None;
    }

    match method {
        "initialize" => Some(result_response(
            id.unwrap_or(Value::Null),
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    "name": "koushi",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),
        "ping" => Some(result_response(id.unwrap_or(Value::Null), json!({}))),
        "tools/list" => Some(result_response(
            id.unwrap_or(Value::Null),
            json!({ "tools": tool_list() }),
        )),
        "tools/call" => Some(handle_tool_call(id.unwrap_or(Value::Null), params, backend).await),
        other if is_notification => {
            let _ = other;
            None
        }
        other => Some(error_response(
            id.unwrap_or(Value::Null),
            -32601,
            &format!("method not found: {other}"),
        )),
    }
}

fn tool_list() -> Vec<Value> {
    tools::definitions()
        .into_iter()
        .map(|tool| {
            let mut listed = json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
                "annotations": {
                    "readOnlyHint": tool.read_only,
                    "destructiveHint": !tool.read_only,
                    "openWorldHint": false
                }
            });
            if let Some(output_schema) = tool.output_schema {
                listed["outputSchema"] = output_schema;
            }
            listed
        })
        .collect()
}

async fn handle_tool_call(id: Value, params: Value, backend: &Arc<dyn Backend>) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return tool_result(id, tool_error("tools/call requires a tool name"));
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Err(message) = tools::validate_arguments(name, &arguments) {
        return tool_result(id, tool_error(&message));
    }

    let outcome = backend
        .call(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await;
    let result = match outcome {
        Ok(value) => value,
        Err(error) => tool_error(&format!("{}: {}", error.code.as_str(), error.message)),
    };
    // A backend result must already be a tool result. If it is not, wrap it so
    // the client never sees a shape it does not understand.
    let result = if result.get("content").is_some() {
        result
    } else {
        json!({ "content": [{ "type": "text", "text": result.to_string() }] })
    };
    tool_result(id, result)
}

fn tool_result(id: Value, result: Value) -> Value {
    result_response(id, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeBackend {
        seen: Mutex<Vec<String>>,
        reply: Result<Value, WireError>,
    }

    impl Backend for FakeBackend {
        fn call<'a>(
            &'a self,
            method: &'a str,
            params: Value,
        ) -> BoxFuture<'a, Result<Value, WireError>> {
            Box::pin(async move {
                self.seen
                    .lock()
                    .expect("lock")
                    .push(format!("{method}:{params}"));
                self.reply.clone()
            })
        }
    }

    fn backend(reply: Result<Value, WireError>) -> Arc<FakeBackend> {
        Arc::new(FakeBackend {
            seen: Mutex::new(Vec::new()),
            reply,
        })
    }

    async fn run(input: &str, backend: Arc<FakeBackend>) -> Vec<Value> {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        client
            .write_all(input.as_bytes())
            .await
            .expect("write input");
        client.shutdown().await.expect("close input");
        let mut output = Vec::new();
        serve(server, &mut output, backend).await.expect("serve");
        output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).expect("json line"))
            .collect()
    }

    #[tokio::test]
    async fn initialize_then_list_tools() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n"
        );
        let output = run(input, backend(Ok(json!({})))).await;
        assert_eq!(output.len(), 2, "notifications must not be answered");
        assert_eq!(output[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(
            output[0]["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
        let tools = output[1]["result"]["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 14);
        let read = tools
            .iter()
            .find(|tool| tool["name"] == "read_room_messages")
            .expect("read_room_messages");
        assert_eq!(read["annotations"]["readOnlyHint"], true);
        let send = tools
            .iter()
            .find(|tool| tool["name"] == "send_message")
            .expect("send_message");
        assert_eq!(send["annotations"]["readOnlyHint"], false);
        let search = tools
            .iter()
            .find(|tool| tool["name"] == "search_messages")
            .expect("search_messages");
        assert_eq!(search["annotations"]["readOnlyHint"], true);
        let wait = tools
            .iter()
            .find(|tool| tool["name"] == "wait_for_events")
            .expect("wait_for_events");
        assert_eq!(wait["outputSchema"]["type"], "object");
        assert!(
            wait["outputSchema"]["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|key| key == "coverage"))
        );
        assert!(read.get("outputSchema").is_none());
    }

    #[tokio::test]
    async fn tool_call_forwards_and_returns_content() {
        let backend = backend(Ok(json!({
            "content": [{ "type": "text", "text": "two rooms" }],
            "isError": false
        })));
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"list_rooms","arguments":{"account_id":"acct-1"}}}"#,
            "\n"
        );
        let output = run(input, backend.clone()).await;
        assert_eq!(output[0]["id"], 9);
        assert_eq!(output[0]["result"]["content"][0]["text"], "two rooms");
        let seen = backend.seen.lock().expect("lock");
        assert!(seen[0].contains("\"name\":\"list_rooms\""));
        assert!(seen[0].contains("acct-1"));
    }

    #[tokio::test]
    async fn backend_failure_becomes_a_tool_error() {
        let backend = backend(Err(WireError::new_without_detail(
            ErrorCode::KoushiUnavailable,
        )));
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_accounts","arguments":{}}}"#,
            "\n"
        );
        let output = run(input, backend).await;
        assert_eq!(output[0]["result"]["isError"], true);
        assert!(
            output[0]["result"]["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("koushi_unavailable")
        );
    }

    #[tokio::test]
    async fn invalid_arguments_do_not_reach_the_backend() {
        let backend = backend(Ok(json!({ "content": [] })));
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_messages","arguments":{"account_id":"a"}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
            "\n"
        );
        let output = run(input, backend.clone()).await;
        assert!(output[0]["result"]["isError"].as_bool().expect("isError"));
        assert!(output[1]["result"]["isError"].as_bool().expect("isError"));
        assert!(backend.seen.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn unknown_method_is_a_jsonrpc_error() {
        let input = concat!(r#"{"jsonrpc":"2.0","id":6,"method":"bogus/method"}"#, "\n");
        let output = run(input, backend(Ok(json!({})))).await;
        assert_eq!(output[0]["error"]["code"], -32601);
    }
}
