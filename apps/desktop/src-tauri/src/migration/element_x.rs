//! Element X's local crypto store (macOS: the iOS app runs in a group
//! container).
//!
//! Detection only, never an import. Every value the Rust SDK writes through its
//! crypto store is encrypted under a passphrase Element X generates at sign-in
//! and keeps in its own keychain, so the pickles inside are unreadable from
//! outside the app. Reporting the store as present-and-unimportable is what stops
//! the settings surface from offering a migration that cannot work.

use std::path::{Path, PathBuf};

const SESSIONS_SUBPATH: &str = "Library/Group Containers/group.io.element/Library/Application Support/io.element.elementx/Sessions";
const STORE_FILE: &str = "matrix-sdk-crypto.sqlite3";

/// How many Element X sessions hold a crypto store, and whether the group
/// container could be listed at all.
pub(super) fn detect_sessions() -> (u64, bool) {
    detect_in(sessions_root().as_deref())
}

pub(super) fn detect_in(root: Option<&Path>) -> (u64, bool) {
    let Some(root) = root else {
        return (0, false);
    };
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            // macOS protects other applications' group containers, and a denied
            // read does not always report itself as `PermissionDenied`. Only a
            // missing root means "no Element X data here"; anything else must
            // not read as an empty store.
            return (0, error.kind() != std::io::ErrorKind::NotFound);
        }
    };
    let sessions = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter(|entry| entry.path().join(STORE_FILE).is_file())
        .count() as u64;
    (sessions, false)
}

fn sessions_root() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    dirs::home_dir().map(|home| home.join(SESSIONS_SUBPATH))
}
