//! Builds the Claude Desktop extension (MCPB) for the bundled launcher.
//!
//! The bundle holds `manifest.json`, the app icon, and a byte-for-byte copy of
//! the launcher at `server/koushi-mcp[.exe]`, as the MCPB examples do; the copy
//! keeps the launcher's code signature. Koushi writes it to the Downloads
//! folder and opens it, and Claude Desktop shows its own install review.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use koushi_mcp_host::registration::{Platform, claude_desktop_manifest};
use serde::Serialize;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, System};

pub(crate) const FILE_NAME: &str = "Koushi.mcpb";
const ICON: &[u8] = include_bytes!("../../icons/icon.png");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ExtensionOutcome {
    /// Written and handed to the system, which opens Claude Desktop's install
    /// review.
    Opened {
        path: String,
    },
    /// Written, but nothing opened it; the user installs it from Claude
    /// Desktop's settings or by dragging it into the window.
    Saved {
        path: String,
    },
    /// Claude Desktop does not run on this platform.
    Unsupported,
    LauncherUnavailable,
    WriteFailed,
}

/// The `.mcpb` archive for `launcher`.
pub(crate) fn build_bundle(
    launcher: &Path,
    version: &str,
    platform: Platform,
) -> std::io::Result<Vec<u8>> {
    let manifest = claude_desktop_manifest(version, platform);
    let entry_point = manifest["server"]["entry_point"]
        .as_str()
        .unwrap_or("server/koushi-mcp")
        .to_owned();
    let launcher_bytes = std::fs::read(launcher)?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(std::io::Error::other)?;

    let mut buffer = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut buffer);
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        // zip 9 stamps DOS attributes on a Windows host unless a system is
        // named, which drops the launcher's 0o755 from the archive.
        .system(System::Unix);
    let entries: [(&str, &[u8], u32); 3] = [
        ("manifest.json", &manifest_bytes, 0o644),
        ("icon.png", ICON, 0o644),
        (&entry_point, &launcher_bytes, 0o755),
    ];
    for (name, bytes, mode) in entries {
        zip.start_file(name, stored.unix_permissions(mode))
            .map_err(std::io::Error::other)?;
        zip.write_all(bytes)?;
    }
    zip.finish().map_err(std::io::Error::other)?;
    Ok(buffer.into_inner())
}

/// Writes `bytes` as `directory/Koushi.mcpb` through a temporary file and a
/// rename, so an existing file, or a planted symlink, is replaced rather than
/// written through.
pub(crate) fn write_bundle(directory: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    let target = directory.join(FILE_NAME);
    temporary.persist(&target).map_err(|error| error.error)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn launcher(dir: &Path) -> PathBuf {
        let path = dir.join("koushi-mcp");
        std::fs::write(&path, b"synthetic launcher").expect("write");
        path
    }

    #[test]
    fn the_bundle_holds_the_manifest_icon_and_an_executable_launcher() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = build_bundle(&launcher(dir.path()), "0.28.0", Platform::MacOs).expect("bundle");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");

        let mut manifest = String::new();
        archive
            .by_name("manifest.json")
            .expect("manifest")
            .read_to_string(&mut manifest)
            .expect("read");
        let manifest: serde_json::Value = serde_json::from_str(&manifest).expect("json");
        assert_eq!(manifest["server"]["entry_point"], "server/koushi-mcp");

        assert_eq!(
            archive.by_name("icon.png").expect("icon").size(),
            ICON.len() as u64
        );
        let server = archive.by_name("server/koushi-mcp").expect("launcher");
        assert_eq!(server.unix_mode().map(|mode| mode & 0o777), Some(0o755));
        assert_eq!(server.size(), b"synthetic launcher".len() as u64);
    }

    #[test]
    fn the_windows_bundle_names_the_exe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes =
            build_bundle(&launcher(dir.path()), "0.28.0", Platform::Windows).expect("bundle");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        assert!(archive.by_name("server/koushi-mcp.exe").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn writing_replaces_a_planted_symlink_instead_of_following_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep me").expect("write");
        std::os::unix::fs::symlink(&victim, dir.path().join(FILE_NAME)).expect("symlink");

        let written = write_bundle(dir.path(), b"bundle").expect("write");
        assert_eq!(std::fs::read(&victim).expect("read"), b"keep me");
        assert!(
            !std::fs::symlink_metadata(&written)
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&written).expect("read"), b"bundle");
    }
}
