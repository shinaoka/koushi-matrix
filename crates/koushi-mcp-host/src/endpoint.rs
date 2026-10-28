//! Endpoint file lifecycle for the MCP bridge.
//!
//! The file lives in the app data directory, is written `0600` inside a `0700`
//! directory, carries the per-launch token, and is removed on shutdown. The
//! token never appears in argv or the environment, so `ps` cannot leak it.

use std::path::{Path, PathBuf};

use koushi_mcp::wire::EndpointFile;

const SCHEMA: u32 = 1;

/// Path of `endpoint.json` under the given app data directory.
pub(crate) fn endpoint_path(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp").join("endpoint.json")
}

/// Path of the socket or pipe named in the endpoint file.
pub(crate) fn transport_path(data_dir: &Path) -> String {
    #[cfg(unix)]
    {
        data_dir
            .join("mcp")
            .join("mcp.sock")
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        format!("\\\\.\\pipe\\koushi-mcp-{}", pipe_suffix(data_dir))
    }
}

#[cfg(windows)]
fn pipe_suffix(data_dir: &Path) -> String {
    // Deterministic per user profile; the pipe ACL still restricts access.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data_dir.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Creates the `mcp` directory with restrictive permissions and returns it.
pub(crate) fn ensure_dir(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = data_dir.join("mcp");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    koushi_mcp::windows_security::PrivateSecurity::new()?.apply(&dir)?;
    Ok(dir)
}

/// Generates a fresh token for this app launch.
pub(crate) fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes)?;
    Ok(base64url(&bytes))
}

pub(crate) fn fill_random(bytes: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut file = std::fs::File::open("/dev/urandom")?;
        file.read_exact(bytes)
    }
    #[cfg(windows)]
    {
        // BCryptGenRandom is process-wide and needs no handle management.
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut core::ffi::c_void,
                buffer: *mut u8,
                len: u32,
                flags: u32,
            ) -> i32;
        }
        const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            return Err(std::io::Error::other("random generation failed"));
        }
        Ok(())
    }
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(combined >> 18) as usize & 63] as char);
        out.push(ALPHABET[(combined >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(combined >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[combined as usize & 63] as char);
        }
    }
    out
}

/// Writes the endpoint file with `0600` permissions.
pub(crate) fn write(
    data_dir: &Path,
    endpoint: &str,
    token: &str,
    app_version: &str,
) -> std::io::Result<PathBuf> {
    ensure_dir(data_dir)?;
    let path = endpoint_path(data_dir);
    let payload = EndpointFile {
        schema: SCHEMA,
        endpoint: endpoint.to_owned(),
        token: token.to_owned(),
        pid: std::process::id(),
        app_version: app_version.to_owned(),
    };
    let bytes = serde_json::to_vec(&payload)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_private(&path, &bytes)?;
    Ok(path)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // Create with 0600 from the start rather than widening then narrowing.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
    #[cfg(windows)]
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        koushi_mcp::windows_security::PrivateSecurity::new()?.apply(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
}

/// Removes the endpoint file and socket. Best effort: shutdown must not fail
/// because a file was already gone.
pub(crate) fn remove(data_dir: &Path) {
    let _ = std::fs::remove_file(endpoint_path(data_dir));
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(data_dir.join("mcp").join("mcp.sock"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_url_safe_and_long_enough() {
        let token = new_token().expect("token");
        assert!(token.len() >= 43, "token too short: {}", token.len());
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(token, new_token().expect("token"));
    }

    #[test]
    fn writes_and_removes_a_private_endpoint_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), &transport_path(dir.path()), "tok", "0.26.0").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let endpoint = koushi_mcp::endpoint::read(&path).expect("read");
        assert_eq!(endpoint.token, "tok");
        remove(dir.path());
        assert!(!path.exists());
    }
}
