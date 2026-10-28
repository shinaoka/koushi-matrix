//! Rejection happens before an unbounded read or disclosure to another transport.
use koushi_mcp::{endpoint, ipc, wire::MAX_FRAME_BYTES};
use serde_json::json;
use tokio::io::{AsyncWriteExt, BufReader};

#[tokio::test]
async fn unterminated_frames_fail_at_the_cap_without_waiting_for_eof() {
    let (mut writer, reader) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        let _ = writer.write_all(&vec![b'x'; MAX_FRAME_BYTES + 1]).await;
        // Hold the writer open. A read_to_end/read_line without an allocation
        // cap would wait here instead of rejecting the input.
        std::future::pending::<()>().await;
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        ipc::read_frame(&mut BufReader::new(reader)),
    )
    .await;
    task.abort();
    let _ = task.await;
    assert!(result.expect("bounded read terminated").is_err());
}

fn write_endpoint(dir: &std::path::Path, value: serde_json::Value) -> std::path::PathBuf {
    let path = dir.join("endpoint.json");
    std::fs::write(&path, value.to_string()).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).expect("dir mode");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("file mode");
    }
    #[cfg(windows)]
    {
        let security = koushi_mcp::windows_security::PrivateSecurity::new().expect("descriptor");
        security.apply(dir).expect("dir ACL");
        security.apply(&path).expect("file ACL");
    }
    path
}

#[test]
fn endpoint_schema_nonlocal_transports_and_size_are_checked() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (schema, transport) in [
        (99, "/tmp/mcp.sock"),
        (1, "https://example.invalid"),
        (1, "\\\\remote\\pipe\\mcp"),
    ] {
        let path = write_endpoint(
            dir.path(),
            json!({"schema":schema,"endpoint":transport,"token":"synthetic","pid":1,"app_version":"0"}),
        );
        assert!(endpoint::read(&path).is_err());
    }
    let path = write_endpoint(
        dir.path(),
        json!({"schema":1,"endpoint":"/tmp/mcp.sock","token":"x".repeat(17000),"pid":1,"app_version":"0"}),
    );
    assert!(endpoint::read(&path).is_err());
}

#[cfg(unix)]
#[test]
fn endpoint_symlinks_fifos_and_broad_parent_permissions_are_refused() {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_endpoint(
        dir.path(),
        json!({"schema":1,"endpoint":"/tmp/mcp.sock","token":"synthetic","pid":1,"app_version":"0"}),
    );
    let link = dir.path().join("link.json");
    symlink(&path, &link).expect("symlink");
    assert!(endpoint::read(&link).is_err());
    let fifo = dir.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).expect("fifo path");
    // SAFETY: name is NUL terminated; mkfifo does not retain it.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(endpoint::read(&fifo).is_err());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755))
        .expect("broaden dir");
    assert!(endpoint::read(&path).is_err());
}

#[cfg(windows)]
#[test]
fn windows_endpoint_dacl_is_restrictive_and_broad_acl_is_rejected() {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Security::Authorization::{SE_FILE_OBJECT, SetSecurityInfo};
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_endpoint(
        dir.path(),
        json!({"schema":1,"endpoint":"\\\\.\\pipe\\koushi-fixture","token":"synthetic","pid":1,"app_version":"0"}),
    );
    assert!(endpoint::read(&path).is_ok());
    let file = std::fs::OpenOptions::new()
        // READ_CONTROL is included by GENERIC_READ; deliberately modifying
        // the fixture ACL additionally requires WRITE_DAC on this handle.
        .access_mode(windows_sys::Win32::Foundation::GENERIC_READ | 0x0004_0000)
        .open(&path)
        .expect("open");
    // SAFETY: handle is live. A NULL DACL deliberately grants everyone access
    // for this synthetic regression fixture; the validator must reject it.
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(result, 0);
    assert!(endpoint::read(&path).is_err());
}

#[cfg(windows)]
#[tokio::test]
async fn an_untrusted_connected_pipe_receives_no_handshake_bytes() {
    use tokio::io::AsyncReadExt;
    use tokio::net::windows::named_pipe::ServerOptions;
    let dir = tempfile::tempdir().expect("tempdir");
    let name = format!("\\\\.\\pipe\\koushi-untrusted-{}", std::process::id());
    // Windows' default pipe DACL includes broad read access. Even with a
    // valid private endpoint file this must not receive the connection token.
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .expect("pipe");
    let path = write_endpoint(
        dir.path(),
        json!({"schema":1,"endpoint":name,"token":"synthetic","pid":1,"app_version":"0"}),
    );
    let server_check = async move {
        server.connect().await.expect("connected");
        let mut byte = [0u8; 1];
        let read = tokio::time::timeout(std::time::Duration::from_secs(5), server.read(&mut byte))
            .await
            .expect("closed in time");
        assert!(
            matches!(read, Ok(0)) || read.is_err(),
            "untrusted server received hello bytes"
        );
    };
    let client_check = async {
        let connected = ipc::connect(
            &path,
            koushi_mcp::wire::ClientInfo {
                name: "koushi-cli".into(),
                version: "1".into(),
            },
        )
        .await;
        assert!(connected.is_err());
    };
    // Poll ConnectNamedPipe before opening/closing the rejected client. This
    // makes zero-byte observation independent of task scheduling.
    tokio::join!(biased; server_check, client_check);
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_unread_stdout_does_not_block_the_command_deadline() {
    use koushi_mcp::wire::{
        ClientMessage, PROTOCOL_VERSION, ServerMessage, decode_line, encode_line,
    };
    use std::process::{Command, Stdio};
    use tokio::net::windows::named_pipe::ServerOptions;
    let dir = tempfile::tempdir().expect("tempdir");
    let name = format!("\\\\.\\pipe\\koushi-output-fixture-{}", std::process::id());
    let security = koushi_mcp::windows_security::PrivateSecurity::new().expect("ACL");
    let mut attrs = security.attributes();
    // SAFETY: attributes and descriptor remain alive during pipe creation.
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .create_with_security_attributes_raw(&name, std::ptr::from_mut(&mut attrs).cast())
    }
    .expect("private pipe");
    let path = write_endpoint(
        dir.path(),
        json!({"schema":1,"endpoint":name,"token":"synthetic","pid":1,"app_version":"0"}),
    );
    let task = tokio::spawn(async move {
        server.connect().await.expect("connected");
        let (read, mut write) = tokio::io::split(server);
        let mut read = BufReader::new(read);
        let hello = ipc::read_frame(&mut read).await.expect("hello");
        assert!(matches!(
            decode_line::<ClientMessage>(&hello),
            Ok(ClientMessage::Hello { .. })
        ));
        write
            .write_all(
                encode_line(&ServerMessage::HelloOk {
                    protocol: PROTOCOL_VERSION,
                    connection_id: "fixture".into(),
                    app_version: "0".into(),
                })
                .expect("encode")
                .as_bytes(),
            )
            .await
            .expect("hello response");
        let request = ipc::read_frame(&mut read).await.expect("request");
        let ClientMessage::Request { id, .. } = decode_line(&request).expect("request json") else {
            panic!("request shape")
        };
        write
            .write_all(
                encode_line(&ServerMessage::success(
                    id,
                    json!({"isError":false,"structuredContent":{"body":"x".repeat(500000)}}),
                ))
                .expect("encode")
                .as_bytes(),
            )
            .await
            .expect("response");
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_koushi"))
        .args(["accounts", "--json", "--timeout", "1", "--endpoint"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let unread = child.stdout.take().expect("unread pipe");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().expect("poll").is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().expect("kill");
            child.wait().expect("reap");
            panic!("unread output blocked deadline");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    drop(unread);
    let output = child.wait_with_output().expect("output");
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(output.stderr, b"koushi: timeout\n");
    task.await.expect("server");
}
