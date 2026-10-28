//! End-to-end test of the launcher's IPC client against a fake app socket.
#![cfg(unix)]

use std::path::Path;

use koushi_mcp::ipc;
use koushi_mcp::wire::{
    ClientInfo, ClientMessage, PROTOCOL_VERSION, ServerMessage, WireError, decode_line, encode_line,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn write_endpoint(path: &Path, socket: &str, token: &str) {
    std::fs::set_permissions(
        path.parent().expect("parent"),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("private directory");
    let payload = serde_json::json!({
        "schema": 1,
        "endpoint": socket,
        "token": token,
        "pid": std::process::id(),
        "app_version": "0.26.0"
    });
    std::fs::write(path, serde_json::to_vec(&payload).expect("json")).expect("write");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
}

async fn accept_once(
    listener: tokio::net::UnixListener,
    expected_token: &'static str,
    reply: Result<serde_json::Value, WireError>,
) {
    let (stream, _) = listener.accept().await.expect("accept");
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read hello");
    let ClientMessage::Hello { token, .. } = decode_line(&line).expect("hello") else {
        panic!("expected hello");
    };
    if token != expected_token {
        let encoded = encode_line(&ServerMessage::Error {
            code: koushi_mcp::ErrorCode::Unauthorized,
            message: "nope".to_owned(),
        })
        .expect("encode");
        write_half
            .write_all(encoded.as_bytes())
            .await
            .expect("write");
        return;
    }
    let hello_ok = encode_line(&ServerMessage::HelloOk {
        protocol: PROTOCOL_VERSION,
        connection_id: "test".to_owned(),
        app_version: "0.26.0".to_owned(),
    })
    .expect("encode");
    write_half
        .write_all(hello_ok.as_bytes())
        .await
        .expect("write hello_ok");

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .expect("read request");
    let ClientMessage::Request { id, .. } = decode_line(&request_line).expect("request") else {
        panic!("expected request");
    };
    let response = match reply {
        Ok(value) => ServerMessage::success(id, value),
        Err(error) => ServerMessage::failure(id, error),
    };
    write_half
        .write_all(encode_line(&response).expect("encode").as_bytes())
        .await
        .expect("write response");
}

#[tokio::test]
async fn connects_authenticates_and_returns_the_tool_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("mcp.sock");
    let endpoint_path = dir.path().join("endpoint.json");
    write_endpoint(&endpoint_path, &socket_path.to_string_lossy(), "tok");

    let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind");
    let server = tokio::spawn(accept_once(
        listener,
        "tok",
        Ok(serde_json::json!({ "content": [] })),
    ));

    let mut client = ipc::connect(
        &endpoint_path,
        ClientInfo {
            name: "test".to_owned(),
            version: "1".to_owned(),
        },
    )
    .await
    .expect("connect");
    let result = client
        .call("tools/call", serde_json::json!({ "name": "list_accounts" }))
        .await
        .expect("call");
    assert_eq!(result, serde_json::json!({ "content": [] }));
    server.await.expect("server task");
}

#[tokio::test]
async fn a_rejected_token_surfaces_unauthorized() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("mcp.sock");
    let endpoint_path = dir.path().join("endpoint.json");
    write_endpoint(&endpoint_path, &socket_path.to_string_lossy(), "wrong");

    let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind");
    let server = tokio::spawn(accept_once(listener, "tok", Ok(serde_json::json!({}))));

    let error = match ipc::connect(
        &endpoint_path,
        ClientInfo {
            name: "test".to_owned(),
            version: "1".to_owned(),
        },
    )
    .await
    {
        Ok(_) => panic!("must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.code, koushi_mcp::ErrorCode::Unauthorized);
    server.await.expect("server task");
}

#[tokio::test]
async fn a_missing_endpoint_file_reports_koushi_unavailable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint_path = dir.path().join("endpoint.json");
    let error = match ipc::connect(
        &endpoint_path,
        ClientInfo {
            name: "test".to_owned(),
            version: "1".to_owned(),
        },
    )
    .await
    {
        Ok(_) => panic!("must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, koushi_mcp::ErrorCode::KoushiUnavailable);
}
