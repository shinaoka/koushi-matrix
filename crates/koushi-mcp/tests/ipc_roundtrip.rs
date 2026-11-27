//! End-to-end test of the launcher's IPC client against a fake app socket.
#![cfg(unix)]

use std::path::Path;

use koushi_mcp::ipc;
use koushi_mcp::wire::{
    ClientInfo, ClientMessage, LEGACY_PROTOCOL, ServerMessage, WireError, decode_line, encode_line,
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
            reason: None,
            supported: None,
        })
        .expect("encode");
        write_half
            .write_all(encoded.as_bytes())
            .await
            .expect("write");
        return;
    }
    let hello_ok = encode_line(&ServerMessage::HelloOk {
        protocol: LEGACY_PROTOCOL,
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

/// Fake host that reads one hello, returns it, and answers with `reply`.
async fn answer_hello(
    reply: ServerMessage,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    tokio::task::JoinHandle<ClientMessage>,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("mcp.sock");
    let endpoint_path = dir.path().join("endpoint.json");
    write_endpoint(&endpoint_path, &socket_path.to_string_lossy(), "tok");
    let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind");
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let (read_half, mut write_half) = stream.into_split();
        let mut line = String::new();
        BufReader::new(read_half)
            .read_line(&mut line)
            .await
            .expect("hello");
        write_half
            .write_all(encode_line(&reply).expect("encode").as_bytes())
            .await
            .expect("reply");
        decode_line(&line).expect("hello json")
    });
    (dir, endpoint_path, task)
}

fn info() -> ClientInfo {
    ClientInfo {
        name: "test".to_owned(),
        version: "1".to_owned(),
    }
}

fn hello_ok(protocol: u32) -> ServerMessage {
    ServerMessage::HelloOk {
        protocol,
        connection_id: "t".to_owned(),
        app_version: "0".to_owned(),
    }
}

#[tokio::test]
async fn an_unprofiled_launcher_sends_protocol_1_without_a_profile() {
    let (_dir, endpoint, task) = answer_hello(hello_ok(1)).await;
    ipc::connect(&endpoint, info()).await.expect("connect");
    let ClientMessage::Hello {
        protocol, profile, ..
    } = task.await.expect("task")
    else {
        panic!("hello");
    };
    assert_eq!(protocol, 1);
    assert!(profile.is_none());
}

#[tokio::test]
async fn a_profiled_launcher_sends_protocol_2_with_the_profile() {
    let (_dir, endpoint, task) = answer_hello(hello_ok(2)).await;
    let profile = koushi_mcp::wire::ProfileHello {
        id: "p1".to_owned(),
        secret: "s".to_owned(),
    };
    ipc::connect_with_profile(&endpoint, info(), Some(profile))
        .await
        .expect("connect");
    let ClientMessage::Hello {
        protocol, profile, ..
    } = task.await.expect("task")
    else {
        panic!("hello");
    };
    assert_eq!(protocol, 2);
    assert_eq!(profile.expect("profile").id, "p1");
}

#[tokio::test]
async fn version_mismatches_map_to_a_typed_protocol_version_error() {
    let profile = || {
        Some(koushi_mcp::wire::ProfileHello {
            id: "p1".to_owned(),
            secret: "s".to_owned(),
        })
    };
    // A host from before protocol 2: the legacy bare `unsupported`.
    // Another protocol in `hello_ok`.
    // A typed error from the host.
    for reply in [
        ServerMessage::Error {
            code: koushi_mcp::ErrorCode::Unsupported,
            message: "older".to_owned(),
            reason: None,
            supported: None,
        },
        hello_ok(1),
        ServerMessage::Error {
            code: koushi_mcp::ErrorCode::Unsupported,
            message: "x".to_owned(),
            reason: Some("protocol_version".to_owned()),
            supported: Some(vec![1]),
        },
    ] {
        let (_dir, endpoint, _task) = answer_hello(reply).await;
        let error = match ipc::connect_with_profile(&endpoint, info(), profile()).await {
            Ok(_) => panic!("must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, koushi_mcp::ErrorCode::Unsupported);
        assert_eq!(error.reason.as_deref(), Some("protocol_version"));
    }
}
