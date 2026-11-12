//! Element X detection over synthetic session directories this test builds
//! itself. No test reads a real Element X store.

use super::element_x::detect_in;

fn session_dir(root: &std::path::Path, id: &str, with_store: bool) -> std::path::PathBuf {
    let session = root.join(id);
    std::fs::create_dir_all(&session).expect("session dir");
    if with_store {
        std::fs::write(session.join("matrix-sdk-crypto.sqlite3"), b"store").expect("store file");
    }
    session
}

#[test]
fn only_directories_that_hold_a_store_count_as_sessions() {
    let root = tempfile::tempdir().expect("tempdir");
    session_dir(root.path(), "session-a", true);
    session_dir(root.path(), "session-b", true);
    session_dir(root.path(), "signed-out", false);

    let (sessions, unreadable) = detect_in(Some(root.path()));

    assert_eq!(sessions, 2);
    assert!(!unreadable);
}

#[test]
fn a_missing_sessions_root_is_not_an_unreadable_one() {
    let (sessions, unreadable) = detect_in(None);

    assert_eq!(sessions, 0);
    assert!(!unreadable);
}

/// macOS may refuse the group container entirely; that must read as "could not
/// look", not as "no Element X data", or the UI offers the wrong fallback.
#[cfg(unix)]
#[test]
fn a_protected_group_container_is_reported_as_unreadable() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().expect("tempdir");
    let sessions = session_dir(root.path(), "sessions", false);
    session_dir(&sessions, "session-a", true);
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    let outcome = detect_in(Some(&sessions));

    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700))
        .expect("chmod back");
    assert_eq!(outcome, (0, true));
}
