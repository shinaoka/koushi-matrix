use matrix_desktop_key::{
    CredentialBackendErrorKind, CredentialStore, InMemoryCredentialBackend, LocalSecretError,
    LocalUnlockSecret, SessionKeyId, is_locked_or_inaccessible_error, is_missing_credential_error,
};

fn key_id() -> SessionKeyId {
    SessionKeyId {
        homeserver: "https://matrix.example".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE123".to_owned(),
    }
}

#[test]
fn credential_backend_fake_round_trips_one_local_unlock_secret() {
    let backend = InMemoryCredentialBackend::default();
    let store = CredentialStore::with_backend("matrix-desktop-test", backend);
    let secret = LocalUnlockSecret::generate();

    store.save(&key_id(), &secret).expect("save");
    let loaded = store.load(&key_id()).expect("load");

    assert_eq!(
        secret.derive_sdk_store_key().as_bytes(),
        loaded.derive_sdk_store_key().as_bytes()
    );
}

#[test]
fn credential_backend_fake_reports_missing_credential_without_raw_error_text() {
    let backend = InMemoryCredentialBackend::default();
    let store = CredentialStore::with_backend("matrix-desktop-test", backend);

    let error = store.load(&key_id()).expect_err("missing credential");

    assert!(is_missing_credential_error(&error));
    assert_eq!(
        format!("{error}"),
        "credential backend error: missing credential"
    );
    assert!(!format!("{error:?}").contains("@user-a:example.invalid"));
}

#[test]
fn credential_backend_fake_locked_state_maps_to_locked_or_inaccessible() {
    let backend = InMemoryCredentialBackend::default();
    backend.set_error(CredentialBackendErrorKind::LockedOrInaccessible);
    let store = CredentialStore::with_backend("matrix-desktop-test", backend);

    let error = store.load(&key_id()).expect_err("locked credential store");

    assert!(is_locked_or_inaccessible_error(&error));
    assert!(matches!(
        error,
        LocalSecretError::CredentialBackend(CredentialBackendErrorKind::LockedOrInaccessible)
    ));
    assert_eq!(
        format!("{error}"),
        "credential backend error: locked or inaccessible"
    );
}

#[test]
fn credential_backend_macos_temporary_keychain_round_trip_is_env_gated() {
    if std::env::var_os("MATRIX_DESKTOP_MACOS_KEYCHAIN_QA").is_none() {
        eprintln!("skipping macOS keychain QA; MATRIX_DESKTOP_MACOS_KEYCHAIN_QA is not set");
        return;
    }

    #[cfg(not(target_os = "macos"))]
    panic!("MATRIX_DESKTOP_MACOS_KEYCHAIN_QA is only supported on macOS");

    #[cfg(target_os = "macos")]
    {
        run_macos_temporary_keychain_round_trip().expect("macOS temporary keychain QA failed");
    }
}

#[cfg(target_os = "macos")]
fn run_macos_temporary_keychain_round_trip() -> Result<(), String> {
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    const PASSWORD: &str = "matrix-desktop-temporary-keychain-qa-password";

    fn run_security(args: &[String]) -> Result<String, String> {
        let subcommand = args.first().map(String::as_str).unwrap_or("unknown");
        let output = Command::new("security")
            .args(args)
            .output()
            .map_err(|_| format!("security {subcommand} could not be executed"))?;
        if !output.status.success() {
            return Err(format!("security {subcommand} failed"));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn current_user_keychains() -> Result<Vec<String>, String> {
        let output = run_security(&["list-keychains".into(), "-d".into(), "user".into()])?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.trim_matches('"').to_owned())
            .collect())
    }

    fn current_default_keychain() -> Result<Option<String>, String> {
        let output = run_security(&["default-keychain".into(), "-d".into(), "user".into()])?;
        Ok(output
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.trim_matches('"').to_owned()))
    }

    struct KeychainGuard {
        path: String,
        previous_keychains: Vec<String>,
        previous_default_keychain: Option<String>,
    }

    impl Drop for KeychainGuard {
        fn drop(&mut self) {
            if let Some(previous_default_keychain) = &self.previous_default_keychain {
                let _ = run_security(&[
                    "default-keychain".to_owned(),
                    "-d".to_owned(),
                    "user".to_owned(),
                    "-s".to_owned(),
                    previous_default_keychain.clone(),
                ]);
            }
            let mut restore_args = vec![
                "list-keychains".to_owned(),
                "-d".to_owned(),
                "user".to_owned(),
                "-s".to_owned(),
            ];
            restore_args.extend(self.previous_keychains.clone());
            let _ = run_security(&restore_args);
            let _ = run_security(&[
                "unlock-keychain".to_owned(),
                "-p".to_owned(),
                PASSWORD.to_owned(),
                self.path.clone(),
            ]);
            let _ = run_security(&["delete-keychain".to_owned(), self.path.clone()]);
        }
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before unix epoch".to_owned())?
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "matrix-desktop-keychain-qa-{}-{unique}.keychain-db",
        std::process::id()
    ));
    let path = path.to_string_lossy().into_owned();
    let previous_keychains = current_user_keychains()?;
    let previous_default_keychain = current_default_keychain()?;
    let guard = KeychainGuard {
        path: path.clone(),
        previous_keychains,
        previous_default_keychain,
    };

    run_security(&[
        "create-keychain".to_owned(),
        "-p".to_owned(),
        PASSWORD.to_owned(),
        path.clone(),
    ])?;
    run_security(&[
        "set-keychain-settings".to_owned(),
        "-lut".to_owned(),
        "21600".to_owned(),
        path.clone(),
    ])?;
    run_security(&[
        "unlock-keychain".to_owned(),
        "-p".to_owned(),
        PASSWORD.to_owned(),
        path.clone(),
    ])?;
    run_security(&[
        "default-keychain".to_owned(),
        "-d".to_owned(),
        "user".to_owned(),
        "-s".to_owned(),
        path.clone(),
    ])?;

    let mut list_args = vec![
        "list-keychains".to_owned(),
        "-d".to_owned(),
        "user".to_owned(),
        "-s".to_owned(),
        path.clone(),
    ];
    list_args.extend(guard.previous_keychains.clone());
    run_security(&list_args)?;
    // Hosted macOS runners can reject partition-list updates for a temporary
    // generic-password-only keychain. Keep this as best-effort; the proof below
    // still fails if the real backend cannot save/load/delete or fail closed.
    let _ = run_security(&[
        "set-key-partition-list".to_owned(),
        "-S".to_owned(),
        "apple-tool:,apple:".to_owned(),
        "-s".to_owned(),
        "-k".to_owned(),
        PASSWORD.to_owned(),
        path.clone(),
    ]);

    let store = CredentialStore::new(&format!("matrix-desktop-keychain-qa-{unique}"));
    let secret = LocalUnlockSecret::generate();
    store
        .save(&key_id(), &secret)
        .map_err(|_| "temporary keychain save failed".to_owned())?;
    let loaded = store
        .load(&key_id())
        .map_err(|_| "temporary keychain load failed".to_owned())?;
    assert_eq!(
        secret.derive_search_index_key().as_str(),
        loaded.derive_search_index_key().as_str()
    );

    store
        .delete(&key_id())
        .map_err(|_| "temporary keychain delete failed".to_owned())?;
    let error = store
        .load(&key_id())
        .expect_err("deleted temporary keychain credential must be missing");
    assert!(
        is_missing_credential_error(&error),
        "deleted temporary keychain credential must map to a coarse missing-credential error"
    );
    Ok(())
}
