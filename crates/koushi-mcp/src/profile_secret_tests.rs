use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

fn write(dir: &Path, name: &str, mode: u32, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    path
}

#[test]
fn a_private_regular_file_is_read_and_trimmed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write(dir.path(), "s", 0o600, "  abc-DEF_123\n");
    assert_eq!(read(&path).as_deref(), Ok("abc-DEF_123"));
}

#[test]
fn a_group_or_other_readable_file_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    for mode in [0o640, 0o604, 0o660, 0o644, 0o602] {
        let path = write(dir.path(), "s", mode, "secret");
        assert_eq!(read(&path), Err(SecretFileError::TooPermissive), "{mode:o}");
    }
}

#[test]
fn a_symlink_is_never_followed_even_to_a_private_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = write(dir.path(), "real", 0o600, "secret");
    let link = dir.path().join("link");
    symlink(&target, &link).expect("symlink");
    assert_eq!(read(&link), Err(SecretFileError::Symlink));
}

#[test]
fn a_directory_missing_or_empty_file_is_unusable() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(read(dir.path()).is_err());
    assert_eq!(
        read(&dir.path().join("absent")),
        Err(SecretFileError::Unusable)
    );
    let empty = write(dir.path(), "empty", 0o600, " \n");
    assert_eq!(read(&empty), Err(SecretFileError::Unusable));
    let huge = write(dir.path(), "huge", 0o600, &"x".repeat(5000));
    assert_eq!(read(&huge), Err(SecretFileError::Unusable));
}

#[test]
fn the_explanations_never_carry_the_path_or_content() {
    for error in [
        SecretFileError::Symlink,
        SecretFileError::NotRegular,
        SecretFileError::WrongOwner,
        SecretFileError::TooPermissive,
        SecretFileError::Unusable,
    ] {
        assert!(!error.explanation().contains('/'));
    }
}
