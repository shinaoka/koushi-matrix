//! Local socket client used by the launcher.
//!
//! One connection per client session. The launcher re-reads `endpoint.json` on
//! every reconnect, so a Koushi restart with a fresh token needs no client
//! restart: the next call reconnects and presents the new token.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::endpoint;
use crate::wire::{
    ClientInfo, ClientMessage, EndpointFile, ErrorCode, MAX_FRAME_BYTES, PROTOCOL_VERSION,
    ServerMessage, WireError, decode_line, encode_line,
};

#[cfg(unix)]
type Stream = tokio::net::UnixStream;
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

/// `tokio::io::split` halves: the platform stream types have no portable owned
/// split (named pipes have none at all).
type ReadHalf = tokio::io::ReadHalf<Stream>;
type WriteHalf = tokio::io::WriteHalf<Stream>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Covers the app's 50 s maximum long poll plus settlement overhead.
const CALL_TIMEOUT: Duration = Duration::from_secs(70);

/// A live IPC connection. Owned by the single stdio loop, so never shared.
pub struct Client {
    reader: BufReader<ReadHalf>,
    writer: WriteHalf,
    next_id: u64,
}

async fn open(endpoint_path: &EndpointFile) -> Result<Stream, WireError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let unavailable = || WireError::new_without_detail(ErrorCode::KoushiUnavailable);
        let metadata =
            std::fs::symlink_metadata(&endpoint_path.endpoint).map_err(|_| unavailable())?;
        // The token must never be sent to another OS user or a non-socket.
        // SAFETY: geteuid has no preconditions and returns this process's UID.
        let uid = unsafe { libc::geteuid() };
        if !metadata.file_type().is_socket()
            || metadata.uid() != uid
            || metadata.mode() & 0o022 != 0
        {
            return Err(unavailable());
        }
        let stream = tokio::net::UnixStream::connect(&endpoint_path.endpoint)
            .await
            .map_err(|_| unavailable())?;
        if stream.peer_cred().map_err(|_| unavailable())?.uid() != uid {
            return Err(unavailable());
        }
        Ok(stream)
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        let stream = loop {
            match ClientOptions::new().open(&endpoint_path.endpoint) {
                Ok(stream) => break stream,
                // A connected instance can be busy while the host creates
                // the next one. The outer CONNECT_TIMEOUT bounds this wait.
                Err(error)
                    if error.raw_os_error()
                        == Some(windows_sys::Win32::Foundation::ERROR_PIPE_BUSY as i32) =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(_) => return Err(WireError::new_without_detail(ErrorCode::KoushiUnavailable)),
            }
        };
        crate::windows_security::validate_pipe(&stream)
            .map_err(|_| WireError::new_without_detail(ErrorCode::Unauthorized))?;
        Ok(stream)
    }
}

/// Connects and completes the `hello` handshake.
pub async fn connect(path: &Path, client: ClientInfo) -> Result<Client, WireError> {
    let endpoint_file = endpoint::read(path)
        .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))?;

    let stream = tokio::time::timeout(CONNECT_TIMEOUT, open(&endpoint_file))
        .await
        .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))??;

    let (read_half, writer) = tokio::io::split(stream);
    let mut client_socket = Client {
        reader: BufReader::new(read_half),
        writer,
        next_id: 1,
    };

    let hello = ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        token: endpoint_file.token,
        client,
    };
    let line =
        encode_line(&hello).map_err(|_| WireError::new_without_detail(ErrorCode::Internal))?;
    client_socket
        .writer
        .write_all(line.as_bytes())
        .await
        .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))?;
    client_socket
        .writer
        .flush()
        .await
        .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))?;

    match client_socket.read_message().await? {
        ServerMessage::HelloOk { protocol, .. } if protocol == PROTOCOL_VERSION => {}
        ServerMessage::HelloOk { .. } => {
            return Err(WireError::new(
                ErrorCode::Unsupported,
                "the running Koushi speaks an unsupported bridge protocol",
            ));
        }
        ServerMessage::Error { code, message } => return Err(WireError::new(code, message)),
        other => {
            return Err(WireError::new(
                ErrorCode::Internal,
                format!("unexpected handshake reply: {other:?}"),
            ));
        }
    }

    Ok(client_socket)
}

impl Client {
    async fn read_message(&mut self) -> Result<ServerMessage, WireError> {
        let line = tokio::time::timeout(CALL_TIMEOUT, read_frame(&mut self.reader))
            .await
            .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))?
            .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))?;
        if line.is_empty() {
            return Err(WireError::new_without_detail(ErrorCode::KoushiUnavailable));
        }
        decode_line(&line).map_err(|_| WireError::new_without_detail(ErrorCode::Internal))
    }

    /// Sends one request and waits for its response.
    pub async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, WireError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let line = encode_line(&ClientMessage::Request {
            id,
            method: method.to_owned(),
            params,
        })
        .map_err(|_| WireError::new_without_detail(ErrorCode::Internal))?;
        if line.len() > MAX_FRAME_BYTES {
            return Err(WireError::new_without_detail(ErrorCode::Unsupported));
        }
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))?;
        self.writer
            .flush()
            .await
            .map_err(|_| WireError::new_without_detail(ErrorCode::KoushiUnavailable))?;

        match self.read_message().await? {
            ServerMessage::Response {
                id: got,
                ok,
                result,
                error,
            } if got == id => {
                if ok {
                    Ok(result.unwrap_or(serde_json::Value::Null))
                } else {
                    Err(error.unwrap_or_else(|| WireError::new_without_detail(ErrorCode::Internal)))
                }
            }
            // One request is in flight at a time, so a mismatched id is a
            // protocol violation rather than something to resynchronize.
            ServerMessage::Response { .. } => {
                Err(WireError::new_without_detail(ErrorCode::Internal))
            }
            ServerMessage::Error { code, message } => Err(WireError::new(code, message)),
            ServerMessage::HelloOk { .. } => {
                Err(WireError::new_without_detail(ErrorCode::Internal))
            }
        }
    }
}

/// Read one frame with the limit applied before allocation, even without LF.
/// Reused by both local bridge peers; a rejected stream must be closed.
pub async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid frame"))
}
