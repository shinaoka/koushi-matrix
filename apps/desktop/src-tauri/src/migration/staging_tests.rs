//! Staging-file cleanup. The files here are empty and only named like a staging
//! file; no key material is involved.

use super::sweep_staging_files;

#[test]
fn only_staging_files_are_swept() {
    let root = tempfile::tempdir().expect("tempdir");
    let staging = root.path().join("koushi-migrated-keys-abc.json");
    let unrelated = root.path().join("someone-elses-secret.json");
    std::fs::write(&staging, b"{}").expect("staging file");
    std::fs::write(&unrelated, b"{}").expect("unrelated file");

    sweep_staging_files(root.path(), |_| true);

    assert!(!staging.exists(), "the staging file must be gone");
    assert!(unrelated.exists(), "an unrelated temp file must survive");
}
