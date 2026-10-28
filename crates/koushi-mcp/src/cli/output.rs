use std::io::{self, Write};
use std::process::ExitCode;

use super::{Failure, MAX_OUTPUT_BYTES};

/// JSON handles C0/ESC; additionally neutralize C1, bidi, and invisible format
/// controls in every key/value. JSON parsing restores the exact original text.
pub(super) fn render(value: &serde_json::Value, compact: bool) -> Result<Vec<u8>, Failure> {
    let mut bounded = Bounded(Vec::new());
    if compact {
        serde_json::to_writer(&mut bounded, value)
    } else {
        serde_json::to_writer_pretty(&mut bounded, value)
    }
    .map_err(|_| Failure {
        code: "output_too_large",
        status: 6,
    })?;
    let serialized = String::from_utf8(bounded.0).map_err(|_| Failure::INTERNAL)?;
    let mut bytes = Vec::new();
    for c in serialized.chars() {
        if c.is_control() && !matches!(c, '\n' | '\r' | '\t')
            || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
        {
            write!(&mut bytes, "\\u{:04x}", c as u32).map_err(|_| Failure::INTERNAL)?;
        } else {
            let mut utf8 = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
        }
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err(Failure {
                code: "output_too_large",
                status: 6,
            });
        }
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Keep result delivery inside the live deadline/signal select. Tokio uses
/// a blocking writer underneath; main bounds runtime shutdown so an unread
/// pipe cannot prevent process exit after this future is cancelled.
pub(super) async fn write_result(bytes: &[u8]) -> Result<(), Failure> {
    use tokio::io::AsyncWriteExt;
    let mut stdout = tokio::io::stdout();
    let result = async {
        stdout.write_all(bytes).await?;
        stdout.flush().await
    }
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(_) => Err(Failure {
            code: "output_error",
            status: 1,
        }),
    }
}

pub(super) fn write_stdout(bytes: &[u8]) -> ExitCode {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    match writer.write_all(bytes).and_then(|()| writer.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => fail(Failure {
            code: "output_error",
            status: 1,
        }),
    }
}

pub(super) fn fail(error: Failure) -> ExitCode {
    let _ = writeln!(io::stderr().lock(), "koushi: {}", error.code);
    ExitCode::from(error.status)
}

struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_OUTPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("output too large"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
