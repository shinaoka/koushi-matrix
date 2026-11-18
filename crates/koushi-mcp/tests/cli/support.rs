//! The shipped CLI process and a fake app endpoint, shared by the CLI suite.
use std::process::{Command, Stdio};

pub fn cli() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_koushi"));
    command.stdin(Stdio::null());
    command
}

#[cfg(unix)]
pub use unix::app;

#[cfg(unix)]
mod unix {
    use koushi_mcp::wire::{
        ClientMessage, PROTOCOL_VERSION, ServerMessage, decode_line, encode_line,
    };
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    /// A fake app endpoint that accepts one `tools/call` and answers `reply`;
    /// a null `reply` waits for the client to disconnect instead. The thread
    /// returns the call's params.
    pub fn app(
        reply: Value,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::thread::JoinHandle<Value>,
        std::sync::mpsc::Receiver<()>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("mcp.sock");
        let endpoint = dir.path().join("endpoint.json");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private directory");
        let listener = UnixListener::bind(&socket).expect("bind");
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .expect("socket mode");
        std::fs::write(&endpoint, json!({"schema":1,"endpoint":socket,"token":"synthetic-token","pid":1,"app_version":"0"}).to_string()).expect("endpoint");
        std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).expect("mode");
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("hello");
            let ClientMessage::Hello {
                token,
                client,
                protocol,
            } = decode_line(&line).expect("hello json")
            else {
                panic!("hello shape")
            };
            assert_eq!(token, "synthetic-token");
            assert_eq!(client.name, "koushi-cli");
            assert_eq!(protocol, PROTOCOL_VERSION);
            stream
                .write_all(
                    encode_line(&ServerMessage::HelloOk {
                        protocol: PROTOCOL_VERSION,
                        connection_id: "fixture".into(),
                        app_version: "0".into(),
                    })
                    .expect("encode")
                    .as_bytes(),
                )
                .expect("hello response");
            line.clear();
            reader.read_line(&mut line).expect("request");
            let ClientMessage::Request { id, method, params } =
                decode_line(&line).expect("request json")
            else {
                panic!("request shape")
            };
            assert_eq!(method, "tools/call");
            ready_tx.send(()).expect("request ready");
            if reply.is_null() {
                let mut byte = [0u8; 1];
                assert_eq!(
                    std::io::Read::read(&mut stream, &mut byte).expect("disconnected"),
                    0
                );
                return params;
            }
            stream
                .write_all(
                    encode_line(&ServerMessage::success(id, reply))
                        .expect("encode")
                        .as_bytes(),
                )
                .expect("response");
            params
        });
        (dir, endpoint, thread, ready_rx)
    }
}
