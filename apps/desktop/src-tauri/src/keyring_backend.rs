//! OS-native credential store adapter backed by the `keyring` crate.
//!
//! This is the ONLY file in the workspace that imports `keyring`. It maps
//! `keyring::Error` into `CredentialBackendErrorKind` (platform-free) so that
//! `matrix-desktop-key` and `matrix-desktop-core` never depend on keyring.
//!
//! Architecture: Phase 5 DI inversion — the platform binary (`src-tauri`)
//! owns the OS adapter and injects it into `CoreRuntime` via
//! `start_with_data_dir_and_os_backend`.

use matrix_desktop_key::CredentialBackend;
use matrix_desktop_key::CredentialBackendErrorKind;

#[derive(Clone, Debug)]
pub struct KeyringCredentialBackend;

impl CredentialBackend for KeyringCredentialBackend {
    fn set_password(
        &self,
        service_name: &str,
        account_name: &str,
        value: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        keyring::Entry::new(service_name, account_name)
            .map_err(kind_from_keyring_error)?
            .set_password(value)
            .map_err(kind_from_keyring_error)
    }

    fn get_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<String, CredentialBackendErrorKind> {
        keyring::Entry::new(service_name, account_name)
            .map_err(kind_from_keyring_error)?
            .get_password()
            .map_err(kind_from_keyring_error)
    }

    fn delete_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        keyring::Entry::new(service_name, account_name)
            .map_err(kind_from_keyring_error)?
            .delete_credential()
            .map_err(kind_from_keyring_error)
    }
}

/// Map a `keyring::Error` to a platform-free `CredentialBackendErrorKind`.
fn kind_from_keyring_error(error: keyring::Error) -> CredentialBackendErrorKind {
    match error {
        keyring::Error::NoEntry => CredentialBackendErrorKind::MissingCredential,
        keyring::Error::NoStorageAccess(_) => CredentialBackendErrorKind::LockedOrInaccessible,
        keyring::Error::BadEncoding(_) | keyring::Error::Ambiguous(_) => {
            CredentialBackendErrorKind::Corrupt
        }
        keyring::Error::PlatformFailure(_)
        | keyring::Error::TooLong(_, _)
        | keyring::Error::Invalid(_, _) => CredentialBackendErrorKind::Unavailable,
        _ => CredentialBackendErrorKind::Unavailable,
    }
}

/// Helper for the keyring_backend integration test.
///
/// Maps a `keyring::Result<()>` so that `NoEntry` (missing credential) is
/// treated as success — the same semantics as `CredentialStore::delete_raw`.
#[doc(hidden)]
pub fn map_delete_result(
    result: keyring::Result<()>,
) -> Result<(), matrix_desktop_key::LocalSecretError> {
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(matrix_desktop_key::LocalSecretError::CredentialBackend(
            kind_from_keyring_error(error),
        )),
    }
}
