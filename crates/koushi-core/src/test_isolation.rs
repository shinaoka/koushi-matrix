//! Tests and QA must never read or write the real desktop profile.
//!
//! Every test/QA store constructor calls [`assert_not_user_profile`]. Test
//! runtimes start through [`crate::runtime::CoreRuntime::start_isolated`],
//! which owns temporary data and credential directories.

use std::path::{Path, PathBuf};

/// Directory name of the desktop profile (`koushi-desktop`), as resolved by
/// the desktop shell under the platform's local data directory.
const PROFILE_DIR_NAME: &str = "koushi-desktop";

/// Panic if `data_dir` is, or is inside, a real Koushi desktop profile.
pub(crate) fn assert_not_user_profile(data_dir: &Path) {
    if let Some(profile) = user_profile_containing(data_dir, &user_profiles()) {
        panic!(
            "test/QA stores must not use the real Koushi profile ({}); use CoreRuntime::start_isolated or a temporary directory",
            profile.display()
        );
    }
}

fn user_profiles() -> Vec<PathBuf> {
    let mut profiles = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        profiles.push(local.join(PROFILE_DIR_NAME));
    }
    // The former test default (`$HOME/.local/share/koushi-desktop`), which is
    // also the Linux desktop profile.
    if let Some(home) = dirs::home_dir() {
        profiles.push(home.join(".local").join("share").join(PROFILE_DIR_NAME));
    }
    if let Some(configured) = std::env::var_os("KOUSHI_DATA_DIR").filter(|dir| !dir.is_empty()) {
        profiles.push(PathBuf::from(configured));
    }
    profiles
}

fn user_profile_containing<'a>(data_dir: &Path, profiles: &'a [PathBuf]) -> Option<&'a PathBuf> {
    let data_dir = std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    profiles
        .iter()
        .find(|profile| data_dir.starts_with(profile))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_paths_and_their_children_are_refused() {
        let profiles = [PathBuf::from("/home/synthetic/.local/share/koushi-desktop")];
        for refused in [
            "/home/synthetic/.local/share/koushi-desktop",
            "/home/synthetic/.local/share/koushi-desktop/accounts/v2",
        ] {
            assert!(user_profile_containing(Path::new(refused), &profiles).is_some());
        }
        for allowed in [
            "/tmp/.tmpAbC123",
            "/home/synthetic/.local/share/koushi-desktop-real-qa",
            "/home/synthetic/repo/.local-secrets/headless-local-qa",
        ] {
            assert!(user_profile_containing(Path::new(allowed), &profiles).is_none());
        }
    }

    #[test]
    #[should_panic(expected = "must not use the real Koushi profile")]
    fn store_constructors_refuse_the_real_profile() {
        let profile = dirs::data_local_dir()
            .expect("local data directory")
            .join(PROFILE_DIR_NAME);
        // Construction only; nothing is read or written before the guard.
        let _ = crate::store::StoreActor::new(profile);
    }

    #[test]
    fn isolated_runtime_stores_are_not_a_profile() {
        let stores = crate::runtime::IsolatedStores::new();
        assert_not_user_profile(stores.data_dir());
        assert_not_user_profile(stores.credential_dir());
    }
}
