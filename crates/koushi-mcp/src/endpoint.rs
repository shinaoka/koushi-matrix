//! Endpoint discovery and the `endpoint.json` contract.
//!
//! The app writes the file on startup (`0600`, directory `0700`) and removes it
//! on shutdown. The launcher only reads it. The token inside is per launch; it
//! never appears in argv, environment, or logs, which is why the file exists at
//! all instead of a command-line argument.

use std::path::{Path, PathBuf};

use crate::wire::EndpointFile;

/// Application data directory name the desktop app uses
/// (`DATA_DIR_NAME` in the Tauri adapter). The launcher must look in the same
/// place or the endpoint file is never found.
pub const DATA_DIR_NAME: &str = "koushi-desktop";

/// Default location of `endpoint.json` on this platform.
pub fn default_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(DATA_DIR_NAME)
                .join("mcp/endpoint.json")
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA").map(|dir| {
            PathBuf::from(dir)
                .join(DATA_DIR_NAME)
                .join("mcp")
                .join("endpoint.json")
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // `dirs::data_local_dir()` on Linux: `$XDG_DATA_HOME`, else
        // `$HOME/.local/share`.
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })?;
        Some(base.join(DATA_DIR_NAME).join("mcp/endpoint.json"))
    }
}

/// Reads and parses the endpoint file.
///
/// On Unix the file must not be group- or world-accessible; a loosened mode is
/// treated as an error rather than silently trusted.
pub fn read(path: &Path) -> std::io::Result<EndpointFile> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Never follow an endpoint symlink or block opening a FIFO.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    let invalid =
        || std::io::Error::new(std::io::ErrorKind::PermissionDenied, "untrusted endpoint");
    if !metadata.is_file() {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions and returns this process's UID.
        let uid = unsafe { libc::geteuid() };
        let parent = std::fs::metadata(path.parent().ok_or_else(invalid)?)?;
        if metadata.uid() != uid
            || metadata.mode() & 0o077 != 0
            || !parent.is_dir()
            || parent.uid() != uid
            || parent.mode() & 0o077 != 0
        {
            return Err(invalid());
        }
    }
    #[cfg(windows)]
    crate::windows_security::validate_file(&file)?;
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 {
        return Err(invalid());
    }
    let endpoint: EndpointFile = serde_json::from_slice(&bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid endpoint"))?;
    #[cfg(unix)]
    let local = Path::new(&endpoint.endpoint).is_absolute();
    #[cfg(windows)]
    let local = endpoint.endpoint.starts_with(r"\\.\pipe\")
        && !endpoint.endpoint[9..].is_empty()
        && !endpoint.endpoint[9..].contains(['\\', '/']);
    if endpoint.schema != 1 || !local || endpoint.token.is_empty() || endpoint.token.len() > 1024 {
        return Err(invalid());
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_endpoint(path: &Path, content: &str) {
        #[cfg(unix)]
        std::fs::set_permissions(
            path.parent().expect("parent"),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("private directory");
        std::fs::write(path, content).expect("write");
        #[cfg(windows)]
        crate::windows_security::PrivateSecurity::new()
            .expect("ACL")
            .apply(path)
            .expect("private endpoint");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        }
    }

    #[test]
    fn the_default_path_matches_the_desktop_data_dir() {
        let Some(path) = default_path() else {
            return;
        };
        let tail: Vec<String> = path
            .iter()
            .rev()
            .take(3)
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            tail,
            vec![
                "endpoint.json".to_owned(),
                "mcp".to_owned(),
                DATA_DIR_NAME.to_owned()
            ],
            "launcher and app must agree on the data directory"
        );
    }

    #[test]
    fn reads_a_private_endpoint_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("endpoint.json");
        let transport = if cfg!(windows) {
            r"\\.\pipe\koushi-fixture"
        } else {
            "/tmp/x.sock"
        };
        write_endpoint(
            &path,
            &serde_json::json!({
                "schema":1,"endpoint":transport,"token":"abc","pid":7,"app_version":"0.26.0"
            })
            .to_string(),
        );
        let endpoint = read(&path).expect("read");
        assert_eq!(endpoint.schema, 1);
        assert_eq!(endpoint.token, "abc");
        assert_eq!(endpoint.pid, 7);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_world_readable_endpoint_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("endpoint.json");
        std::fs::write(
            &path,
            r#"{"schema":1,"endpoint":"/tmp/x.sock","token":"abc","pid":7,"app_version":"0.26.0"}"#,
        )
        .expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let error = read(&path).expect_err("must reject");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
